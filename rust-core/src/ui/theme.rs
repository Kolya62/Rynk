// SPDX-License-Identifier: GPL-3.0-or-later
//! Colors close to Yandex Keyboard's light and dark themes.

use egui::Color32;

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub key: Color32,
    pub key_func: Color32,
    pub key_pressed: Color32,
    pub key_shadow: Color32,
    pub text: Color32,
    pub text_func: Color32,
    pub hint: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub strip_text: Color32,
    pub strip_primary: Color32,
    pub divider: Color32,
    pub popup: Color32,
    pub popup_sel: Color32,
    pub popup_sel_text: Color32,
}

/// Yandex yellow
const YELLOW: Color32 = Color32::from_rgb(0xFF, 0xDB, 0x4D);

pub const LIGHT: Palette = Palette {
    dark: false,
    bg: Color32::from_rgb(0xE6, 0xE8, 0xEC),
    key: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    key_func: Color32::from_rgb(0xC8, 0xCD, 0xD5),
    key_pressed: Color32::from_rgb(0xB4, 0xBA, 0xC4),
    key_shadow: Color32::from_rgb(0xA7, 0xAD, 0xB7),
    text: Color32::from_rgb(0x11, 0x11, 0x11),
    text_func: Color32::from_rgb(0x2B, 0x2E, 0x33),
    hint: Color32::from_rgb(0x8C, 0x92, 0x9C),
    accent: YELLOW,
    accent_text: Color32::from_rgb(0x15, 0x15, 0x15),
    strip_text: Color32::from_rgb(0x55, 0x5A, 0x63),
    strip_primary: Color32::from_rgb(0x0A, 0x0A, 0x0A),
    divider: Color32::from_rgb(0xC3, 0xC8, 0xD0),
    popup: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    popup_sel: YELLOW,
    popup_sel_text: Color32::from_rgb(0x11, 0x11, 0x11),
};

pub const DARK: Palette = Palette {
    dark: true,
    bg: Color32::from_rgb(0x16, 0x16, 0x18),
    key: Color32::from_rgb(0x3B, 0x3B, 0x3F),
    key_func: Color32::from_rgb(0x28, 0x28, 0x2B),
    key_pressed: Color32::from_rgb(0x55, 0x55, 0x5B),
    key_shadow: Color32::from_rgb(0x0B, 0x0B, 0x0C),
    text: Color32::from_rgb(0xF4, 0xF4, 0xF6),
    text_func: Color32::from_rgb(0xDD, 0xDD, 0xE2),
    hint: Color32::from_rgb(0x8E, 0x8E, 0x96),
    accent: YELLOW,
    accent_text: Color32::from_rgb(0x15, 0x15, 0x15),
    strip_text: Color32::from_rgb(0xA8, 0xA8, 0xB0),
    strip_primary: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    divider: Color32::from_rgb(0x34, 0x34, 0x38),
    popup: Color32::from_rgb(0x4A, 0x4A, 0x50),
    popup_sel: YELLOW,
    popup_sel_text: Color32::from_rgb(0x11, 0x11, 0x11),
};
