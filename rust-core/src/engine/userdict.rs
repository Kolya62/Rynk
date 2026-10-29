// SPDX-License-Identifier: GPL-3.0-or-later
//! Personal dictionary: words and word pairs the user typed, stored per language.
//!
//! A new word becomes "known" (never autocorrected away, offered as a suggestion) after it is
//! typed twice, or at once when the user undoes an autocorrection of it.

use std::collections::HashMap;

/// Uses after which an unknown word is trusted
pub const KNOWN_AFTER: u32 = 2;
const MAX_WORDS: usize = 20_000;
const MAX_PAIRS: usize = 40_000;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct UserDict {
    /// lowercase → (spelling as typed, uses)
    pub words: HashMap<String, (String, u32)>,
    /// "prev next" (lowercase) → uses
    pub pairs: HashMap<String, u32>,
    pub dirty: bool,
}

impl UserDict {
    pub fn add_word(&mut self, word: &str, uses: u32) {
        let lower = word.to_lowercase();
        if lower.chars().count() < 2 || lower.chars().count() > 40 {
            return;
        }
        let e = self.words.entry(lower).or_insert_with(|| (word.to_string(), 0));
        e.1 = e.1.saturating_add(uses);
        // keep a capitalized spelling only if the user keeps typing it that way mid-sentence
        if word.chars().next().is_some_and(char::is_lowercase) {
            e.0 = word.to_string();
        }
        self.dirty = true;
        if self.words.len() > MAX_WORDS {
            prune(&mut self.words, |v| v.1);
        }
    }

    pub fn add_pair(&mut self, prev: &str, next: &str) {
        let key = format!("{} {}", prev.to_lowercase(), next.to_lowercase());
        *self.pairs.entry(key).or_default() += 1;
        self.dirty = true;
        if self.pairs.len() > MAX_PAIRS {
            prune(&mut self.pairs, |v| *v);
        }
    }

    pub fn forget(&mut self, word: &str) {
        let lower = word.to_lowercase();
        self.words.remove(&lower);
        self.pairs.retain(|k, _| !k.split(' ').any(|w| w == lower));
        self.dirty = true;
    }

    pub fn uses(&self, lower: &str) -> u32 {
        self.words.get(lower).map_or(0, |w| w.1)
    }

    pub fn is_known(&self, lower: &str) -> bool {
        self.uses(lower) >= KNOWN_AFTER
    }

    /// log10 probability of a learned word (comparable with corpus unigrams)
    pub fn logp(&self, lower: &str) -> Option<f32> {
        let n = self.uses(lower);
        (n > 0).then(|| -5.5 + (n as f32).log10().min(3.0))
    }

    /// log10 P(next | prev) from the user's own pairs
    pub fn pair_logp(&self, prev: &str, next_lower: &str) -> Option<f32> {
        let n = *self.pairs.get(&format!("{} {next_lower}", prev.to_lowercase()))?;
        let total: u32 = self.pair_total(prev);
        Some((n as f32 / (total as f32 + 2.0)).log10() + if n >= 2 { 0.3 } else { -0.4 })
    }

    fn pair_total(&self, prev: &str) -> u32 {
        let p = format!("{} ", prev.to_lowercase());
        self.pairs.iter().filter(|(k, _)| k.starts_with(&p)).map(|(_, v)| *v).sum()
    }

    /// Learned continuations of `prev`, best first
    pub fn continuations(&self, prev: &str) -> Vec<(String, f32)> {
        let p = format!("{} ", prev.to_lowercase());
        let total: u32 = self.pair_total(prev);
        let mut v: Vec<(String, f32)> = self
            .pairs
            .iter()
            .filter(|(k, n)| k.starts_with(&p) && **n >= 2)
            .map(|(k, n)| (k[p.len()..].to_string(), (*n as f32 / (total as f32 + 2.0)).log10() + 0.3))
            .collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        v
    }

    pub fn serialize(&self) -> String {
        let mut s = String::new();
        for (k, (spelling, n)) in &self.words {
            if spelling.to_lowercase() == *k {
                s.push_str(&format!("w\t{spelling}\t{n}\n"));
            }
        }
        for (k, n) in &self.pairs {
            s.push_str(&format!("p\t{k}\t{n}\n"));
        }
        s
    }

    pub fn parse(text: &str) -> Self {
        let mut d = UserDict::default();
        for line in text.lines() {
            let mut it = line.split('\t');
            match (it.next(), it.next(), it.next().and_then(|n| n.parse::<u32>().ok())) {
                (Some("w"), Some(w), Some(n)) => {
                    d.words.insert(w.to_lowercase(), (w.to_string(), n));
                }
                (Some("p"), Some(k), Some(n)) if k.contains(' ') => {
                    d.pairs.insert(k.to_string(), n);
                }
                _ => {}
            }
        }
        d
    }
}

/// Drops the least used quarter
fn prune<V>(map: &mut HashMap<String, V>, uses: impl Fn(&V) -> u32) {
    let mut counts: Vec<u32> = map.values().map(&uses).collect();
    counts.sort_unstable();
    let cut = counts[counts.len() / 4];
    map.retain(|_, v| uses(v) > cut);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_and_roundtrips() {
        let mut d = UserDict::default();
        d.add_word("кринж", 1);
        assert!(!d.is_known("кринж"));
        d.add_word("кринж", 1);
        assert!(d.is_known("кринж"));
        d.add_pair("полный", "кринж");
        d.add_pair("полный", "кринж");
        assert_eq!(d.continuations("Полный")[0].0, "кринж");
        let back = UserDict::parse(&d.serialize());
        assert_eq!(back.words, d.words);
        assert_eq!(back.pairs, d.pairs);
    }
}
