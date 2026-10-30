// SPDX-License-Identifier: GPL-3.0-or-later
//! Paints the keyboard. Layout coordinates are pixels; egui works in points (px / density).

use super::icons;
use super::theme::Palette;
use crate::emoji;
use crate::keyboard::{EnterAction, Keyboard, PopupKind, StripItem, Tool};
use crate::layout::{KeyKind, Mode, ShiftState};
use egui::{pos2, vec2, Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Stroke};

struct P<'a> {
    p: &'a Painter,
    dp: f32,
    pal: Palette,
}

impl P<'_> {
    fn r(&self, r: &crate::layout::Rect) -> Rect {
        Rect::from_min_size(pos2(r.x / self.dp, r.y / self.dp), vec2(r.w / self.dp, r.h / self.dp))
    }

    /// Text that fits a width: shrinks the font until it does
    fn text_fit(&self, c: Pos2, s: &str, size: f32, max_w: f32, col: Color32) {
        let s = &super::textvis::visual(s);
        let mut size = size;
        for _ in 0..6 {
            let g = self.p.layout_no_wrap(s.to_string(), FontId::proportional(size), col);
            if g.size().x <= max_w || size < 9.0 {
                self.p.galley(c - g.size() / 2.0, g, col);
                return;
            }
            size *= 0.85;
        }
    }

    /// Bold look without a bold font: the text drawn twice, a hair apart
    fn text_bold(&self, c: Pos2, s: &str, size: f32, max_w: f32, col: Color32) {
        self.text_fit(c, s, size, max_w, col);
        self.text_fit(c + vec2(0.55, 0.0), s, size, max_w, col);
    }

    fn key_bg(&self, r: Rect, fill: Color32) {
        let radius = CornerRadius::same(7);
        self.p.rect_filled(r.translate(vec2(0.0, 1.0)), radius, self.pal.key_shadow);
        self.p.rect_filled(r, radius, fill);
    }
}

pub fn paint(kb: &Keyboard, painter: &Painter, pal: Palette) {
    let dp = kb.dp;
    let g = P { p: painter, dp, pal };
    let full = Rect::from_min_size(Pos2::ZERO, vec2(kb.width / dp, kb.height / dp));
    painter.rect_filled(full, 0.0, pal.bg);
    paint_strip(kb, &g);
    if kb.mode == Mode::Emoji {
        paint_emoji(kb, &g);
    } else if kb.mode == Mode::Candidates {
        paint_cand_panel(kb, &g);
    } else {
        paint_keys(kb, &g);
    }
    paint_overlays(kb, &g);
    paint_trail(kb, &g);
}

/// The path of a gesture being drawn: thin at the old end, full width at the finger
fn paint_trail(kb: &Keyboard, g: &P) {
    let pts: Vec<Pos2> = kb.trail.iter().map(|&(x, y)| pos2(x / g.dp, y / g.dp)).collect();
    let n = pts.len();
    if n < 2 {
        return;
    }
    for (i, w) in pts.windows(2).enumerate() {
        let t = (i + 1) as f32 / (n - 1) as f32;
        let color = g.pal.accent.gamma_multiply(0.35 + 0.65 * t);
        g.p.line_segment([w[0], w[1]], egui::Stroke::new(1.5 + 3.5 * t, color));
        g.p.circle_filled(w[1], (1.5 + 3.5 * t) / 2.0, color);
    }
}

fn paint_strip(kb: &Keyboard, g: &P) {
    let cells = kb.strip_layout();
    let h = kb.strip_h / g.dp;
    let words = cells.iter().any(|c| matches!(c.0, StripItem::Word { .. } | StripItem::Cand { .. }));
    let n = cells.len();
    for (i, (item, x0, x1)) in cells.iter().enumerate() {
        let (x0, x1) = (x0 / g.dp, x1 / g.dp);
        let cell = x1 - x0;
        let c = pos2((x0 + x1) / 2.0, h / 2.0);
        match item {
            StripItem::Word { text, primary, original } => {
                let col = if *primary { g.pal.strip_primary } else { g.pal.strip_text };
                if *primary {
                    g.text_bold(c, text, 18.0, cell - 12.0, col);
                } else if *original {
                    g.text_fit(c, text, 16.0, cell - 12.0, g.pal.hint);
                } else {
                    g.text_fit(c, text, 17.0, cell - 12.0, col);
                }
            }
            StripItem::Cand { text, primary, .. } => {
                if *primary {
                    g.text_bold(c, text, 19.0, cell - 8.0, g.pal.strip_primary);
                } else {
                    g.text_fit(c, text, 19.0, cell - 8.0, g.pal.strip_text);
                }
            }
            StripItem::Expand(open) => {
                let col = g.pal.strip_text;
                if *open {
                    icons::chevron_up(g.p, c, 24.0, col);
                } else {
                    icons::chevron_down(g.p, c, 24.0, col);
                }
            }
            StripItem::Query(q) => {
                let r = Rect::from_min_max(pos2(x0 + 6.0, h * 0.16), pos2(x1 - 6.0, h * 0.84));
                g.p.rect_filled(r, CornerRadius::same(8), g.pal.key);
                let s = r.height() * 0.5;
                icons::search(g.p, pos2(r.left() + s, c.y), s, g.pal.hint);
                let text_c = pos2((r.left() + 2.0 * s + r.right()) / 2.0, c.y);
                let max_w = r.width() - 2.5 * s;
                if q.is_empty() {
                    g.text_fit(text_c, "…", 16.0, max_w, g.pal.hint);
                } else {
                    g.text_fit(text_c, &format!("{q}|"), 16.0, max_w, g.pal.strip_text);
                }
            }
            StripItem::Emoji(e) => {
                emoji_image(kb, g.p, e, c, (h * 0.6).min(cell - 4.0));
            }
            StripItem::Tool(t) => {
                let col = g.pal.strip_text;
                let s = 24.0;
                match t {
                    Tool::Settings => icons::gear(g.p, c, s, col),
                    Tool::Emoji => icons::smile(g.p, c, s, col),
                    Tool::Paste => icons::clipboard(g.p, c, s, col),
                    Tool::SwitchIme => icons::keyboard(g.p, c, s, col),
                    Tool::Hide => icons::chevron_down(g.p, c, s, col),
                }
            }
        }
        if words && i + 1 < n {
            g.p.line_segment([pos2(x1, h * 0.28), pos2(x1, h * 0.72)], Stroke::new(1.0, g.pal.divider));
        }
    }
}

fn paint_cand_panel(kb: &Keyboard, g: &P) {
    let top = kb.strip_h / g.dp;
    let bottom = (kb.height - kb.bottom_inset) / g.dp;
    let clip = Rect::from_min_max(pos2(0.0, top), pos2(kb.width / g.dp, bottom));
    let p = g.p.with_clip_rect(clip);
    for (i, r) in kb.cand_panel() {
        let r = g.r(&r);
        if r.bottom() < top || r.top() > bottom {
            continue;
        }
        p.rect_stroke(r, 0.0, Stroke::new(0.5, g.pal.divider), egui::StrokeKind::Inside);
        let text = &kb.cands[i].text;
        let gal = p.layout_no_wrap(text.clone(), FontId::proportional(20.0), g.pal.text);
        p.galley(r.center() - gal.size() / 2.0, gal, g.pal.text);
    }
}

fn paint_keys(kb: &Keyboard, g: &P) {
    let lang = kb.lang();
    for (i, key) in kb.keys.iter().enumerate() {
        let r = g.r(&key.rect);
        let pressed = kb.pressed.contains(&i);
        let is_enter = key.kind == KeyKind::Enter;
        let shift_lit = key.kind == KeyKind::Shift && kb.shift != ShiftState::Off;
        let fill = if is_enter {
            if pressed { g.pal.accent.gamma_multiply(0.8) } else { g.pal.accent }
        } else if pressed {
            g.pal.key_pressed
        } else if shift_lit {
            g.pal.key
        } else if key.func {
            g.pal.key_func
        } else {
            g.pal.key
        };
        g.key_bg(r, fill);
        let c = r.center();
        let icon = (r.height() * 0.5).min(26.0);
        let fg = if is_enter { g.pal.accent_text } else if key.func { g.pal.text_func } else { g.pal.text };
        match &key.kind {
            KeyKind::Char(_) => {
                let size = if key.letter { (r.height() * 0.72).min(25.0) } else if key.func { 22.0 } else { (r.height() * 0.44).min(23.0) };
                g.text_fit(c + vec2(0.0, 1.0), &visible(&key.label), size, r.width() - 4.0, fg);
                if let Some(h) = &key.hint {
                    g.p.text(r.right_top() + vec2(-4.0, 2.0), Align2::RIGHT_TOP, h, FontId::proportional(11.0), g.pal.hint);
                }
            }
            KeyKind::Shift => icons::shift(g.p, c, icon, fg, kb.shift != ShiftState::Off, kb.shift == ShiftState::Caps),
            KeyKind::Backspace => icons::backspace(g.p, c, icon, fg),
            KeyKind::Enter => match kb.editor.action {
                EnterAction::Search => icons::search(g.p, c, icon, fg),
                EnterAction::Send => icons::send(g.p, c, icon, fg),
                EnterAction::Go | EnterAction::Next => icons::arrow_right(g.p, c, icon, fg),
                EnterAction::Previous => icons::arrow_left(g.p, c, icon, fg),
                EnterAction::Done => icons::check(g.p, c, icon, fg),
                EnterAction::Newline => icons::enter(g.p, c, icon, fg),
            },
            KeyKind::Space => {
                let label = if kb.prefs.languages.len() > 1 { lang.name } else { "" };
                g.text_fit(c, label, 14.0, r.width() - 16.0, g.pal.hint);
            }
            KeyKind::Language => icons::globe(g.p, c, icon, fg),
            KeyKind::Emoji => icons::smile(g.p, c, icon, fg),
            KeyKind::Symbols | KeyKind::SymbolsMore | KeyKind::Alphabet => {
                g.text_fit(c, &key.label, 16.0, r.width() - 6.0, fg);
            }
        }
    }
}

fn paint_emoji(kb: &Keyboard, g: &P) {
    let (top, bottom) = kb.emoji_area();
    let (top, bottom) = (top / g.dp, bottom / g.dp);
    let cell = kb.emoji_cell() / g.dp;
    let scroll = kb.emoji_scroll / g.dp;
    let clip = Rect::from_min_max(pos2(0.0, top), pos2(kb.width / g.dp, bottom));
    let p = g.p.with_clip_rect(clip);
    for (i, e) in emoji::items(kb.emoji_cat).iter().enumerate() {
        let x = (i % 8) as f32 * cell + cell / 2.0;
        let y = top + (i / 8) as f32 * cell + cell / 2.0 - scroll;
        if y < top - cell || y > bottom + cell {
            continue;
        }
        // Android's emoji font is a color bitmap font egui cannot draw: the platform draws
        // the images, and the cell stays empty for the moment until they arrive
        emoji_image(kb, &p, e, pos2(x, y), cell * 0.62);
    }
    // tabs: [ABC] [categories] [⌫]
    let n = kb.emoji_tab_count();
    let w = kb.width / g.dp / n as f32;
    let h = (kb.height - kb.bottom_inset) / g.dp - bottom;
    for i in 0..n {
        let r = Rect::from_min_size(pos2(i as f32 * w, bottom), vec2(w, h)).shrink(3.0);
        let c = r.center();
        if i == 0 {
            g.key_bg(r, g.pal.key_func);
            g.text_fit(c, "ABC", 15.0, r.width(), g.pal.text_func);
        } else if i == n - 1 {
            g.key_bg(r, g.pal.key_func);
            icons::backspace(g.p, c, 22.0, g.pal.text_func);
        } else if i == 1 {
            icons::search(g.p, c, 22.0, g.pal.text_func);
        } else {
            if i - 2 == kb.emoji_cat {
                g.p.rect_filled(r, CornerRadius::same(7), g.pal.key);
            }
            let icon = emoji::CATEGORIES[i - 2].0;
            emoji_image(kb, g.p, icon, c, 26.0);
        }
    }
}

/// Draws a color emoji image the platform made; false when there is none yet
fn emoji_image(kb: &Keyboard, p: &Painter, e: &str, c: Pos2, size: f32) -> bool {
    let Some(img) = kb.emoji_images.get(e) else { return false };
    // Textures live in the egui context they were made in, and every new surface brings a new
    // context: keep them in the context itself so a fresh one makes its own
    let id = egui::Id::NULL.with("emoji-textures");
    let ctx = p.ctx();
    let tex = ctx.data_mut(|d| d.get_temp::<std::collections::HashMap<String, egui::TextureHandle>>(id).and_then(|m| m.get(e).cloned()));
    let tex = tex.unwrap_or_else(|| {
        let ci = egui::ColorImage::from_rgba_premultiplied(img.size, &img.rgba);
        let t = ctx.load_texture(format!("emoji:{e}"), ci, egui::TextureOptions::LINEAR);
        ctx.data_mut(|d| {
            d.get_temp_mut_or_default::<std::collections::HashMap<String, egui::TextureHandle>>(id).insert(e.to_string(), t.clone());
        });
        t
    });
    {
        let aspect = img.size[0] as f32 / img.size[1].max(1) as f32;
        let r = Rect::from_center_size(c, vec2(size * aspect, size));
        p.image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    true
}

fn paint_overlays(kb: &Keyboard, g: &P) {
    if let Some(pop) = &kb.popup {
        let rects: Vec<Rect> = pop.cells.iter().map(|c| g.r(c)).collect();
        if let Some(bound) = rects.iter().copied().reduce(|a, b| a.union(b)) {
            let b = bound.expand(4.0);
            g.p.rect_filled(b.translate(vec2(0.0, 2.0)), CornerRadius::same(10), Color32::from_black_alpha(60));
            g.p.rect_filled(b, CornerRadius::same(10), g.pal.popup);
        }
        for (i, (r, item)) in rects.iter().zip(&pop.items).enumerate() {
            let sel = i == pop.sel;
            if sel {
                g.p.rect_filled(r.shrink(2.0), CornerRadius::same(7), g.pal.popup_sel);
            }
            let col = if sel { g.pal.popup_sel_text } else { g.pal.text };
            let size = if pop.kind == PopupKind::Languages { 17.0 } else { 23.0 };
            g.text_fit(r.center(), &visible(item), size, r.width() - 4.0, col);
        }
        return;
    }
    if !kb.prefs.key_preview {
        return;
    }
    // magnified preview above a pressed character key
    for &i in &kb.pressed {
        let Some(key) = kb.keys.get(i) else { continue };
        if !matches!(key.kind, KeyKind::Char(_)) || key.func {
            continue;
        }
        let r = g.r(&key.rect);
        let w = (r.width() * 1.35).max(44.0);
        let h = r.height() * 1.1;
        let x = (r.center().x - w / 2.0).clamp(2.0, kb.width / g.dp - w - 2.0);
        let bubble = Rect::from_min_size(pos2(x, (r.top() - h - 6.0).max(0.0)), vec2(w, h));
        g.p.rect_filled(bubble.translate(vec2(0.0, 2.0)), CornerRadius::same(10), Color32::from_black_alpha(50));
        g.p.rect_filled(bubble, CornerRadius::same(10), g.pal.popup);
        g.text_fit(bubble.center(), &visible(&key.label), 32.0, w - 6.0, g.pal.text);
    }
}

/// A lone combining mark is shown on a dotted circle, as Indic keyboards do
fn visible(label: &str) -> String {
    match label.chars().next() {
        Some(c) if crate::keyboard::is_mark(c) || c == '\u{0CBC}' => format!("\u{25CC}{label}"),
        _ => label.to_string(),
    }
}
