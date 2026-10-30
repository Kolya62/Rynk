// SPDX-License-Identifier: GPL-3.0-or-later
//! Gesture (swipe) typing: the finger draws a path across the letters of a word.
//!
//! Every dictionary word has an ideal path: the polyline through the centers of its keys.
//! A word's channel cost compares the drawn path with that ideal path in two ways:
//! * location: both paths resampled to the same number of points, mean distance between
//!   corresponding points (in key widths);
//! * coverage: every letter's key must lie close to the drawn path, in order.
//! The language model then weighs the candidates, as in tap decoding.

use super::touch::TouchModel;

/// Points each path is resampled to
const SAMPLES: usize = 40;
/// Weight of the mean point distance (key widths) in log10 units
const W_LOCATION: f32 = 2.2;
/// Weight of the worst letter miss beyond half a key width
const W_MISS: f32 = 3.0;
/// Weight of the start and end points being away from the first and last letters
const W_ENDS: f32 = 1.2;
/// Candidates above this channel cost are dropped
const MAX_COST: f32 = 6.0;

/// Geometry of a layout for gesture decoding
pub struct GestureKeys<'a> {
    touch: &'a TouchModel,
    key_w: f32,
}

impl<'a> GestureKeys<'a> {
    pub fn new(touch: &'a TouchModel) -> Self {
        let mut ws: Vec<f32> = touch.keys().iter().map(|k| k.w).collect();
        ws.sort_by(f32::total_cmp);
        let key_w = ws.get(ws.len() / 2).copied().unwrap_or(1.0).max(1.0);
        Self { touch, key_w }
    }

    pub fn key_w(&self) -> f32 {
        self.key_w
    }

    fn center(&self, ch: char) -> Option<(f32, f32)> {
        self.touch.key_for(ch).map(|(k, _)| (k.cx, k.cy))
    }

    /// Letters whose keys lie near a point, nearest first
    pub fn near(&self, p: (f32, f32), radius: f32) -> Vec<char> {
        let mut v: Vec<(f32, char)> = self
            .touch
            .keys()
            .iter()
            .map(|k| (dist((k.cx, k.cy), p), k.ch))
            .filter(|(d, _)| *d <= radius * self.key_w)
            .collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v.into_iter().map(|x| x.1).collect()
    }

    /// The ideal path of a lowercase word, or None when a letter has no key
    pub fn ideal(&self, word: &str) -> Option<Vec<(f32, f32)>> {
        let mut pts: Vec<(f32, f32)> = Vec::new();
        for ch in word.chars() {
            let c = self.center(ch)?;
            if pts.last() != Some(&c) {
                pts.push(c);
            }
        }
        if pts.is_empty() {
            None
        } else {
            Some(pts)
        }
    }

    /// Channel cost (log10 units) of drawing `path` for a word with the given ideal path
    pub fn cost(&self, path: &Path, ideal: &[(f32, f32)]) -> Option<f32> {
        let w = self.key_w;
        let len = polyline_len(ideal);
        // a long word cannot come from a short stroke, nor the reverse
        if len > path.len * 1.8 + 1.5 * w || path.len > len * 1.8 + 1.5 * w {
            return None;
        }
        let ends = (dist(path.pts[0], ideal[0]) + dist(*path.pts.last().unwrap(), *ideal.last().unwrap())) / w;
        if ends > 2.2 {
            return None;
        }
        let ideal_s = resample(ideal, SAMPLES);
        let loc = path.samples.iter().zip(&ideal_s).map(|(a, b)| dist(*a, *b)).sum::<f32>() / (SAMPLES as f32 * w);
        // every key must be passed near, in order: the best monotonic alignment of the
        // letters to path points, minimizing the summed squared misses
        let n = path.pts.len();
        let miss_of = |c: (f32, f32), p: (f32, f32)| ((dist(c, p) / w) - 0.5).max(0.0).powi(2);
        let mut best: Vec<f32> = path.pts.iter().map(|p| miss_of(ideal[0], *p)).collect();
        for &c in &ideal[1..] {
            let mut run = f32::INFINITY;
            for i in 0..n {
                run = run.min(best[i]);
                best[i] = run + miss_of(c, path.pts[i]);
            }
        }
        let total = best.iter().copied().fold(f32::INFINITY, f32::min);
        let worst = 0.5 + total.sqrt();
        let miss = (worst - 0.5).max(0.0);
        let cost = W_LOCATION * loc * loc + W_MISS * miss * miss + W_ENDS * ends;
        (cost <= MAX_COST).then_some(cost)
    }
}

/// A drawn path prepared for comparison
pub struct Path {
    /// The raw points, densified so that consecutive points are close
    pub pts: Vec<(f32, f32)>,
    pub samples: Vec<(f32, f32)>,
    pub len: f32,
}

impl Path {
    pub fn new(raw: &[(f32, f32)], key_w: f32) -> Option<Self> {
        if raw.len() < 2 {
            return None;
        }
        let len = polyline_len(raw);
        // dense points: the coverage test needs a point near every key the finger crossed
        let n = ((len / (key_w * 0.2)).ceil() as usize).clamp(2, 400);
        let pts = resample(raw, n);
        Some(Self { samples: resample(raw, SAMPLES), pts, len })
    }
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn polyline_len(p: &[(f32, f32)]) -> f32 {
    p.windows(2).map(|w| dist(w[0], w[1])).sum()
}

/// `n` points evenly spaced along a polyline (a single point repeats)
pub fn resample(p: &[(f32, f32)], n: usize) -> Vec<(f32, f32)> {
    let total = polyline_len(p);
    if p.len() < 2 || total <= f32::EPSILON {
        return vec![p.first().copied().unwrap_or((0.0, 0.0)); n];
    }
    let step = total / (n - 1) as f32;
    let mut out = Vec::with_capacity(n);
    out.push(p[0]);
    let mut seg = 0;
    let mut seg_start = 0.0f32; // distance along the path where segment `seg` starts
    for i in 1..n - 1 {
        let target = step * i as f32;
        while seg < p.len() - 2 && seg_start + dist(p[seg], p[seg + 1]) < target {
            seg_start += dist(p[seg], p[seg + 1]);
            seg += 1;
        }
        let l = dist(p[seg], p[seg + 1]).max(f32::EPSILON);
        let t = ((target - seg_start) / l).clamp(0.0, 1.0);
        out.push((p[seg].0 + (p[seg + 1].0 - p[seg].0) * t, p[seg].1 + (p[seg + 1].1 - p[seg].1) * t));
    }
    out.push(*p.last().unwrap());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_is_even() {
        let r = resample(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], 5);
        assert_eq!(r.len(), 5);
        assert!((r[2].0 - 10.0).abs() < 1e-3 && r[2].1.abs() < 1e-3);
        assert_eq!(r[4], (10.0, 10.0));
    }
}
