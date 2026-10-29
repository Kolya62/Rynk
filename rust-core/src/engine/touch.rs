// SPDX-License-Identifier: GPL-3.0-or-later
//! Spatial touch model: each key is a 2D Gaussian around its (personally calibrated) center.
//!
//! Used twice, as in Yandex Keyboard's design:
//! * the dynamic grid picks the intended letter of a tap from geometry × next-letter probability,
//!   while never growing a key's hit area by more than 50%;
//! * the noisy-channel decoder scores how likely a tap sequence is for a candidate word.

use std::collections::HashMap;

/// Key half-size → standard deviation of touches (fitted to typical thumb scatter)
const SIGMA: f32 = 0.42;
/// A key's effective hit box may grow to at most 150% of its size (75% of size from center)
const MAX_REACH: f32 = 0.75;
/// Weight of the language prior in the dynamic grid (log domain)
const GRID_LM_WEIGHT: f32 = 0.55;
/// Floor of next-letter probability so no letter is ever unreachable
const GRID_MIN_P: f32 = 1e-4;
/// Personal calibration: learning rate and the largest shift as a share of key size
const CALIB_RATE: f32 = 0.08;
const CALIB_MAX: f32 = 0.25;
const LN10: f32 = std::f32::consts::LN_10;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyGeom {
    /// Lowercase letter the key types
    pub ch: char,
    pub cx: f32,
    pub cy: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tap {
    /// Letter the keyboard committed for this tap (lowercase)
    pub ch: char,
    /// Touch position, or None when the letter was chosen exactly (long-press popup, paste)
    pub pos: Option<(f32, f32)>,
}

#[derive(Debug, Default, Clone)]
pub struct TouchModel {
    keys: Vec<KeyGeom>,
    by_char: HashMap<char, usize>,
    /// Letters reachable through a key's long-press popup → (key index)
    variants: HashMap<char, usize>,
    /// Personal offset of touches per letter (dx, dy) in pixels
    calib: HashMap<char, (f32, f32)>,
}

impl TouchModel {
    pub fn new(keys: Vec<KeyGeom>, variants: &[(char, Vec<char>)]) -> Self {
        let by_char = keys.iter().enumerate().map(|(i, k)| (k.ch, i)).collect::<HashMap<_, _>>();
        let mut v = HashMap::new();
        for (base, alts) in variants {
            if let Some(&i) = by_char.get(base) {
                for a in alts {
                    if !by_char.contains_key(a) {
                        v.entry(*a).or_insert(i);
                    }
                }
            }
        }
        Self { keys, by_char, variants: v, calib: HashMap::new() }
    }

    pub fn keys(&self) -> &[KeyGeom] {
        &self.keys
    }

    pub fn calibration(&self) -> &HashMap<char, (f32, f32)> {
        &self.calib
    }

    pub fn set_calibration(&mut self, c: HashMap<char, (f32, f32)>) {
        self.calib = c;
    }

    fn center(&self, k: &KeyGeom) -> (f32, f32) {
        let (ox, oy) = self.calib.get(&k.ch).copied().unwrap_or((0.0, 0.0));
        (k.cx + ox, k.cy + oy)
    }

    /// Natural-log likelihood of a touch for key `k` (unnormalized Gaussian)
    fn ln_lik(&self, k: &KeyGeom, x: f32, y: f32) -> f32 {
        let (cx, cy) = self.center(k);
        let sx = SIGMA * k.w;
        let sy = SIGMA * k.h;
        let dx = (x - cx) / sx;
        let dy = (y - cy) / sy;
        -0.5 * (dx * dx + dy * dy)
    }

    /// Key for a letter: its own key, or the key whose popup holds it (+ log10 penalty)
    pub fn key_for(&self, ch: char) -> Option<(&KeyGeom, f32)> {
        if let Some(&i) = self.by_char.get(&ch) {
            return Some((&self.keys[i], 0.0));
        }
        self.variants.get(&ch).map(|&i| (&self.keys[i], 0.35))
    }

    /// Channel cost (−log10, ≥ 0) of tapping at `tap` while meaning `ch`, relative to the
    /// most likely key at that point. `None` when the letter cannot be typed on this layout.
    pub fn cost(&self, tap: &Tap, ch: char) -> Option<f32> {
        match tap.pos {
            None => {
                if tap.ch == ch {
                    Some(0.0)
                } else if self.key_for(ch).map(|k| k.0.ch) == self.key_for(tap.ch).map(|k| k.0.ch) {
                    // same key, other popup variant: "e" for "é"
                    Some(0.6)
                } else {
                    Some(3.0)
                }
            }
            Some((x, y)) => {
                let (k, penalty) = self.key_for(ch)?;
                let best = self
                    .keys
                    .iter()
                    .map(|kk| self.ln_lik(kk, x, y))
                    .fold(f32::NEG_INFINITY, f32::max);
                Some(((best - self.ln_lik(k, x, y)) / LN10).max(0.0) + penalty)
            }
        }
    }

    /// Dynamic grid: the letter meant by a touch at (x, y) given next-letter probabilities.
    /// With an empty distribution it is the plain nearest-key decision.
    pub fn resolve(&self, x: f32, y: f32, next: &[(char, f32)]) -> Option<char> {
        let nearest = self.nearest(x, y)?;
        if next.is_empty() {
            return Some(nearest);
        }
        let mut best = (nearest, f32::NEG_INFINITY);
        for k in &self.keys {
            let (cx, cy) = (k.cx, k.cy);
            let inside_reach = (x - cx).abs() <= MAX_REACH * k.w && (y - cy).abs() <= MAX_REACH * k.h;
            if !inside_reach {
                continue;
            }
            let p = next.iter().find(|n| n.0 == k.ch).map_or(GRID_MIN_P, |n| n.1.max(GRID_MIN_P));
            let s = self.ln_lik(k, x, y) + GRID_LM_WEIGHT * p.ln();
            if s > best.1 {
                best = (k.ch, s);
            }
        }
        Some(best.0)
    }

    /// Key whose (static) rectangle is nearest to the point
    pub fn nearest(&self, x: f32, y: f32) -> Option<char> {
        self.keys
            .iter()
            .map(|k| {
                let dx = ((x - k.cx).abs() - k.w / 2.0).max(0.0);
                let dy = ((y - k.cy).abs() - k.h / 2.0).max(0.0);
                (k.ch, dx * dx + dy * dy, (x - k.cx).powi(2) + (y - k.cy).powi(2))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1).then(a.2.total_cmp(&b.2)))
            .map(|k| k.0)
    }

    /// Personal calibration from a word the user accepted: shift each key's center toward
    /// where this user actually touches it.
    pub fn learn(&mut self, taps: &[Tap], word: &str) {
        let letters: Vec<char> = word.chars().flat_map(char::to_lowercase).collect();
        if letters.len() != taps.len() {
            return;
        }
        for (tap, &ch) in taps.iter().zip(&letters) {
            let (Some((x, y)), Some(&i)) = (tap.pos, self.by_char.get(&ch)) else { continue };
            let k = self.keys[i];
            let e = self.calib.entry(ch).or_insert((0.0, 0.0));
            e.0 = (e.0 + CALIB_RATE * (x - k.cx - e.0)).clamp(-CALIB_MAX * k.w, CALIB_MAX * k.w);
            e.1 = (e.1 + CALIB_RATE * (y - k.cy - e.1)).clamp(-CALIB_MAX * k.h, CALIB_MAX * k.h);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 10-column grid of keys for tests: rows of letters, keys 10×15 px
    pub fn grid(rows: &[&str]) -> TouchModel {
        let mut keys = Vec::new();
        for (r, row) in rows.iter().enumerate() {
            let n = row.chars().count() as f32;
            let off = (11.0 - n) * 5.0;
            for (c, ch) in row.chars().enumerate() {
                keys.push(KeyGeom { ch, cx: off + c as f32 * 10.0 + 5.0, cy: r as f32 * 15.0 + 7.5, w: 10.0, h: 15.0 });
            }
        }
        TouchModel::new(keys, &[('е', vec!['ё']), ('ь', vec!['ъ'])])
    }

    pub fn ru() -> TouchModel {
        grid(&["йцукенгшщзх", "фывапролджэ", "ячсмитьбю"])
    }

    #[test]
    fn grid_grows_likely_key_but_not_beyond_half() {
        let m = ru();
        let a = m.keys().iter().find(|k| k.ch == 'а').unwrap();
        // touch a bit past the right edge of "а", into "п"
        let x = a.cx + a.w * 0.6;
        assert_eq!(m.nearest(x, a.cy), Some('п'));
        assert_eq!(m.resolve(x, a.cy, &[('а', 0.95), ('п', 0.01)]), Some('а'));
        // far past the 150% reach: the likely key can no longer win
        assert_eq!(m.resolve(a.cx + a.w * 0.9, a.cy, &[('а', 0.99), ('п', 0.0001)]), Some('п'));
    }

    #[test]
    fn cost_is_zero_at_center_and_grows() {
        let m = ru();
        let k = *m.keys().iter().find(|k| k.ch == 'к').unwrap();
        let tap = Tap { ch: 'к', pos: Some((k.cx, k.cy)) };
        assert!(m.cost(&tap, 'к').unwrap() < 1e-3);
        assert!(m.cost(&tap, 'у').unwrap() > 0.5);
        assert!(m.cost(&tap, 'ё').is_some());
    }

    #[test]
    fn calibration_moves_toward_touches() {
        let mut m = ru();
        let k = *m.keys().iter().find(|k| k.ch == 'а').unwrap();
        for _ in 0..50 {
            m.learn(&[Tap { ch: 'а', pos: Some((k.cx + 2.0, k.cy + 3.0)) }], "а");
        }
        let (dx, dy) = m.calibration()[&'а'];
        assert!((dx - 2.0).abs() < 0.2 && (dy - 3.0).abs() < 0.3);
    }
}
