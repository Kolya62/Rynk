// SPDX-License-Identifier: GPL-3.0-or-later
//! Corpus language model (`<lang>.rlm`, built offline) and n-gram scoring.
//!
//! ```text
//! header:  "RLM1" | u16 version | u16 reserved | u32 adler32(payload)
//! payload: varint n, n × [varint len, utf8, varint count, u16 freq]   (ids by descending freq)
//!          bigram groups, trigram groups:
//!          varint n_groups, per group: varint key delta, varint k, k × [varint id, u8 q]
//! ```
//!
//! RLM2 (`tools/rlm2.py`) is the same model packed denser: words as codes of a per-file
//! alphabet with a delta-coded `q` of their unigram probability instead of counts, and n-gram
//! ids delta-coded in ascending order followed by their `q / 6`.
//!
//! Bigram key = ctx(prev1); trigram key = ctx(prev2) << 32 | ctx(prev1); ctx(id) = id + 1 and
//! 0 is the sentence start. `q = round(-log10(p) * 32)`.

use std::collections::HashMap;

const MAGIC: &[u8; 4] = b"RLM1";
const MAGIC2: &[u8; 4] = b"RLM2";
const Q_SCALE: f32 = 32.0;
/// Stupid backoff weight per dropped order, log10(0.4)
pub const BACKOFF: f32 = -0.398;
pub const SENTENCE_START: u32 = 0;

#[derive(Clone, Debug)]
pub struct Word {
    /// Canonical spelling ("Москва", "macOS", "привет")
    pub text: String,
    /// log10 unigram probability
    pub logp: f32,
}

#[derive(Default, Debug)]
pub struct Ngrams {
    keys: Vec<u64>,
    offsets: Vec<u32>,
    entries: Vec<(u32, u8)>,
}

impl Ngrams {
    /// Continuations of `key`, most probable first
    pub fn get(&self, key: u64) -> &[(u32, u8)] {
        match self.keys.binary_search(&key) {
            Ok(i) => {
                let s = self.offsets[i] as usize;
                let e = self.offsets.get(i + 1).map_or(self.entries.len(), |&o| o as usize);
                &self.entries[s..e]
            }
            Err(_) => &[],
        }
    }

    pub fn logp(&self, key: u64, id: u32) -> Option<f32> {
        self.get(key).iter().find(|e| e.0 == id).map(|e| -(e.1 as f32) / Q_SCALE)
    }

    /// RLM2 groups: ascending delta-coded ids, then their coarse q; stored most probable first
    fn read2(r: &mut Reader, n_words: u32) -> Option<Self> {
        let n = r.varint()? as usize;
        let mut t = Ngrams {
            keys: Vec::with_capacity(n.min(1 << 22)),
            offsets: Vec::with_capacity(n.min(1 << 22)),
            entries: Vec::new(),
        };
        let mut key = 0u64;
        let mut ids: Vec<u32> = Vec::new();
        for _ in 0..n {
            key = key.checked_add(r.varint()?)?;
            let k = r.varint()? as usize;
            t.keys.push(key);
            let start = t.entries.len();
            t.offsets.push(start as u32);
            ids.clear();
            let mut id = 0u64;
            for _ in 0..k {
                id = id.checked_add(r.varint()?)?;
                if id >= n_words as u64 {
                    return None;
                }
                ids.push(id as u32);
            }
            for &id in &ids {
                let q = (r.u8()? as u32 * 6).min(255) as u8;
                t.entries.push((id, q));
            }
            t.entries[start..].sort_by_key(|e| (e.1, e.0));
        }
        Some(t)
    }

    fn read(r: &mut Reader, n_words: u32) -> Option<Self> {
        let n = r.varint()? as usize;
        let mut t = Ngrams {
            keys: Vec::with_capacity(n.min(1 << 22)),
            offsets: Vec::with_capacity(n.min(1 << 22)),
            entries: Vec::new(),
        };
        let mut key = 0u64;
        for _ in 0..n {
            key = key.checked_add(r.varint()?)?;
            let k = r.varint()? as usize;
            t.keys.push(key);
            t.offsets.push(t.entries.len() as u32);
            for _ in 0..k {
                let id = u32::try_from(r.varint()?).ok()?;
                if id >= n_words {
                    return None;
                }
                t.entries.push((id, r.u8()?));
            }
        }
        Some(t)
    }
}

#[derive(Default, Debug)]
pub struct LanguageModel {
    pub words: Vec<Word>,
    /// lowercase spelling → id (the most frequent spelling wins)
    pub index: HashMap<String, u32>,
    pub bigrams: Ngrams,
    pub trigrams: Ngrams,
}

/// Previous words of the word being typed, nearest first. `None` = unknown, sentence start is
/// expressed by `start = true`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Context {
    pub prev: Vec<String>,
    pub sentence_start: bool,
}

impl LanguageModel {
    pub fn decode(data: &[u8]) -> Option<Self> {
        let data = &*crate::engine::unpack(data)?;
        let v2 = data.len() >= 12 && &data[..4] == MAGIC2 && u16::from_le_bytes([data[4], data[5]]) == 2;
        if !v2 && (data.len() < 12 || &data[..4] != MAGIC || u16::from_le_bytes([data[4], data[5]]) != 1) {
            return None;
        }
        let sum = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let payload = &data[12..];
        if adler32(payload) != sum {
            return None;
        }
        let mut r = Reader { d: payload, p: 0 };
        if v2 {
            return Self::decode2(&mut r);
        }
        let n = u32::try_from(r.varint()?).ok()?;
        let mut raw = Vec::with_capacity((n as usize).min(payload.len()));
        let mut total = 0f64;
        for _ in 0..n {
            let len = r.varint()? as usize;
            let text = std::str::from_utf8(r.bytes(len)?).ok()?.to_string();
            let count = r.varint()?;
            let freq = u16::from_le_bytes([r.u8()?, r.u8()?]);
            total += count as f64;
            raw.push((text, count, freq));
        }
        let total = total.max(1.0);
        let words: Vec<Word> = raw
            .into_iter()
            .map(|(text, count, freq)| {
                // Corpus count when present, else the 0..2500 ranking scale (curated words)
                let logp = if count > 0 {
                    ((count as f64) / total).log10() as f32
                } else {
                    -7.0 + 5.7 * (freq.min(2500) as f32 / 2500.0)
                };
                Word { text, logp: logp.max(-9.0) }
            })
            .collect();
        let bigrams = Ngrams::read(&mut r, n)?;
        let trigrams = Ngrams::read(&mut r, n)?;
        let mut index = HashMap::with_capacity(words.len());
        for (i, w) in words.iter().enumerate() {
            index.entry(w.text.to_lowercase()).or_insert(i as u32);
        }
        Some(Self { words, index, bigrams, trigrams })
    }

    fn decode2(r: &mut Reader) -> Option<Self> {
        let n_alpha = r.varint()? as usize;
        let mut alpha = Vec::with_capacity(n_alpha.min(255));
        for _ in 0..n_alpha.min(255) {
            alpha.push(char::from_u32(u32::try_from(r.varint()?).ok()?)?);
        }
        let n = u32::try_from(r.varint()?).ok()?;
        let mut words = Vec::with_capacity((n as usize).min(r.d.len()));
        let mut q = 0i64;
        for _ in 0..n {
            let len = r.varint()? as usize;
            let mut text = String::with_capacity(len * 2);
            for _ in 0..len {
                let c = r.u8()?;
                if c == 255 {
                    text.push(char::from_u32(u32::try_from(r.varint()?).ok()?)?);
                } else {
                    text.push(*alpha.get(c as usize)?);
                }
            }
            let z = r.varint()? as i64;
            q += if z & 1 == 0 { z >> 1 } else { -((z + 1) >> 1) };
            words.push(Word { text, logp: -(q as f32) / Q_SCALE });
        }
        let bigrams = Ngrams::read2(r, n)?;
        let trigrams = Ngrams::read2(r, n)?;
        let mut index = HashMap::with_capacity(words.len());
        for (i, w) in words.iter().enumerate() {
            index.entry(w.text.to_lowercase()).or_insert(i as u32);
        }
        Some(Self { words, index, bigrams, trigrams })
    }

    pub fn id(&self, lower: &str) -> Option<u32> {
        self.index.get(lower).copied()
    }

    fn ctx_key(&self, word: Option<&String>, start: bool) -> Option<u32> {
        match word {
            Some(w) => self.id(&w.to_lowercase()).map(|i| i + 1),
            None if start => Some(SENTENCE_START),
            None => None,
        }
    }

    /// Context keys (prev2, prev1)
    pub fn keys(&self, ctx: &Context) -> (Option<u32>, Option<u32>) {
        let k1 = self.ctx_key(ctx.prev.first(), ctx.sentence_start);
        let k2 = if ctx.prev.is_empty() {
            None
        } else {
            k1.and_then(|_| self.ctx_key(ctx.prev.get(1), ctx.sentence_start && ctx.prev.len() == 1))
        };
        (k2, k1)
    }

    /// log10 P(word | context) with stupid backoff
    pub fn score(&self, id: u32, keys: (Option<u32>, Option<u32>)) -> f32 {
        if let (Some(k2), Some(k1)) = keys {
            if let Some(p) = self.trigrams.logp(((k2 as u64) << 32) | k1 as u64, id) {
                return p;
            }
        }
        if let Some(k1) = keys.1 {
            if let Some(p) = self.bigrams.logp(k1 as u64, id) {
                return p + BACKOFF;
            }
            return self.words[id as usize].logp + 2.0 * BACKOFF;
        }
        self.words[id as usize].logp
    }

    /// Most probable next words after the context (best first)
    pub fn predict(&self, keys: (Option<u32>, Option<u32>), limit: usize) -> Vec<(u32, f32)> {
        let mut out: Vec<(u32, f32)> = Vec::new();
        let push = |id: u32, p: f32, out: &mut Vec<(u32, f32)>| {
            if let Some(e) = out.iter_mut().find(|e| e.0 == id) {
                e.1 = e.1.max(p);
            } else {
                out.push((id, p));
            }
        };
        if let (Some(k2), Some(k1)) = keys {
            for &(id, q) in self.trigrams.get(((k2 as u64) << 32) | k1 as u64).iter().take(limit * 2) {
                push(id, -(q as f32) / Q_SCALE, &mut out);
            }
        }
        if let Some(k1) = keys.1 {
            for &(id, q) in self.bigrams.get(k1 as u64).iter().take(limit * 3) {
                push(id, -(q as f32) / Q_SCALE + BACKOFF, &mut out);
            }
        }
        out.sort_by(|a, b| b.1.total_cmp(&a.1));
        out.truncate(limit);
        out
    }
}

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Option<u8> {
        let b = *self.d.get(self.p)?;
        self.p += 1;
        Some(b)
    }
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let e = self.p.checked_add(n)?;
        let s = self.d.get(self.p..e)?;
        self.p = e;
        Some(s)
    }
    fn varint(&mut self) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.u8()?;
            v |= ((b & 0x7F) as u64) << shift;
            if b & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }
}

pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn load(code: &str) -> Option<LanguageModel> {
        let path = format!("{}/../app/src/main/assets/lm/{code}.rlm", env!("CARGO_MANIFEST_DIR"));
        LanguageModel::decode(&std::fs::read(path).ok()?)
    }

    #[test]
    fn russian_model_loads_and_scores_context() {
        let lm = load("ru").expect("ru.rlm");
        assert!(lm.words.len() > 50_000);
        let ctx = Context { prev: vec!["как".into()], sentence_start: true };
        let keys = lm.keys(&ctx);
        let dela = lm.id("дела").unwrap();
        let stol = lm.id("стол").unwrap();
        assert!(lm.score(dela, keys) > lm.score(stol, keys));
        let pred = lm.predict(keys, 5);
        assert!(!pred.is_empty());
    }

    #[test]
    fn corrupted_rejected() {
        assert!(LanguageModel::decode(b"RLM1\x01\0\0\0\0\0\0\0junk").is_none());
    }
}
