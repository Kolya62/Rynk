// SPDX-License-Identifier: GPL-3.0-or-later
//! Prediction engine: noisy-channel decoding of taps into words, autocorrection decision,
//! completions and next-word predictions.
//!
//! P(word | taps, context) ∝ P(taps | word) · P(word | context)
//! * channel P(taps | word): per-tap Gaussian touch likelihood from [`touch::TouchModel`] plus
//!   edit operations (extra tap, missed letter, swapped letters);
//! * source P(word | context): trigram/bigram corpus model with stupid backoff, merged with the
//!   user's own words and word pairs.

pub mod cjk;
pub mod gesture;
pub mod lm;
pub mod spelling;
pub mod touch;
pub mod trie;
pub mod userdict;

use lm::{Context, LanguageModel};
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;
use touch::{Tap, TouchModel};
use trie::{Trie, NONE};
use userdict::UserDict;

/// Largest channel cost (−log10) a candidate may have
const MAX_COST: f32 = 6.0;
/// A match whose spatial cost exceeds this is not considered at all
const MAX_TAP_COST: f32 = 3.2;
/// Spatial cost above which a match counts as an edit (a different key than the one touched)
const FAR_TAP: f32 = 1.6;
const COST_INSERT: f32 = 2.2;
const COST_INSERT_DOUBLE: f32 = 0.9;
const COST_DELETE: f32 = 2.2;
const COST_DELETE_DOUBLE: f32 = 0.9;
const COST_SWAP: f32 = 1.5;
/// States kept per tap position in the beam search
const BEAM: usize = 160;
/// Trie nodes whose best words are offered as completions
const MAX_COMPLETION_NODES: usize = 24;
/// Search effort bound per decode
const BUDGET: u32 = 120_000;
/// Margin (log10) a correction must win by over a word that exists in the dictionary
const KNOWN_WORD_MARGIN: f32 = 2.6;
/// Unknown typed word: largest channel cost of an automatic correction
const AUTO_MAX_COST: f32 = 4.6;
/// Unknown but plausible typed word: largest cost of a correction
const PLAUSIBLE_MAX_COST: f32 = 2.4;
/// ... unless the length changes: then only to a common word (log10 probability in context)
const PLAUSIBLE_COMMON_LOGP: f32 = -4.0;
/// One-letter words: largest touch cost of another letter, and the margin it must win by
const SINGLE_MAX_COST: f32 = 1.0;
const SINGLE_MARGIN: f32 = 0.7;
/// Penalty of offering a completion instead of the exact typed length (log10)
const COMPLETION_PENALTY: f32 = 0.6;

/// Asset bytes as stored: zstd-compressed (the APK keeps them as they are) or plain
pub fn unpack(data: &[u8]) -> Option<std::borrow::Cow<'_, [u8]>> {
    if data.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        use std::io::Read;
        let mut src = data;
        let mut dec = ruzstd::decoding::StreamingDecoder::new(&mut src).ok()?;
        let mut out = Vec::with_capacity(data.len() * 2);
        dec.read_to_end(&mut out).ok()?;
        Some(std::borrow::Cow::Owned(out))
    } else {
        Some(std::borrow::Cow::Borrowed(data))
    }
}

/// Corpus model plus its trie, built once per language (in the background)
pub struct Model {
    pub lm: LanguageModel,
    pub trie: Trie,
    /// Letter trigrams (with word boundaries) seen in the vocabulary: a word made only of them
    /// looks like a real word the dictionary lacks (an inflection, a name), not a typo
    trigrams: std::collections::HashSet<u64>,
}

fn trigram_keys(lower: &str) -> impl Iterator<Item = u64> + '_ {
    let cs: Vec<u64> = std::iter::once(1u64).chain(lower.chars().map(|c| c as u64 + 2)).chain(std::iter::once(0u64)).collect();
    (0..cs.len().saturating_sub(2)).map(move |i| cs[i] << 42 | cs[i + 1] << 21 | cs[i + 2]).collect::<Vec<_>>().into_iter()
}

impl Model {
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let lm = LanguageModel::decode(data)?;
        let mut lowers: Vec<(String, u32, f32)> = lm
            .words
            .iter()
            .enumerate()
            .map(|(i, w)| (w.text.to_lowercase(), i as u32, w.logp))
            .collect();
        lowers.retain(|w| !w.0.is_empty());
        let trie = Trie::build(lowers.iter().map(|(w, i, p)| (w.as_str(), *i, *p)));
        let mut trigrams = std::collections::HashSet::new();
        for (w, _, p) in &lowers {
            // rare words carry many typos and foreign names
            if *p > -7.0 {
                trigrams.extend(trigram_keys(w));
            }
        }
        Some(Self { lm, trie, trigrams })
    }

    /// Is every letter trigram of the word one the language uses?
    pub fn plausible(&self, lower: &str) -> bool {
        trigram_keys(lower).all(|k| self.trigrams.contains(&k))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Src {
    Lm(u32),
    User(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub text: String,
    /// log10 posterior (channel + language model)
    pub score: f32,
    pub cost: f32,
    pub edits: u8,
    /// A longer word that starts like the input (only offered as a suggestion)
    pub completion: bool,
}

/// What the suggestion strip shows for the word being typed
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Suggestions {
    /// Shown left → right; the middle one (or the only one) is the primary
    pub items: Vec<String>,
    /// The word that replaces the typed one on space, when autocorrection fires
    pub autocorrect: Option<String>,
}

#[derive(Default)]
pub struct Predictor {
    pub model: Option<Arc<Model>>,
    /// Spelling rules of the current language
    pub rules: &'static [spelling::Rule],
    pub user: UserDict,
    user_words: Vec<String>,
    user_trie: Trie,
    user_trie_stale: bool,
}

impl Predictor {
    pub fn set_model(&mut self, model: Option<Arc<Model>>) {
        self.model = model;
    }

    pub fn set_user(&mut self, user: UserDict) {
        self.user = user;
        self.user_trie_stale = true;
    }

    fn refresh_user_trie(&mut self) {
        if !self.user_trie_stale {
            return;
        }
        self.user_words = self.user.words.iter().filter(|(_, v)| v.1 >= userdict::KNOWN_AFTER).map(|(k, _)| k.clone()).collect();
        let user = &self.user;
        self.user_trie = Trie::build(
            self.user_words.iter().enumerate().map(|(i, w)| (w.as_str(), i as u32, user.logp(w).unwrap_or(-6.0))),
        );
        self.user_trie_stale = false;
    }

    /// Does the dictionary (corpus or personal) know this word?
    pub fn is_known(&self, lower: &str) -> bool {
        self.user.is_known(lower) || self.model.as_ref().is_some_and(|m| m.lm.id(lower).is_some())
    }

    /// Next-letter distribution after a lowercase prefix (drives the dynamic grid)
    pub fn next_letters(&self, prefix: &str) -> Vec<(char, f32)> {
        self.model.as_ref().map(|m| m.trie.next_letters(prefix)).unwrap_or_default()
    }

    fn word_score(&self, src: Src, lower: &str, ctx: &Context, keys: (Option<u32>, Option<u32>)) -> f32 {
        let corpus = match (src, &self.model) {
            (Src::Lm(id), Some(m)) => m.lm.score(id, keys),
            _ => self
                .model
                .as_ref()
                .and_then(|m| m.lm.id(lower).map(|id| m.lm.score(id, keys)))
                .unwrap_or(f32::NEG_INFINITY),
        };
        let mut s = corpus;
        if let Some(u) = self.user.logp(lower) {
            s = s.max(u + if ctx.prev.is_empty() { 0.0 } else { 2.0 * lm::BACKOFF });
            // personal use lifts a word above corpus words of similar probability
            s += 0.15 * (self.user.uses(lower) as f32).log10().min(2.0);
        }
        if let Some(prev) = ctx.prev.first() {
            if let Some(p) = self.user.pair_logp(prev, lower) {
                s = s.max(p);
            }
        }
        s
    }

    fn display(&self, src: Src) -> String {
        match (src, &self.model) {
            (Src::Lm(id), Some(m)) => m.lm.words[id as usize].text.clone(),
            (Src::User(i), _) => {
                let lower = &self.user_words[i as usize];
                self.user.words.get(lower).map_or_else(|| lower.clone(), |w| w.0.clone())
            }
            _ => String::new(),
        }
    }

    /// Candidates for the taps of the word being typed, best first
    pub fn decode(&mut self, taps: &[Tap], touch: &TouchModel, ctx: &Context) -> Vec<Candidate> {
        if taps.is_empty() {
            return Vec::new();
        }
        self.refresh_user_trie();
        let max_edits = match taps.len() {
            0..=2 => 0,
            3..=5 => 1,
            _ => 2,
        };
        let mut found: HashMap<Src, (f32, u8, bool)> = HashMap::new();
        if let Some(m) = self.model.clone() {
            let mut s = Search { trie: &m.trie, taps, touch, rules: self.rules, max_edits, budget: BUDGET, out: Vec::new() };
            s.run();
            let logp = |id: u32| m.lm.words[id as usize].logp;
            for (id, cost, edits, node) in s.out {
                if id != NONE {
                    merge(&mut found, Src::Lm(id), cost, edits, false);
                }
                if let Some(n) = node {
                    for cid in top_words(&m.trie, n, 3, logp) {
                        merge(&mut found, Src::Lm(cid), cost, edits, true);
                    }
                }
            }
        }
        if !self.user_trie.is_empty() {
            let user = &self.user;
            let words = &self.user_words;
            let mut s = Search { trie: &self.user_trie, taps, touch, rules: self.rules, max_edits, budget: BUDGET / 4, out: Vec::new() };
            s.run();
            for (id, cost, edits, node) in s.out {
                if id != NONE {
                    merge(&mut found, Src::User(id), cost, edits, false);
                }
                if let Some(n) = node {
                    let logp = |i: u32| user.logp(&words[i as usize]).unwrap_or(-6.0);
                    for cid in top_words(&self.user_trie, n, 2, logp) {
                        merge(&mut found, Src::User(cid), cost, edits, true);
                    }
                }
            }
        }
        let keys = self.model.as_ref().map(|m| m.lm.keys(ctx)).unwrap_or((None, None));
        let mut cands: Vec<Candidate> = Vec::with_capacity(found.len());
        let mut seen: HashMap<String, usize> = HashMap::new();
        for (src, (cost, edits, completion)) in found {
            let text = self.display(src);
            let lower = text.to_lowercase();
            let lm = self.word_score(src, &lower, ctx, keys);
            if !lm.is_finite() {
                continue;
            }
            let score = lm - cost - if completion { COMPLETION_PENALTY } else { 0.0 };
            let c = Candidate { text, score, cost, edits, completion };
            match seen.get(&lower) {
                Some(&i) if cands[i].score >= score => {}
                Some(&i) => cands[i] = c,
                None => {
                    seen.insert(lower, cands.len());
                    cands.push(c);
                }
            }
        }
        cands.sort_by(|a, b| b.score.total_cmp(&a.score));
        cands
    }

    /// Suggestions and the autocorrection decision for the word being typed.
    /// Words for a swiped path (keyboard pixels), best first, in dictionary spelling
    pub fn gesture(&mut self, raw: &[(f32, f32)], touch: &TouchModel, ctx: &Context, limit: usize) -> Vec<String> {
        let keys_g = gesture::GestureKeys::new(touch);
        let Some(path) = gesture::Path::new(raw, keys_g.key_w()) else { return Vec::new() };
        let starts = keys_g.near(path.pts[0], 1.6);
        let ends = keys_g.near(*path.pts.last().unwrap(), 1.6);
        if starts.is_empty() || ends.is_empty() {
            return Vec::new();
        }
        self.refresh_user_trie();
        let keys = self.model.as_ref().map(|m| m.lm.keys(ctx)).unwrap_or((None, None));
        let fits = |lower: &str| {
            let mut cs = lower.chars();
            let first = cs.next();
            let last = lower.chars().next_back();
            lower.chars().count() >= 2 && first.is_some_and(|c| starts.contains(&c)) && last.is_some_and(|c| ends.contains(&c))
        };
        let mut scored: Vec<(f32, Src, String)> = Vec::new();
        if let Some(m) = self.model.clone() {
            for (id, w) in m.lm.words.iter().enumerate() {
                let first = w.text.chars().next().map(|c| c.to_lowercase().next().unwrap_or(c));
                if !first.is_some_and(|c| starts.contains(&c)) {
                    continue;
                }
                let lower = w.text.to_lowercase();
                if !fits(&lower) {
                    continue;
                }
                let Some(ideal) = keys_g.ideal(&lower) else { continue };
                let Some(cost) = keys_g.cost(&path, &ideal) else { continue };
                let score = self.word_score(Src::Lm(id as u32), &lower, ctx, keys) - cost;
                scored.push((score, Src::Lm(id as u32), lower));
            }
        }
        for (i, lower) in self.user_words.iter().enumerate() {
            if !fits(lower) || scored.iter().any(|s| &s.2 == lower) {
                continue;
            }
            let Some(ideal) = keys_g.ideal(lower) else { continue };
            let Some(cost) = keys_g.cost(&path, &ideal) else { continue };
            let score = self.word_score(Src::User(i as u32), lower, ctx, keys) - cost;
            scored.push((score, Src::User(i as u32), lower.clone()));
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut out: Vec<String> = Vec::new();
        for (_, src, _) in scored {
            let text = self.display(src);
            if !text.is_empty() && !out.iter().any(|o| o.to_lowercase() == text.to_lowercase()) {
                out.push(text);
            }
            if out.len() >= limit {
                break;
            }
        }
        out
    }

    pub fn suggest(&mut self, typed: &str, taps: &[Tap], touch: &TouchModel, ctx: &Context, autocorrect: bool) -> Suggestions {
        if typed.is_empty() {
            return Suggestions::default();
        }
        let cands = self.decode(taps, touch, ctx);
        let lower = typed.to_lowercase();
        let typed_known = self.is_known(&lower);
        let keys = self.model.as_ref().map(|m| m.lm.keys(ctx)).unwrap_or((None, None));
        let typed_score = if typed_known {
            let src = self.model.as_ref().and_then(|m| m.lm.id(&lower)).map(Src::Lm).unwrap_or(Src::User(u32::MAX));
            self.word_score(src, &lower, ctx, keys)
        } else {
            f32::NEG_INFINITY
        };

        let plausible = !typed_known && self.model.as_ref().is_some_and(|m| m.plausible(&lower));
        let single = typed.chars().count() == 1 && typed.chars().all(char::is_alphabetic);
        let correction = if autocorrect && single {
            // A one-letter word: the touch may sit between two keys ("а" or "в"); the context
            // decides, but only between one-letter words and only for an ambiguous touch
            let own = cands.iter().find(|c| !c.completion && c.text.to_lowercase() == lower).map(|c| c.score);
            cands
                .iter()
                .filter(|c| !c.completion && c.text.chars().count() == 1 && c.text.to_lowercase() != lower)
                .find(|c| c.cost <= SINGLE_MAX_COST && own.is_none_or(|o| c.score > o + SINGLE_MARGIN))
                .map(|c| apply_case(typed, &c.text))
        } else if autocorrect && may_autocorrect(typed) {
            cands
                .iter()
                .find(|c| !c.completion)
                .filter(|c| c.text.to_lowercase() != lower)
                .filter(|c| {
                    if typed_known {
                        c.score > typed_score + KNOWN_WORD_MARGIN && c.edits <= 1
                    } else if plausible {
                        // looks like a real word the dictionary lacks: only a clear slip of
                        // the finger, keeping the length
                        let len_diff = c.text.chars().count().abs_diff(typed.chars().count());
                        let common = c.score + c.cost > PLAUSIBLE_COMMON_LOGP;
                        c.cost <= PLAUSIBLE_MAX_COST && (len_diff == 0 || len_diff == 1 && common)
                    } else {
                        c.cost <= AUTO_MAX_COST
                    }
                })
                .map(|c| apply_case(typed, &c.text))
        } else {
            None
        };

        // Strip: [alternative] [primary] [alternative], the primary is what space commits
        let mut alts: Vec<String> = Vec::new();
        for c in &cands {
            let t = apply_case(typed, &c.text);
            if Some(&t) != correction.as_ref() && t != typed && !alts.contains(&t) {
                alts.push(t);
            }
            if alts.len() >= 2 {
                break;
            }
        }
        let primary = correction.clone().unwrap_or_else(|| typed.to_string());
        let mut items = Vec::new();
        if correction.is_some() {
            // the typed word stays one tap away, quoted
            items.push(format!("«{typed}»"));
            items.push(primary);
            if let Some(a) = alts.into_iter().next() {
                items.push(a);
            }
        } else {
            let mut it = alts.into_iter();
            if let Some(a) = it.next() {
                items.push(a);
            }
            items.push(primary);
            if let Some(b) = it.next() {
                items.push(b);
            }
        }
        Suggestions { items, autocorrect: correction }
    }

    /// Next-word predictions after a committed word (or at a sentence start)
    pub fn predict(&self, ctx: &Context, capitalize: bool) -> Vec<String> {
        let mut out: Vec<(String, f32)> = Vec::new();
        if let Some(prev) = ctx.prev.first() {
            out.extend(self.user.continuations(prev).into_iter().take(3));
        }
        if let Some(m) = &self.model {
            let keys = m.lm.keys(ctx);
            if keys.1.is_some() {
                for (id, p) in m.lm.predict(keys, 6) {
                    let w = &m.lm.words[id as usize].text;
                    if !w.chars().any(char::is_alphanumeric) {
                        continue;
                    }
                    out.push((w.clone(), p));
                }
            }
        }
        out.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut seen = Vec::<String>::new();
        let mut res = Vec::new();
        for (w, _) in out {
            let l = w.to_lowercase();
            if seen.contains(&l) {
                continue;
            }
            seen.push(l);
            res.push(if capitalize { capitalize_first(&w) } else { w });
            if res.len() == 3 {
                break;
            }
        }
        // most likely in the middle, as on the strip
        if res.len() == 3 {
            res.swap(0, 1);
        }
        res
    }

    /// Learning after a word was committed as the user wants it
    pub fn learn(&mut self, word: &str, prev: Option<&str>) {
        let lower = word.to_lowercase();
        if !word.chars().any(char::is_alphabetic) {
            return;
        }
        let was_known = self.user.is_known(&lower);
        self.user.add_word(word, 1);
        if let Some(p) = prev {
            self.user.add_pair(p, word);
        }
        if was_known != self.user.is_known(&lower) {
            self.user_trie_stale = true;
        }
    }

    /// The user undid an autocorrection: trust the typed word from now on
    pub fn learn_rejected(&mut self, typed: &str) {
        let n = self.user.uses(&typed.to_lowercase());
        self.user.add_word(typed, userdict::KNOWN_AFTER.saturating_sub(n).max(1));
        self.user_trie_stale = true;
    }
}

fn merge(found: &mut HashMap<Src, (f32, u8, bool)>, src: Src, cost: f32, edits: u8, completion: bool) {
    match found.get_mut(&src) {
        None => {
            found.insert(src, (cost, edits, completion));
        }
        // an exact-length reading always beats a completion of the same word
        Some(e) if (e.2 && !completion) || (e.2 == completion && cost < e.0) => *e = (cost, edits, completion),
        Some(_) => {}
    }
}

/// Should a typed token be considered for automatic correction at all?
fn may_autocorrect(typed: &str) -> bool {
    let n = typed.chars().count();
    n >= 2
        && typed.chars().all(|c| c.is_alphabetic() || c == '\'' || c == '-' || c == '’')
        && !(n > 1 && typed.chars().all(|c| !c.is_alphabetic() || c.is_uppercase()))
}

pub fn capitalize_first(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Carries the user's capitalization onto a candidate's canonical spelling
pub fn apply_case(typed: &str, cand: &str) -> String {
    let letters: Vec<char> = typed.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() > 1 && letters.iter().all(|c| c.is_uppercase()) {
        return cand.to_uppercase();
    }
    if typed.chars().next().is_some_and(char::is_uppercase) {
        return capitalize_first(cand);
    }
    cand.to_string()
}

/// Depth-first noisy-channel search over a trie
struct Search<'a> {
    trie: &'a Trie,
    taps: &'a [Tap],
    touch: &'a TouchModel,
    rules: &'a [spelling::Rule],
    max_edits: u8,
    budget: u32,
    /// (word id, channel cost, edits, node for completions)
    out: Vec<(u32, f32, u8, Option<u32>)>,
}

impl Search<'_> {
    /// Beam search over the trie, one bucket of states per tap position: each bucket keeps
    /// only its `BEAM` cheapest states, so the work per keystroke stays bounded however many
    /// edits are allowed.
    fn run(&mut self) {
        let taps = self.taps;
        let n_taps = taps.len();
        // (node, edits) -> (cost, last letter)
        type Bucket = HashMap<(u32, u8), (f32, Option<char>)>;
        let mut buckets: Vec<Bucket> = (0..=n_taps).map(|_| HashMap::new()).collect();
        fn put(b: &mut Bucket, node: u32, edits: u8, cost: f32, last: Option<char>) -> bool {
            if cost > MAX_COST {
                return false;
            }
            match b.get_mut(&(node, edits)) {
                Some(v) if v.0 <= cost => false,
                Some(v) => {
                    *v = (cost, last);
                    true
                }
                None => {
                    b.insert((node, edits), (cost, last));
                    true
                }
            }
        }
        put(&mut buckets[0], self.trie.root(), 0, 0.0, None);
        for i in 0..=n_taps {
            // missed letters (and spelling rules that type nothing) stay at this tap
            if i > 0 {
                let mut work: Vec<(u32, u8)> = buckets[i].keys().copied().collect();
                while let Some((n, e)) = work.pop() {
                    if self.budget == 0 {
                        break;
                    }
                    self.budget -= 1;
                    let Some(&(cost, last)) = buckets[i].get(&(n, e)) else { continue };
                    if e >= self.max_edits {
                        continue;
                    }
                    for &(c, k) in self.trie.children(n) {
                        let double = last == Some(c);
                        let dc = if double { COST_DELETE_DOUBLE } else { COST_DELETE };
                        if put(&mut buckets[i], k, e + 1, cost + dc, Some(c)) {
                            work.push((k, e + 1));
                        }
                    }
                    for &(typed, meant, rc) in self.rules.iter().filter(|r| r.0.is_empty()) {
                        if let Some(m) = self.walk(n, meant) {
                            if put(&mut buckets[i], m, e + 1, cost + rc, meant.chars().last().or(last)) {
                                work.push((m, e + 1));
                            }
                        }
                        let _ = typed;
                    }
                }
            }
            // keep the cheapest states
            let mut states: Vec<((u32, u8), (f32, Option<char>))> = buckets[i].drain().collect();
            states.sort_by(|a, b| a.1 .0.total_cmp(&b.1 .0));
            states.truncate(BEAM);
            if i == n_taps {
                let mut completions = 0;
                for &((n, e), (cost, _)) in &states {
                    let node = self.trie.node(n);
                    if node.word != NONE {
                        self.out.push((node.word, cost, e, None));
                    }
                    if e <= 1 && n_taps >= 2 && completions < MAX_COMPLETION_NODES {
                        completions += 1;
                        self.out.push((NONE, cost, e, Some(n)));
                    }
                }
                break;
            }
            let tap = &taps[i];
            for &((n, e), (cost, last)) in &states {
                if self.budget == 0 {
                    break;
                }
                self.budget -= 1;
                let kids: &[(char, u32)] = self.trie.children(n);
                // match
                for &(c, k) in kids {
                    if let Some(cc) = self.touch.cost(tap, c) {
                        if cc <= MAX_TAP_COST {
                            let far = (cc > FAR_TAP) as u8;
                            if e + far <= self.max_edits {
                                put(&mut buckets[i + 1], k, e + far, cost + cc, Some(c));
                            }
                        }
                    }
                }
                if e >= self.max_edits {
                    continue;
                }
                // extra tap
                let double = last == Some(tap.ch) || (i > 0 && taps[i - 1].ch == tap.ch);
                put(&mut buckets[i + 1], n, e + 1, cost + if double { COST_INSERT_DOUBLE } else { COST_INSERT }, last);
                // swapped neighbours
                if i + 1 < n_taps {
                    for &(c1, k1) in kids {
                        let Some(a) = self.touch.cost(&taps[i + 1], c1) else { continue };
                        if a > 1.0 {
                            continue;
                        }
                        for &(c2, k2) in self.trie.children(k1) {
                            if c1 == c2 {
                                continue;
                            }
                            if let Some(b) = self.touch.cost(tap, c2) {
                                if b <= 1.0 {
                                    put(&mut buckets[i + 2], k2, e + 1, cost + COST_SWAP + a + b, Some(c2));
                                }
                            }
                        }
                    }
                }
                // spelling rules: the touched letters spell `typed` where the word has `meant`
                for &(typed, meant, rc) in self.rules {
                    let tl = typed.chars().count();
                    if tl == 0 || i + tl > n_taps || !typed.chars().zip(&taps[i..i + tl]).all(|(c, t)| t.ch == c) {
                        continue;
                    }
                    if let Some(m) = self.walk(n, meant) {
                        put(&mut buckets[i + tl], m, e + 1, cost + rc, meant.chars().last().or(last));
                    }
                }
            }
        }
    }

    fn walk(&self, mut n: u32, s: &str) -> Option<u32> {
        for c in s.chars() {
            n = self.trie.child(n, c)?;
        }
        Some(n)
    }
}

/// The `k` most probable words below a trie node
fn top_words(trie: &Trie, n: u32, k: usize, logp: impl Fn(u32) -> f32) -> Vec<u32> {
    #[derive(PartialEq)]
    struct E(f32, u32, bool);
    impl Eq for E {}
    impl PartialOrd for E {
        fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(o))
        }
    }
    impl Ord for E {
        fn cmp(&self, o: &Self) -> std::cmp::Ordering {
            self.0.total_cmp(&o.0)
        }
    }
    let mut heap = BinaryHeap::new();
    let mut out = Vec::new();
    for &(_, c) in trie.children(n) {
        heap.push(E(trie.node(c).best, c, false));
    }
    let mut pops = 0;
    while let Some(E(_, id, is_word)) = heap.pop() {
        pops += 1;
        if is_word {
            out.push(id);
            if out.len() == k {
                break;
            }
            continue;
        }
        if pops > 400 {
            break;
        }
        let node = trie.node(id);
        if node.word != NONE {
            heap.push(E(logp(node.word), node.word, true));
        }
        for &(_, c) in trie.children(id) {
            heap.push(E(trie.node(c).best, c, false));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::touch::tests::ru;
    use super::*;

    fn model(code: &str) -> Arc<Model> {
        let path = format!("{}/../app/src/main/assets/lm/{code}.rlm", env!("CARGO_MANIFEST_DIR"));
        Arc::new(Model::from_bytes(&std::fs::read(path).unwrap()).unwrap())
    }

    /// Taps at key centers for a string, optionally nudged
    fn taps(m: &TouchModel, s: &str, nudge: &[(usize, f32, f32)]) -> Vec<Tap> {
        s.chars()
            .enumerate()
            .map(|(i, ch)| {
                let k = m.key_for(ch).unwrap().0;
                let (dx, dy) = nudge.iter().find(|n| n.0 == i).map_or((0.0, 0.0), |n| (n.1, n.2));
                Tap { ch, pos: Some((k.cx + dx * k.w, k.cy + dy * k.h)) }
            })
            .collect()
    }

    fn predictor() -> Predictor {
        let mut p = Predictor::default();
        p.set_model(Some(model("ru")));
        p
    }

    fn ctx(prev: &[&str]) -> Context {
        Context { prev: prev.iter().map(|s| s.to_string()).collect(), sentence_start: false }
    }

    #[test]
    fn fixes_neighbour_key_slip() {
        let mut p = predictor();
        let tm = ru();
        // "привет" with "в" slipped onto "а" (its right neighbour), touched near the border
        let t = taps(&tm, "приает", &[(3, -0.45, 0.0)]);
        let s = p.suggest("приает", &t, &tm, &ctx(&[]), true);
        assert_eq!(s.autocorrect.as_deref(), Some("привет"), "{s:?}");
    }

    #[test]
    fn fixes_swapped_and_missing_letters() {
        let mut p = predictor();
        let tm = ru();
        let s = p.suggest("пирвет", &taps(&tm, "пирвет", &[]), &tm, &ctx(&[]), true);
        assert_eq!(s.autocorrect.as_deref(), Some("привет"), "{s:?}");
        let s = p.suggest("сегоня", &taps(&tm, "сегоня", &[]), &tm, &ctx(&[]), true);
        assert_eq!(s.autocorrect.as_deref(), Some("сегодня"), "{s:?}");
    }

    #[test]
    fn keeps_known_words_and_case() {
        let mut p = predictor();
        let tm = ru();
        let s = p.suggest("Стол", &taps(&tm, "стол", &[]), &tm, &ctx(&[]), true);
        assert_eq!(s.autocorrect, None, "{s:?}");
        let s = p.suggest("Пирвет", &taps(&tm, "пирвет", &[]), &tm, &ctx(&[]), true);
        assert_eq!(s.autocorrect.as_deref(), Some("Привет"));
    }

    #[test]
    fn context_chooses_between_readings() {
        let mut p = predictor();
        let tm = ru();
        // "дела" vs "деда": the context "как" makes "дела" win
        let t = taps(&tm, "делв", &[]);
        let s = p.suggest("делв", &t, &tm, &ctx(&["как"]), true);
        assert_eq!(s.autocorrect.as_deref(), Some("дела"), "{s:?}");
    }

    #[test]
    fn rejected_correction_is_learned() {
        let mut p = predictor();
        let tm = ru();
        let t = taps(&tm, "пирвет", &[]);
        assert!(p.suggest("пирвет", &t, &tm, &ctx(&[]), true).autocorrect.is_some());
        p.learn_rejected("пирвет");
        assert_eq!(p.suggest("пирвет", &t, &tm, &ctx(&[]), true).autocorrect, None);
    }

    #[test]
    fn completions_and_predictions() {
        let mut p = predictor();
        let tm = ru();
        let s = p.suggest("спаси", &taps(&tm, "спаси", &[]), &tm, &ctx(&[]), true);
        assert!(s.items.iter().any(|w| w == "спасибо"), "{s:?}");
        let next = p.predict(&ctx(&["спасибо"]), false);
        assert!(!next.is_empty());
    }

    #[test]
    fn decode_is_fast_enough() {
        let mut p = predictor();
        let tm = ru();
        let t = taps(&tm, "достопримечательнасти", &[]);
        let start = std::time::Instant::now();
        let _ = p.suggest("достопримечательнасти", &t, &tm, &ctx(&[]), true);
        assert!(start.elapsed().as_millis() < 400, "{:?}", start.elapsed());
    }

    /// A finger path for a word: through each key with some jitter and a wobbly line between
    fn swipe(m: &TouchModel, word: &str, seed: u32) -> Vec<(f32, f32)> {
        let mut r = seed.wrapping_mul(2654435761).wrapping_add(12345);
        let mut rnd = move || {
            r ^= r << 13;
            r ^= r >> 17;
            r ^= r << 5;
            (r % 1000) as f32 / 1000.0 - 0.5
        };
        let k = |c: char| m.key_for(c).map(|(k, _)| (k.cx, k.cy)).unwrap();
        let pts: Vec<(f32, f32)> = word.chars().map(|c| {
            let (x, y) = k(c);
            (x + rnd() * 5.0, y + rnd() * 6.0)
        }).collect();
        let mut out = vec![pts[0]];
        for w in pts.windows(2) {
            let bend = rnd() * 4.0;
            for i in 1..=6 {
                let t = i as f32 / 6.0;
                let off = (t * std::f32::consts::PI).sin() * bend;
                out.push((w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t + off));
            }
        }
        out
    }

    fn gesture_accuracy(code: &str, m: &TouchModel) -> (f32, f32, Vec<String>) {
        let mut p = Predictor::default();
        p.set_model(Some(model(code)));
        let lm = &p.model.as_ref().unwrap().lm;
        let mut words: Vec<(f32, String)> = lm
            .words
            .iter()
            .map(|w| (w.logp, w.text.clone()))
            .filter(|(_, t)| (3..=10).contains(&t.chars().count()) && t.chars().all(|c| m.key_for(c).is_some()))
            .collect();
        words.sort_by(|a, b| b.0.total_cmp(&a.0));
        let words: Vec<String> = words.into_iter().take(300).map(|w| w.1).collect();
        let ctx = Context { prev: Vec::new(), sentence_start: false };
        let (mut top1, mut top3) = (0, 0);
        let mut misses = Vec::new();
        for (i, w) in words.iter().enumerate() {
            let got = p.gesture(&swipe(m, w, i as u32 + 1), m, &ctx, 3);
            if got.first() == Some(w) {
                top1 += 1;
            } else if misses.len() < 15 {
                misses.push(format!("{w}->{}", got.join("/")));
            }
            if got.contains(w) {
                top3 += 1;
            }
        }
        (top1 as f32 / words.len() as f32, top3 as f32 / words.len() as f32, misses)
    }

    #[test]
    fn gesture_finds_swiped_words() {
        let ru_t = ru();
        let en_t = super::touch::tests::grid(&["qwertyuiop", "asdfghjkl", "zxcvbnm"]);
        for (code, t) in [("ru", &ru_t), ("en", &en_t)] {
            let start = std::time::Instant::now();
            let (a1, a3, misses) = gesture_accuracy(code, t);
            eprintln!("{code}: top1 {a1:.2} top3 {a3:.2} in {:?}; misses {misses:?}", start.elapsed());
            assert!(a1 >= 0.8 && a3 >= 0.93, "{code}: top1 {a1} top3 {a3}");
        }
    }
}
