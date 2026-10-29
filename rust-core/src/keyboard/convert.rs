// SPDX-License-Identifier: GPL-3.0-or-later
//! Reading conversion in the keyboard: pinyin, zhuyin, jyutping and romaji → kana → kanji.
//!
//! The raw input (letters or bopomofo) stays in the composing region; the strip shows
//! candidates. Chinese: space commits the best candidate. Japanese: space walks through the
//! candidates and Enter confirms, as Japanese users expect.

use super::{Cmd, Keyboard, StripItem};
use crate::compose::{kana, zhuyin};
use crate::engine::cjk::Conv;
use crate::layout::{self, Composer, Mode};

/// Full-width punctuation for Chinese and Japanese text
fn full_width(lang_code: &str, s: &str) -> String {
    let ja = lang_code == "ja";
    match s {
        "," | "，" if ja => "、".into(),
        "," => "，".into(),
        "." => "。".into(),
        "?" => "？".into(),
        "!" => "！".into(),
        ":" => "：".into(),
        ";" => "；".into(),
        "(" => "（".into(),
        ")" => "）".into(),
        "~" => "～".into(),
        _ => s.to_string(),
    }
}

impl Keyboard {
    /// The composer in effect: conversion is off in password, number, e-mail and URL fields
    pub fn composer(&self) -> Composer {
        let c = self.lang().composer;
        let e = &self.editor;
        if c.converts() && (e.password || e.numeric() || e.email || e.url) {
            Composer::None
        } else {
            c
        }
    }

    pub(super) fn cjk_active(&self) -> bool {
        self.composer().converts() && !self.cjk_input.is_empty()
    }

    /// The dictionary reading of the raw input
    fn reading(&self) -> String {
        match self.composer() {
            Composer::Kana => kana::to_hiragana(&self.cjk_input, true),
            Composer::Zhuyin => zhuyin::syllables(&self.cjk_input).into_iter().map(|s| s.0).collect(),
            _ => self.cjk_input.to_lowercase(),
        }
    }

    /// Raw input characters that make up the first `consumed` reading characters
    fn raw_len(&self, consumed: usize) -> usize {
        let raw: Vec<char> = self.cjk_input.chars().collect();
        match self.composer() {
            // the shortest romaji prefix that has fully become those kana
            Composer::Kana => (1..=raw.len())
                .find(|&k| {
                    let h = kana::to_hiragana(&raw[..k].iter().collect::<String>(), true);
                    !h.chars().any(|c| c.is_ascii_alphabetic()) && h.chars().count() >= consumed
                })
                .unwrap_or(raw.len()),
            Composer::Zhuyin => {
                let (mut got, mut len) = (0, 0);
                for (py, n) in zhuyin::syllables(&self.cjk_input) {
                    if got >= consumed {
                        break;
                    }
                    got += py.chars().count();
                    len += n;
                }
                // trailing tone marks belong to the consumed syllable
                len.min(raw.len())
            }
            _ => consumed.min(raw.len()),
        }
    }

    /// What the composing region shows
    pub(super) fn cjk_display(&self) -> String {
        if self.composer() == Composer::Kana {
            if let Some(c) = self.conv_sel.and_then(|i| self.cands.get(i)) {
                let rest: String = self.reading().chars().skip(c.consumed).collect();
                return format!("{}{rest}", c.text);
            }
            return kana::to_hiragana(&self.cjk_input, false);
        }
        self.cjk_input.clone()
    }

    pub(super) fn refresh_cands(&mut self) {
        self.cands.clear();
        self.cand_scroll = 0.0;
        if self.cjk_input.is_empty() {
            return;
        }
        let reading = self.reading();
        let conv = self.lang().conv;
        let is_kana = self.composer() == Composer::Kana;
        let mut cands = match self.convs.get(&format!("cjk/{conv}.rcd")) {
            Some(d) => d.convert(&reading, &self.predictor.user, is_kana, 60),
            None => Vec::new(),
        };
        if is_kana {
            // the kana themselves are always among the first choices
            let n = reading.chars().count();
            let hira = Conv { text: reading.clone(), consumed: n, score: 0.0 };
            let kata = Conv { text: kana::to_katakana(&reading), consumed: n, score: 0.0 };
            cands.retain(|c| !(c.consumed == n && (c.text == hira.text || c.text == kata.text)));
            let at = cands.len().min(1);
            cands.insert(at, hira);
            cands.insert((at + 2).min(cands.len()), kata);
        }
        self.cands = cands;
        self.dirty = true;
    }

    fn show(&mut self) {
        let shown = self.cjk_display();
        self.emit(Cmd::Composing(shown));
        self.dirty = true;
    }

    /// Commits candidate `i`; the rest of the input stays to be converted
    pub(super) fn cjk_pick(&mut self, i: usize) {
        let Some(c) = self.cands.get(i).cloned() else { return };
        let drop = self.raw_len(c.consumed);
        self.commit(&c.text);
        if self.prefs.learn && !self.editor.password {
            self.predictor.user.add_word(&c.text, 1);
        }
        self.cjk_input = self.cjk_input.chars().skip(drop).collect();
        self.conv_sel = None;
        if self.mode == Mode::Candidates {
            self.mode = Mode::Alphabet;
            self.rebuild();
        }
        self.refresh_cands();
        if !self.cjk_input.is_empty() {
            self.show();
        }
        self.dirty = true;
    }

    /// Commits whatever the input converts to best (all of it)
    pub(super) fn cjk_commit_all(&mut self) {
        let mut guard = 0;
        while !self.cjk_input.is_empty() && guard < 64 {
            guard += 1;
            let i = self.conv_sel.unwrap_or(0);
            if self.cands.is_empty() {
                let raw = if self.composer() == Composer::Kana { kana::to_hiragana(&self.cjk_input, true) } else { self.cjk_input.clone() };
                self.commit(&raw);
                self.cjk_input.clear();
                break;
            }
            self.cjk_pick(i.min(self.cands.len() - 1));
        }
        self.conv_sel = None;
        self.cands.clear();
    }

    /// A key typed while a converting composer is active; returns true when consumed
    pub(super) fn cjk_type(&mut self, text: &str) -> bool {
        let composer = self.composer();
        let is_input = match composer {
            Composer::Zhuyin => text.chars().all(zhuyin::is_zhuyin),
            Composer::Kana => text.chars().all(|c| c.is_ascii_alphabetic() || c == '-'),
            _ => text.chars().all(|c| c.is_ascii_alphabetic() || c == '\''),
        } && !text.is_empty();
        if is_input && !(text == "-" && self.cjk_input.is_empty()) {
            if self.conv_sel.is_some() {
                // typing on after choosing a conversion confirms it
                let i = self.conv_sel.unwrap_or(0);
                self.cjk_pick(i);
                self.cjk_commit_all();
            }
            self.cjk_input.push_str(&text.to_lowercase());
            self.refresh_cands();
            self.show();
            return true;
        }
        // anything else ends the input: convert it, then the punctuation in full width
        self.cjk_commit_all();
        let code = self.lang().code;
        let out = full_width(code, text);
        self.commit(&out);
        self.after_separator(text);
        true
    }

    pub(super) fn cjk_space(&mut self) {
        if self.composer() == Composer::Kana {
            // Japanese: space converts and then walks through the candidates
            if self.cands.is_empty() {
                self.cjk_commit_all();
                return;
            }
            let next = self.conv_sel.map_or(0, |i| (i + 1) % self.cands.len());
            self.conv_sel = Some(next);
            self.show();
        } else {
            self.cjk_pick(0);
            if self.cands.is_empty() && !self.cjk_input.is_empty() {
                self.cjk_commit_all();
            }
        }
    }

    pub(super) fn cjk_enter(&mut self) {
        match self.conv_sel {
            Some(i) => {
                self.cjk_pick(i);
                self.cjk_commit_all();
            }
            None => {
                // Enter keeps the reading as typed: letters, or kana for Japanese
                let raw = if self.composer() == Composer::Kana { kana::to_hiragana(&self.cjk_input, true) } else { self.cjk_input.clone() };
                self.commit(&raw);
                self.cjk_input.clear();
                self.cands.clear();
            }
        }
        if self.mode == Mode::Candidates {
            self.mode = Mode::Alphabet;
            self.rebuild();
        }
        self.dirty = true;
    }

    pub(super) fn cjk_backspace(&mut self) {
        if self.conv_sel.is_some() {
            self.conv_sel = None;
        } else {
            self.cjk_input.pop();
            self.refresh_cands();
        }
        if self.cjk_input.is_empty() && self.mode == Mode::Candidates {
            self.mode = Mode::Alphabet;
            self.rebuild();
        }
        self.show();
    }

    /// Estimated width (px) of a candidate cell
    fn cand_width(&self, text: &str) -> f32 {
        let units: f32 = text.chars().map(|c| if c.is_ascii() { 0.55 } else { 1.0 }).sum();
        (units * 19.0 + 26.0).max(46.0) * self.dp
    }

    /// Candidate cells of the strip: (item, x0, x1) in px
    pub(super) fn cand_strip(&self) -> Vec<(StripItem, f32, f32)> {
        let expand_w = 44.0 * self.dp;
        let avail = self.width - expand_w;
        let mut out = Vec::new();
        let mut x = 0.0;
        for (i, c) in self.cands.iter().enumerate() {
            let w = self.cand_width(&c.text);
            if x + w > avail && i > 0 {
                break;
            }
            let primary = self.conv_sel.map_or(i == 0, |s| s == i);
            out.push((StripItem::Cand { text: c.text.clone(), idx: i, primary }, x, x + w));
            x += w;
        }
        if out.len() < self.cands.len() || self.mode == Mode::Candidates {
            out.push((StripItem::Expand(self.mode == Mode::Candidates), self.width - expand_w, self.width));
        }
        out
    }

    /// Cells of the open candidate panel: (candidate index, rect in px), scrolled
    pub fn cand_panel(&self) -> Vec<(usize, layout::Rect)> {
        let top = self.strip_h;
        let row_h = 50.0 * self.dp;
        let mut out = Vec::new();
        let (mut x, mut y) = (0.0f32, top - self.cand_scroll);
        for (i, c) in self.cands.iter().enumerate() {
            let w = self.cand_width(&c.text).max(self.width / 5.0).min(self.width);
            if x + w > self.width {
                x = 0.0;
                y += row_h;
            }
            out.push((i, layout::Rect { x, y, w, h: row_h }));
            x += w;
        }
        out
    }

    pub(super) fn toggle_cand_panel(&mut self) {
        self.mode = if self.mode == Mode::Candidates { Mode::Alphabet } else { Mode::Candidates };
        self.cand_scroll = 0.0;
        self.rebuild();
    }
}
