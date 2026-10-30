// SPDX-License-Identifier: GPL-3.0-or-later
//! Reading → word conversion for Chinese (pinyin, zhuyin), Cantonese (jyutping) and Japanese
//! (kana → kanji). Dictionaries are built offline by `tools/cjkgen.py` (`<code>.rcd`).
//!
//! The best conversion of the whole input is a Viterbi path over dictionary words, so a phrase
//! ("zhongguoren", "たべた") converts at once; single words, completions of an unfinished
//! syllable and conversions of a prefix are offered as well.

use super::userdict::UserDict;
use std::ops::Range;

const Q_SCALE: f32 = 16.0;
/// Cost of starting a new word in a phrase (log10): prefers fewer, longer words
const SEGMENT_COST: f32 = 0.6;
/// A character left unconverted (kana stays kana)
const RAW_CHAR_COST: f32 = 5.5;
/// Ending a phrase on an unfinished syllable
const COMPLETION_COST: f32 = 1.5;
/// Per letter the completion adds beyond what was typed
const COMPLETION_PER_CHAR: f32 = 0.35;
const MAX_KEY: usize = 24;

pub struct ConvDict {
    keys: Vec<Box<str>>,
    words: Vec<Box<str>>,
    logp: Vec<f32>,
}

/// A conversion candidate: replaces the first `consumed` reading characters with `text`
#[derive(Clone, Debug, PartialEq)]
pub struct Conv {
    pub text: String,
    pub consumed: usize,
    pub score: f32,
}

impl ConvDict {
    pub fn decode(data: &[u8]) -> Option<Self> {
        let data = &*super::unpack(data)?;
        if data.len() < 12 || &data[..4] != b"RCD1" {
            return None;
        }
        let sum = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let p = &data[12..];
        if super::lm::adler32(p) != sum {
            return None;
        }
        let mut i = 0usize;
        let varint = |i: &mut usize| -> Option<usize> {
            let mut v = 0usize;
            for shift in (0..35).step_by(7) {
                let b = *p.get(*i)?;
                *i += 1;
                v |= ((b & 0x7F) as usize) << shift;
                if b & 0x80 == 0 {
                    return Some(v);
                }
            }
            None
        };
        let n = varint(&mut i)?;
        let mut d = ConvDict { keys: Vec::with_capacity(n), words: Vec::with_capacity(n), logp: Vec::with_capacity(n) };
        for _ in 0..n {
            let l = varint(&mut i)?;
            let k = std::str::from_utf8(p.get(i..i + l)?).ok()?;
            i += l;
            let l = varint(&mut i)?;
            let w = std::str::from_utf8(p.get(i..i + l)?).ok()?;
            i += l;
            let q = *p.get(i)?;
            i += 1;
            d.keys.push(k.into());
            d.words.push(w.into());
            d.logp.push(-(q as f32) / Q_SCALE);
        }
        Some(d)
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Entries whose key is exactly `key` (best first)
    fn exact(&self, key: &str) -> Range<usize> {
        let a = self.keys.partition_point(|k| &**k < key);
        let b = a + self.keys[a..].partition_point(|k| &**k == key);
        a..b
    }

    /// Entries whose key starts with `prefix`
    fn prefixed(&self, prefix: &str) -> Range<usize> {
        let a = self.keys.partition_point(|k| &**k < prefix);
        let b = a + self.keys[a..].partition_point(|k| k.starts_with(prefix));
        a..b
    }

    fn score(&self, i: usize, user: &UserDict) -> f32 {
        let uses = user.uses(&self.words[i]);
        self.logp[i] + if uses > 0 { 1.5 + 0.8 * (uses as f32).log10() } else { 0.0 }
    }

    fn best_in(&self, r: Range<usize>, user: &UserDict) -> Option<usize> {
        r.max_by(|&a, &b| self.score(a, user).total_cmp(&self.score(b, user)))
    }

    /// Best conversion of the whole reading as a phrase
    fn phrase(&self, chars: &[char], user: &UserDict, raw_allowed: bool) -> Option<(String, f32, usize)> {
        let n = chars.len();
        // best[i] = (score, from, text)
        let mut best: Vec<Option<(f32, usize, String)>> = vec![None; n + 1];
        best[0] = Some((0.0, 0, String::new()));
        let mut key = String::new();
        for i in 0..n {
            let Some((base, _, _)) = best[i].clone() else { continue };
            key.clear();
            for j in i + 1..=(i + MAX_KEY).min(n) {
                key.push(chars[j - 1]);
                if let Some(e) = self.best_in(self.exact(&key), user) {
                    let s = base + self.score(e, user) - SEGMENT_COST;
                    if best[j].as_ref().is_none_or(|b| s > b.0) {
                        best[j] = Some((s, i, self.words[e].to_string()));
                    }
                }
            }
            if raw_allowed {
                let s = base - RAW_CHAR_COST;
                if best[i + 1].as_ref().is_none_or(|b| s > b.0) {
                    best[i + 1] = Some((s, i, chars[i].to_string()));
                }
            }
            // an unfinished last syllable: complete it with its most likely word
            if i > 0 || n <= MAX_KEY {
                let rest: String = chars[i..].iter().collect();
                if !rest.is_empty() && i < n {
                    let r = self.prefixed(&rest);
                    if !r.is_empty() && r.len() < 50_000 {
                        let r = r.start..r.end.min(r.start + 4000);
                        if let Some(e) = self.best_in(r, user) {
                            let extra = self.keys[e].chars().count().saturating_sub(n - i) as f32;
                            let s = base + self.score(e, user) - SEGMENT_COST - COMPLETION_COST - COMPLETION_PER_CHAR * extra;
                            if best[n].as_ref().is_none_or(|b| s > b.0) {
                                best[n] = Some((s, i, self.words[e].to_string()));
                            }
                        }
                    }
                }
            }
        }
        // walk back
        let (score, _, _) = best[n].clone()?;
        let mut parts = Vec::new();
        let mut j = n;
        let mut segments = 0;
        while j > 0 {
            let (_, from, text) = best[j].clone()?;
            parts.push(text);
            segments += 1;
            j = from;
        }
        parts.reverse();
        Some((parts.concat(), score, segments))
    }

    /// Candidates for a reading, best first
    pub fn convert(&self, reading: &str, user: &UserDict, raw_allowed: bool, limit: usize) -> Vec<Conv> {
        let chars: Vec<char> = reading.chars().collect();
        let n = chars.len();
        let mut out: Vec<Conv> = Vec::new();
        let push = |out: &mut Vec<Conv>, text: &str, consumed: usize, score: f32| {
            if !out.iter().any(|c| c.text == text && c.consumed == consumed) {
                out.push(Conv { text: text.to_string(), consumed, score });
            }
        };
        if n == 0 || self.is_empty() {
            return out;
        }
        // 1. the whole reading: the phrase and single words, best first
        let mut whole: Vec<Conv> = Vec::new();
        if let Some((text, score, segments)) = self.phrase(&chars, user, raw_allowed) {
            if segments > 1 {
                whole.push(Conv { text, consumed: n, score });
            }
        }
        for e in self.exact(reading) {
            whole.push(Conv { text: self.words[e].to_string(), consumed: n, score: self.score(e, user) });
        }
        whole.sort_by(|a, b| b.score.total_cmp(&a.score));
        for c in whole.into_iter().take(limit) {
            push(&mut out, &c.text, n, c.score);
        }
        // 2. completions of an unfinished reading (after every exact reading)
        let r = self.prefixed(reading);
        let mut comp: Vec<usize> = r.clone().take(6000).filter(|&e| self.keys[e].len() > reading.len()).collect();
        comp.sort_by(|&a, &b| self.score(b, user).total_cmp(&self.score(a, user)));
        let n_comp = if out.is_empty() { 8 } else { 4 };
        for e in comp.into_iter().take(n_comp) {
            push(&mut out, &self.words[e], n, self.score(e, user) - COMPLETION_COST);
        }
        // 3. words for prefixes, longest first
        let mut key: String;
        for j in (1..n).rev() {
            key = chars[..j].iter().collect();
            let mut ex: Vec<usize> = self.exact(&key).collect();
            ex.sort_by(|&a, &b| self.score(b, user).total_cmp(&self.score(a, user)));
            for e in ex.into_iter().take(if j + 1 == n { 8 } else { 4 }) {
                push(&mut out, &self.words[e], j, self.score(e, user) - 0.5);
            }
            if out.len() >= limit {
                break;
            }
        }
        out.truncate(limit);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(code: &str) -> ConvDict {
        let p = format!("{}/../app/src/main/assets/cjk/{code}.rcd", env!("CARGO_MANIFEST_DIR"));
        ConvDict::decode(&std::fs::read(p).unwrap()).unwrap()
    }

    #[test]
    fn pinyin_words_and_phrases() {
        let d = dict("zh-hans");
        let u = UserDict::default();
        assert_eq!(d.convert("nihao", &u, false, 10)[0].text, "你好");
        assert_eq!(d.convert("shi", &u, false, 10)[0].text, "是");
        assert_eq!(d.convert("woaini", &u, false, 10)[0].text, "我爱你");
        assert_eq!(d.convert("zhongguoren", &u, false, 10)[0].text, "中国人");
        let c = d.convert("xiexi", &u, false, 10);
        assert!(c.iter().any(|c| c.text == "谢谢"), "{c:?}");
    }

    #[test]
    fn traditional_and_cantonese() {
        let u = UserDict::default();
        assert_eq!(dict("zh-hant").convert("women", &u, false, 5)[0].text, "我們");
        let y = dict("yue");
        assert_eq!(y.convert("hai", &u, false, 5)[0].text, "係");
        assert_eq!(y.convert("mgoi", &u, false, 5)[0].text, "唔該");
    }

    #[test]
    fn japanese_inflections() {
        let d = dict("ja");
        let u = UserDict::default();
        assert_eq!(d.convert("にほん", &u, true, 10)[0].text, "日本");
        assert_eq!(d.convert("たべた", &u, true, 10)[0].text, "食べた");
        assert_eq!(d.convert("こーひー", &u, true, 10)[0].text, "コーヒー");
    }

    #[test]
    fn learning_reorders() {
        let d = dict("zh-hans");
        let mut u = UserDict::default();
        let before = d.convert("shi", &u, false, 10);
        let target = before[3].text.clone();
        u.add_word(&target, 5);
        assert_eq!(d.convert("shi", &u, false, 10)[0].text, target);
    }
}
