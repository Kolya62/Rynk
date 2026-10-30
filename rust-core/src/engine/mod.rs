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
/// Search effort bound per decode
const BUDGET: u32 = 120_000;
/// Margin (log10) a correction must win by over a word that exists in the dictionary
const KNOWN_WORD_MARGIN: f32 = 2.6;
/// Unknown typed word: largest channel cost of an automatic correction
const AUTO_MAX_COST: f32 = 4.6;
/// Penalty of offering a completion instead of the exact typed length (log10)
const COMPLETION_PENALTY: f32 = 0.6;

/// Corpus model plus its trie, built once per language (in the background)
pub struct Model {
    pub lm: LanguageModel,
    pub trie: Trie,
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
        Some(Self { lm, trie })
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
            let mut s = Search { trie: &m.trie, taps, touch, max_edits, budget: BUDGET, out: Vec::new() };
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
            let mut s = Search { trie: &self.user_trie, taps, touch, max_edits, budget: BUDGET / 4, out: Vec::new() };
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

        let correction = if autocorrect && may_autocorrect(typed) {
            cands
                .iter()
                .find(|c| !c.completion)
                .filter(|c| c.text.to_lowercase() != lower)
                .filter(|c| {
                    if typed_known {
                        c.score > typed_score + KNOWN_WORD_MARGIN && c.edits <= 1
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
    max_edits: u8,
    budget: u32,
    /// (word id, channel cost, edits, node for completions)
    out: Vec<(u32, f32, u8, Option<u32>)>,
}

impl Search<'_> {
    fn run(&mut self) {
        self.go(self.trie.root(), 0, 0.0, 0, None);
    }

    fn go(&mut self, n: u32, i: usize, cost: f32, edits: u8, last: Option<char>) {
        if cost > MAX_COST || self.budget == 0 {
            return;
        }
        self.budget -= 1;
        let taps = self.taps;
        if i == taps.len() {
            let node = self.trie.node(n);
            let word = (node.word != NONE).then_some(node.word);
            if let Some(w) = word {
                self.out.push((w, cost, edits, None));
            }
            // completions only from close readings
            if edits <= 1 && taps.len() >= 2 {
                self.out.push((NONE, cost, edits, Some(n)));
            }
        }
        let kids: &[(char, u32)] = self.trie.children(n);
        if i < taps.len() {
            let tap = &taps[i];
            // match
            for &(c, k) in kids {
                if let Some(cc) = self.touch.cost(tap, c) {
                    if cc <= MAX_TAP_COST {
                        let far = (cc > FAR_TAP) as u8;
                        if edits + far <= self.max_edits {
                            self.go(k, i + 1, cost + cc, edits + far, Some(c));
                        }
                    }
                }
            }
            if edits < self.max_edits {
                // extra tap
                let double = last == Some(tap.ch) || (i > 0 && taps[i - 1].ch == tap.ch);
                self.go(n, i + 1, cost + if double { COST_INSERT_DOUBLE } else { COST_INSERT }, edits + 1, last);
                // swapped neighbours
                if i + 1 < taps.len() {
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
                                    self.go(k2, i + 2, cost + COST_SWAP + a + b, edits + 1, Some(c2));
                                }
                            }
                        }
                    }
                }
            }
        }
        // missed letter
        if edits < self.max_edits && i > 0 {
            for &(c, k) in kids {
                let double = last == Some(c);
                self.go(k, i, cost + if double { COST_DELETE_DOUBLE } else { COST_DELETE }, edits + 1, Some(c));
            }
        }
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
