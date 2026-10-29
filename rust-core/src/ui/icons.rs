// SPDX-License-Identifier: GPL-3.0-or-later
//! Vector icons drawn with the egui painter (no icon font needed). `s` is the icon size.

use egui::{epaint::PathShape, epaint::PathStroke, pos2, vec2, Color32, Painter, Pos2, Shape, Stroke};

fn stroke(s: f32, c: Color32) -> Stroke {
    Stroke::new((s * 0.085).max(1.2), c)
}

fn path(p: &Painter, pts: Vec<Pos2>, closed: bool, st: Stroke) {
    let shape = if closed {
        PathShape::closed_line(pts, st)
    } else {
        PathShape::line(pts, PathStroke::from(st))
    };
    p.add(Shape::Path(shape));
}

pub fn shift(p: &Painter, c: Pos2, s: f32, col: Color32, filled: bool, caps: bool) {
    let h = s * 0.5;
    let w = s * 0.42;
    let pts = vec![
        pos2(c.x, c.y - h),
        pos2(c.x + w, c.y),
        pos2(c.x + w * 0.45, c.y),
        pos2(c.x + w * 0.45, c.y + h * 0.75),
        pos2(c.x - w * 0.45, c.y + h * 0.75),
        pos2(c.x - w * 0.45, c.y),
        pos2(c.x - w, c.y),
    ];
    if filled {
        p.add(Shape::convex_polygon(pts[..2].iter().chain(&pts[6..]).copied().collect(), col, Stroke::NONE));
        p.rect_filled(egui::Rect::from_min_max(pts[5], pts[3]), 0.0, col);
    } else {
        path(p, pts, true, stroke(s, col));
    }
    if caps {
        p.line_segment([pos2(c.x - w * 0.45, c.y + h * 1.05), pos2(c.x + w * 0.45, c.y + h * 1.05)], stroke(s, col));
    }
}

pub fn backspace(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let w = s * 0.62;
    let h = s * 0.36;
    let pts = vec![
        pos2(c.x - w, c.y),
        pos2(c.x - w * 0.45, c.y - h),
        pos2(c.x + w, c.y - h),
        pos2(c.x + w, c.y + h),
        pos2(c.x - w * 0.45, c.y + h),
    ];
    path(p, pts, true, stroke(s, col));
    let x0 = c.x + w * 0.25;
    let d = h * 0.45;
    p.line_segment([pos2(x0 - d, c.y - d), pos2(x0 + d, c.y + d)], stroke(s, col));
    p.line_segment([pos2(x0 - d, c.y + d), pos2(x0 + d, c.y - d)], stroke(s, col));
}

pub fn enter(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let w = s * 0.5;
    let h = s * 0.32;
    let st = stroke(s, col);
    path(p, vec![pos2(c.x + w, c.y - h), pos2(c.x + w, c.y + h * 0.4), pos2(c.x - w, c.y + h * 0.4)], false, st);
    path(p, vec![pos2(c.x - w + h * 0.6, c.y - h * 0.2), pos2(c.x - w, c.y + h * 0.4), pos2(c.x - w + h * 0.6, c.y + h)], false, st);
}

pub fn arrow_right(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let w = s * 0.45;
    let st = stroke(s, col);
    p.line_segment([pos2(c.x - w, c.y), pos2(c.x + w, c.y)], st);
    path(p, vec![pos2(c.x + w * 0.3, c.y - w * 0.6), pos2(c.x + w, c.y), pos2(c.x + w * 0.3, c.y + w * 0.6)], false, st);
}

pub fn arrow_left(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let w = s * 0.45;
    let st = stroke(s, col);
    p.line_segment([pos2(c.x - w, c.y), pos2(c.x + w, c.y)], st);
    path(p, vec![pos2(c.x - w * 0.3, c.y - w * 0.6), pos2(c.x - w, c.y), pos2(c.x - w * 0.3, c.y + w * 0.6)], false, st);
}

pub fn check(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let w = s * 0.42;
    path(p, vec![pos2(c.x - w, c.y), pos2(c.x - w * 0.25, c.y + w * 0.7), pos2(c.x + w, c.y - w * 0.65)], false, stroke(s, col));
}

pub fn search(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let r = s * 0.27;
    let o = pos2(c.x - s * 0.08, c.y - s * 0.08);
    p.circle_stroke(o, r, stroke(s, col));
    p.line_segment([o + vec2(r * 0.72, r * 0.72), o + vec2(r * 1.7, r * 1.7)], stroke(s, col));
}

pub fn send(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let w = s * 0.45;
    let pts = vec![pos2(c.x - w, c.y - w * 0.85), pos2(c.x + w, c.y), pos2(c.x - w, c.y + w * 0.85), pos2(c.x - w * 0.55, c.y)];
    path(p, pts, true, stroke(s, col));
}

pub fn globe(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let r = s * 0.4;
    let st = stroke(s * 0.85, col);
    p.circle_stroke(c, r, st);
    p.line_segment([pos2(c.x - r, c.y), pos2(c.x + r, c.y)], st);
    let pts: Vec<Pos2> = (0..=16).map(|i| {
        let a = std::f32::consts::PI * (i as f32 / 16.0 - 0.5);
        pos2(c.x + r * 0.45 * a.cos(), c.y + r * a.sin())
    }).collect();
    path(p, pts.clone(), false, st);
    path(p, pts.iter().map(|q| pos2(2.0 * c.x - q.x, q.y)).collect(), false, st);
}

pub fn smile(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let r = s * 0.4;
    let st = stroke(s * 0.85, col);
    p.circle_stroke(c, r, st);
    p.circle_filled(pos2(c.x - r * 0.35, c.y - r * 0.25), r * 0.11, col);
    p.circle_filled(pos2(c.x + r * 0.35, c.y - r * 0.25), r * 0.11, col);
    let pts: Vec<Pos2> = (0..=10).map(|i| {
        let a = std::f32::consts::PI * (0.15 + 0.7 * i as f32 / 10.0);
        pos2(c.x + r * 0.55 * a.cos(), c.y + r * 0.1 + r * 0.45 * a.sin())
    }).collect();
    path(p, pts, false, st);
}

pub fn gear(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let st = stroke(s * 0.85, col);
    let r = s * 0.27;
    p.circle_stroke(c, r * 0.45, st);
    let pts: Vec<Pos2> = (0..48).map(|i| {
        let a = std::f32::consts::TAU * i as f32 / 48.0;
        let tooth = if (i / 3) % 2 == 0 { 1.25 } else { 1.0 };
        pos2(c.x + r * tooth * a.cos(), c.y + r * tooth * a.sin())
    }).collect();
    path(p, pts, true, st);
}

pub fn clipboard(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let st = stroke(s * 0.85, col);
    let r = egui::Rect::from_center_size(c + vec2(0.0, s * 0.04), vec2(s * 0.55, s * 0.7));
    p.rect_stroke(r, s * 0.06, st, egui::StrokeKind::Middle);
    p.rect_filled(egui::Rect::from_center_size(pos2(c.x, r.top()), vec2(s * 0.28, s * 0.12)), s * 0.03, col);
    for i in 0..3 {
        let y = r.top() + s * (0.2 + 0.14 * i as f32);
        p.line_segment([pos2(r.left() + s * 0.1, y), pos2(r.right() - s * 0.1, y)], Stroke::new(st.width * 0.7, col));
    }
}

pub fn keyboard(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let st = stroke(s * 0.8, col);
    let r = egui::Rect::from_center_size(c, vec2(s * 0.8, s * 0.52));
    p.rect_stroke(r, s * 0.06, st, egui::StrokeKind::Middle);
    for row in 0..2 {
        for i in 0..4 {
            let x = r.left() + r.width() * (0.2 + 0.2 * i as f32);
            let y = r.top() + r.height() * (0.3 + 0.22 * row as f32);
            p.circle_filled(pos2(x, y), s * 0.03, col);
        }
    }
    p.line_segment([pos2(r.left() + r.width() * 0.3, r.bottom() - r.height() * 0.2), pos2(r.right() - r.width() * 0.3, r.bottom() - r.height() * 0.2)], Stroke::new(st.width * 0.8, col));
}

pub fn chevron_down(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let w = s * 0.32;
    path(p, vec![pos2(c.x - w, c.y - w * 0.4), pos2(c.x, c.y + w * 0.45), pos2(c.x + w, c.y - w * 0.4)], false, stroke(s, col));
}
