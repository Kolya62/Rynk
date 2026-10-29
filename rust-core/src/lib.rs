// SPDX-License-Identifier: GPL-3.0-or-later
pub mod engine;
pub mod layout;
pub mod compose;
pub mod emoji;
pub mod prefs;
pub mod keyboard;
pub mod ui;
pub mod gfx;
#[cfg(target_os = "android")]
mod android;
