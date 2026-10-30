// SPDX-License-Identifier: GPL-3.0-or-later
//! Autocorrect evaluation: types corpus sentences through the real keyboard with noisy taps and
//! typical spelling mistakes, then compares what was committed with what was meant.
//!
//!     cargo run --release --example ac_eval -- <sentences.txt> [lang] [n_sentences] [sigma]
//!
//! sentences.txt: one sentence per line (a Leipzig `*-sentences.txt` works: "id<TAB>text").

use rynk_core::engine::Model;
use rynk_core::keyboard::{Cmd, Editor, Keyboard};
use rynk_core::layout::KeyKind;
use rynk_core::prefs::Prefs;
use std::sync::Arc;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn gauss(&mut self) -> f32 {
        let (u, v) = (self.next().max(1e-12), self.next());
        ((-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()) as f32
    }
}

/// Typical spelling (not motor) mistakes of Russian writers
fn spelling_error(w: &str, r: &mut Rng) -> Option<String> {
    let rules: &[(&str, &str)] = &[
        ("ться", "тся"), ("тся", "ться"), ("ё", "е"), ("нн", "н"), ("сс", "с"), ("ьз", "з"),
        ("о", "а"), ("е", "и"), ("и", "е"), ("а", "о"), ("щ", "ш"), ("чу", "чю"), ("жи", "жы"),
        ("ши", "шы"), ("ъ", "ь"), ("тс", "ц"), ("здн", "зн"), ("стн", "сн"), ("ь", ""),
    ];
    for _ in 0..6 {
        let (a, b) = rules[(r.next() * rules.len() as f64) as usize];
        if let Some(i) = w.find(a) {
            let mut s = w.to_string();
            s.replace_range(i..i + a.len(), b);
            if s != w {
                return Some(s);
            }
        }
    }
    None
}

fn apply(text: &mut String, comp: &mut Option<usize>, cmds: Vec<Cmd>) {
    for c in cmds {
        match c {
            Cmd::Composing(s) => {
                let start = comp.unwrap_or(text.len());
                text.truncate(start);
                text.push_str(&s);
                *comp = if s.is_empty() { None } else { Some(start) };
            }
            Cmd::Commit(s) => {
                if let Some(start) = comp.take() {
                    text.truncate(start);
                }
                text.push_str(&s);
            }
            Cmd::DeleteBefore(s) => {
                if text.ends_with(&s) {
                    let n = text.len() - s.len();
                    text.truncate(n);
                }
            }
            Cmd::DelKey => {
                text.pop();
            }
            _ => {}
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("sentences file");
    let lang = args.get(2).cloned().unwrap_or_else(|| "ru".into());
    let n_sent: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(300);
    let sigma: f32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.32);
    let spell_rate = 0.06;

    let mut p = Prefs::defaults(&[]);
    p.languages = vec![lang.clone()];
    p.vibrate = false;
    p.learn = false;
    let mut k = Keyboard::new(p);
    k.measure(1080.0, 2.75, 0.0);
    k.start_input(Editor::from_android(0x1, 6), false);
    for c in k.take_commands() {
        if let Cmd::LoadModel(path) = c {
            let file = format!("{}/../app/src/main/assets/{path}", env!("CARGO_MANIFEST_DIR"));
            let b = std::fs::read(file).unwrap();
            k.model_loaded(&path, Arc::new(Model::from_bytes(&b).unwrap()));
        }
    }
    let keys = k.keys.clone();
    let center = |ch: char| keys.iter().find(|key| key.label.to_lowercase() == ch.to_string()).map(|key| key.rect.clone());
    let space = keys.iter().find(|key| key.kind == KeyKind::Space).unwrap().rect.clone();

    let text = std::fs::read_to_string(path).unwrap();
    let mut r = Rng(0x9E3779B97F4A7C15);
    let (mut words, mut clean_ok, mut clean, mut err_fixed, mut err) = (0, 0, 0, 0, 0);
    let mut spell_fixed = 0;
    let mut spell = 0;
    let mut broken: Vec<String> = Vec::new();
    let mut missed: Vec<String> = Vec::new();
    let mut t = 0.0f64;
    for line in text.lines().take(n_sent * 3) {
        let sentence = line.split('\t').last().unwrap_or(line).to_lowercase().replace('ё', "е");
        let truth: Vec<String> = sentence
            .split(|c: char| !c.is_alphabetic() && c != '-')
            .filter(|w| !w.is_empty() && w.chars().all(|c| center(c).is_some()))
            .map(String::from)
            .collect();
        if truth.len() < 4 {
            continue;
        }
        let truth: Vec<String> = truth.into_iter().take(10).collect();
        let mut out = String::new();
        let mut comp = None;
        k.start_input(Editor::from_android(0x1, 6), false);
        if std::env::var("NORULES").is_ok() {
            k.predictor.rules = &[];
        }
        k.set_context("", false);
        let _ = k.take_commands();
        let mut kinds = Vec::new();
        for w in &truth {
            let mut target = w.clone();
            let mut kind = 0; // 0 clean, 1 spelling mistake
            if r.next() < spell_rate {
                if let Some(s) = spelling_error(w, &mut r) {
                    target = s;
                    kind = 1;
                }
            }
            // taps with Gaussian scatter around the key centers
            let mut typed = String::new();
            for ch in target.chars() {
                let rect = center(ch).unwrap();
                let x = rect.cx() + r.gauss() * sigma * rect.w;
                let y = rect.cy() + r.gauss() * sigma * rect.h;
                // what a static grid would read: is the tap itself already wrong?
                let hit = keys.iter().find(|key| key.hit.contains(x, y)).map(|key| key.label.to_lowercase()).unwrap_or_default();
                typed.push_str(&hit);
                k.touch_down(1, x, y, t);
                k.touch_up(1, x, y, t + 30.0);
                t += 150.0;
            }
            let noisy = typed != target;
            kinds.push(if kind == 1 { 2 } else if noisy { 1 } else { 0 });
            k.touch_down(1, space.cx(), space.cy(), t);
            k.touch_up(1, space.cx(), space.cy(), t + 30.0);
            t += 150.0;
            apply(&mut out, &mut comp, k.take_commands());
        }
        let got: Vec<String> = out.to_lowercase().replace('ё', "е").split_whitespace().map(String::from).collect();
        if got.len() != truth.len() {
            continue;
        }
        for ((g, w), kind) in got.iter().zip(&truth).zip(&kinds) {
            words += 1;
            let ok = g == w;
            match kind {
                0 => {
                    clean += 1;
                    if ok {
                        clean_ok += 1
                    } else if broken.len() < 25 {
                        broken.push(format!("{w}→{g}"))
                    }
                }
                1 => {
                    err += 1;
                    if ok {
                        err_fixed += 1
                    } else if missed.len() < 25 {
                        missed.push(format!("{w}:{g}"))
                    }
                }
                _ => {
                    spell += 1;
                    if ok {
                        spell_fixed += 1
                    } else if missed.len() < 40 {
                        missed.push(format!("[{w}]:{g}"))
                    }
                }
            }
        }
        if words > n_sent * 8 {
            break;
        }
    }
    let pct = |a: usize, b: usize| 100.0 * a as f64 / b.max(1) as f64;
    println!("lang {lang} sigma {sigma}: {words} words");
    println!("  clean taps kept right:  {:5.1}%  ({clean_ok}/{clean})", pct(clean_ok, clean));
    println!("  tap errors fixed:       {:5.1}%  ({err_fixed}/{err})", pct(err_fixed, err));
    println!("  spelling errors fixed:  {:5.1}%  ({spell_fixed}/{spell})", pct(spell_fixed, spell));
    println!("  word accuracy:          {:5.1}%", pct(clean_ok + err_fixed + spell_fixed, words));
    println!("  broken: {}", broken.join(" "));
    println!("  missed: {}", missed.join(" "));
}
