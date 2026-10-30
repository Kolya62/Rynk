// SPDX-License-Identifier: GPL-3.0-or-later
//! Keyboard state machine: touches in, editor commands out. Platform independent.

mod convert;

use crate::compose::hangul::Hangul;
use crate::emoji;
use crate::engine::lm::Context;
use crate::engine::touch::{KeyGeom, Tap, TouchModel};
use crate::engine::cjk::{Conv, ConvDict};
use crate::engine::userdict::UserDict;
use crate::engine::{Model, Predictor, Suggestions};
use crate::layout::{self, BuildOptions, Composer, Frame, Key, KeyKind, LangDef, Mode, ShiftState};
use crate::prefs::Prefs;
use std::collections::HashMap;
use std::sync::Arc;

const LONG_PRESS_MS: f64 = 380.0;
/// Largest share of the screen height the keyboard (without the navigation bar) may take
const MAX_SCREEN_SHARE: f32 = 0.5;
/// Space-bar swipe distance that switches the language (dp)
const LANG_SWIPE_DP: f32 = 40.0;
const REPEAT_START_MS: f64 = 420.0;
const REPEAT_MS: f64 = 55.0;
const DOUBLE_TAP_MS: f64 = 350.0;
const DOUBLE_SPACE_MS: f64 = 1200.0;
/// Space-bar drag distance per cursor step (dp)
const CURSOR_STEP_DP: f32 = 11.0;
/// Backspace drag distance per deleted word (dp)
const WORD_STEP_DP: f32 = 45.0;
const SENTENCE_END: &[char] = &['.', '!', '?', '…', '。', '।', '።', '؟', '¡', '¿'];
const SMART_PUNCT: &[&str] = &[",", ".", "!", "?", ":", ";", "…", "،", "؟"];

/// Commands for the editor (applied by the platform layer)
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    /// Replace the composing region with this text
    Composing(String),
    /// Commit text (replaces the composing region if any)
    Commit(String),
    /// Delete exactly this text before the cursor
    DeleteBefore(String),
    /// A hardware-like DEL (deletes the selection or one character)
    DelKey,
    /// Perform the editor action (Search, Send, Go…)
    EditorAction,
    /// Move the cursor by characters (negative = left)
    Cursor(i32),
    Haptic,
    Paste,
    OpenSettings,
    SwitchIme,
    Hide,
    /// The keyboard wants another height
    Relayout,
    /// Load the language model of this asset code (in the background)
    LoadModel(String),
    /// Draw these emoji (space-separated, after the size in px) with the system color font
    RenderEmoji(String),
}

/// An emoji drawn by the platform: premultiplied RGBA
#[derive(Clone, Debug)]
pub struct EmojiImage {
    pub size: [usize; 2],
    pub rgba: Arc<Vec<u8>>,
}

impl Cmd {
    pub fn encode(cmds: &[Cmd]) -> String {
        let mut s = String::new();
        for c in cmds {
            let (op, arg) = match c {
                Cmd::Composing(t) => ("C", t.as_str()),
                Cmd::Commit(t) => ("T", t.as_str()),
                Cmd::DeleteBefore(t) => ("D", t.as_str()),
                Cmd::DelKey => ("K", ""),
                Cmd::EditorAction => ("A", ""),
                Cmd::Cursor(n) => {
                    s.push_str(&format!("M\u{1F}{n}\u{1E}"));
                    continue;
                }
                Cmd::Haptic => ("H", ""),
                Cmd::Paste => ("P", ""),
                Cmd::OpenSettings => ("S", ""),
                Cmd::SwitchIme => ("I", ""),
                Cmd::Hide => ("X", ""),
                Cmd::Relayout => ("R", ""),
                Cmd::LoadModel(c) => ("L", c.as_str()),
                Cmd::RenderEmoji(c) => ("E", c.as_str()),
            };
            s.push_str(op);
            s.push('\u{1F}');
            s.push_str(arg);
            s.push('\u{1E}');
        }
        s
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EnterAction {
    #[default]
    Newline,
    Go,
    Search,
    Send,
    Next,
    Done,
    Previous,
}

/// What the focused field is
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Editor {
    pub class: u32,
    pub password: bool,
    pub email: bool,
    pub url: bool,
    pub no_suggestions: bool,
    pub cap_sentences: bool,
    /// Capitalize every word (names): the app's caps hint is trusted
    pub cap_words: bool,
    pub multi_line: bool,
    pub action: EnterAction,
}

impl Editor {
    /// From Android `EditorInfo.inputType` and `imeOptions`
    pub fn from_android(input_type: i32, ime_options: i32) -> Self {
        let it = input_type as u32;
        let class = it & 0xF;
        let variation = it & 0xFF0;
        let flags = it & 0xFFF000;
        let password = class == 1 && matches!(variation, 0x80 | 0x90 | 0xE0) || class == 2 && variation == 0x10;
        let multi_line = class == 1 && flags & 0x20000 != 0;
        let action = if ime_options as u32 & 0x4000_0000 != 0 {
            EnterAction::Newline
        } else {
            match ime_options & 0xFF {
                2 => EnterAction::Go,
                3 => EnterAction::Search,
                4 => EnterAction::Send,
                5 => EnterAction::Next,
                6 => EnterAction::Done,
                7 => EnterAction::Previous,
                _ => EnterAction::Newline,
            }
        };
        Editor {
            class,
            password,
            email: class == 1 && matches!(variation, 0x20 | 0xD0),
            url: class == 1 && variation == 0x10,
            no_suggestions: class == 1 && flags & 0x80000 != 0,
            cap_sentences: class == 1 && flags & (0x1000 | 0x2000 | 0x4000) != 0,
            cap_words: class == 1 && flags & (0x1000 | 0x2000) != 0,
            multi_line,
            action: if multi_line && action == EnterAction::Newline { EnterAction::Newline } else { action },
        }
    }

    pub fn numeric(&self) -> bool {
        matches!(self.class, 2 | 3 | 4)
    }

    fn wants_suggestions(&self) -> bool {
        !self.password && !self.no_suggestions && !self.numeric() && !self.email && !self.url
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PopupKind {
    Chars,
    Languages,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Popup {
    pub kind: PopupKind,
    pub items: Vec<String>,
    pub cells: Vec<layout::Rect>,
    pub sel: usize,
    pub key: usize,
}

#[derive(Clone, Debug)]
struct Pointer {
    id: i32,
    key: Option<usize>,
    strip: Option<usize>,
    emoji: bool,
    x0: f32,
    y0: f32,
    x: f32,
    y: f32,
    t0: f64,
    done: bool,
    space_drag: bool,
    cursor_sent: i32,
    bksp_drag: bool,
    repeats: u32,
    next_repeat: Option<f64>,
    /// Gesture typing: the finger left its first key while drawing a word
    gesture: bool,
    path: Vec<(f32, f32)>,
    /// Space held long enough to move the cursor (when swiping space switches language)
    cursor_armed: bool,
}

/// The last word entered by a gesture, replaceable by its alternatives
#[derive(Clone, Debug)]
struct GestureCommit {
    committed: String,
    alts: Vec<String>,
    shift: ShiftState,
}

/// Undo record of the last automatic correction
#[derive(Clone, Debug)]
struct Correction {
    typed: String,
    committed: String,
}

/// What a strip cell does
#[derive(Clone, Debug, PartialEq)]
pub enum StripItem {
    Word { text: String, primary: bool, original: bool },
    /// A conversion candidate (Chinese, Japanese)
    Cand { text: String, idx: usize, primary: bool },
    /// Opens (false) or closes (true) the full candidate panel
    Expand(bool),
    Tool(Tool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Settings,
    Emoji,
    Paste,
    SwitchIme,
    Hide,
}

pub struct Keyboard {
    pub prefs: Prefs,
    pub editor: Editor,
    pub lang_idx: usize,
    pub mode: Mode,
    pub shift: ShiftState,
    pub keys: Vec<Key>,
    pub width: f32,
    pub dp: f32,
    pub bottom_inset: f32,
    screen_h: f32,
    pub strip_h: f32,
    pub height: f32,
    pub pressed: Vec<usize>,
    pub popup: Option<Popup>,
    pub emoji_cat: usize,
    pub emoji_scroll: f32,
    pub predictor: Predictor,
    pub touch: TouchModel,
    models: HashMap<String, Arc<Model>>,
    /// Personal dictionaries of the languages not in use right now
    dicts: HashMap<String, UserDict>,
    dict_lang: String,
    calib: HashMap<String, HashMap<char, (f32, f32)>>,
    composing: String,
    taps: Vec<Tap>,
    hangul: Hangul,
    cjk_input: String,
    pub cands: Vec<Conv>,
    conv_sel: Option<usize>,
    convs: HashMap<String, Arc<ConvDict>>,
    pub cand_scroll: f32,
    ctx_before: String,
    caps_hint: bool,
    pub suggestions: Suggestions,
    pub predictions: Vec<String>,
    correction: Option<Correction>,
    auto_space: bool,
    last_space_t: f64,
    last_shift_t: f64,
    static_grid_next: bool,
    pointers: Vec<Pointer>,
    /// Points of the gesture being drawn, for painting the trail
    pub trail: Vec<(f32, f32)>,
    /// Color emoji drawn by the platform, by emoji text
    pub emoji_images: HashMap<String, EmojiImage>,
    /// Emoji categories already sent for drawing (the tab icons are category usize::MAX)
    emoji_asked: std::collections::HashSet<usize>,
    gesture: Option<GestureCommit>,
    out: Vec<Cmd>,
    pub dirty: bool,
}

impl Keyboard {
    pub fn new(prefs: Prefs) -> Self {
        let mut kb = Keyboard {
            prefs,
            editor: Editor::default(),
            lang_idx: 0,
            mode: Mode::Alphabet,
            shift: ShiftState::Off,
            keys: Vec::new(),
            width: 1080.0,
            dp: 2.75,
            bottom_inset: 0.0,
            screen_h: 0.0,
            strip_h: 0.0,
            height: 0.0,
            pressed: Vec::new(),
            popup: None,
            emoji_cat: 0,
            emoji_scroll: 0.0,
            predictor: Predictor::default(),
            touch: TouchModel::default(),
            models: HashMap::new(),
            dicts: HashMap::new(),
            dict_lang: String::new(),
            calib: HashMap::new(),
            composing: String::new(),
            taps: Vec::new(),
            hangul: Hangul::default(),
            cjk_input: String::new(),
            cands: Vec::new(),
            conv_sel: None,
            convs: HashMap::new(),
            cand_scroll: 0.0,
            ctx_before: String::new(),
            caps_hint: false,
            suggestions: Suggestions::default(),
            predictions: Vec::new(),
            correction: None,
            auto_space: false,
            last_space_t: f64::NEG_INFINITY,
            last_shift_t: f64::NEG_INFINITY,
            static_grid_next: false,
            pointers: Vec::new(),
            trail: Vec::new(),
            emoji_images: HashMap::new(),
            emoji_asked: Default::default(),
            gesture: None,
            out: Vec::new(),
            dirty: true,
        };
        kb.rebuild();
        kb.request_model();
        kb
    }

    pub fn lang(&self) -> &'static LangDef {
        self.prefs
            .languages
            .get(self.lang_idx)
            .and_then(|c| LangDef::by_code(c))
            .unwrap_or_else(|| LangDef::by_code("en").unwrap())
    }

    pub fn take_commands(&mut self) -> Vec<Cmd> {
        if self.mode == Mode::Emoji {
            // ask the platform for color images of what the palette shows
            let px = (self.emoji_cell() * 0.62).round() as u32;
            for cat in [usize::MAX, self.emoji_cat] {
                if self.emoji_asked.insert(cat) {
                    let list: Vec<&str> = if cat == usize::MAX {
                        crate::emoji::CATEGORIES.iter().map(|c| c.0).collect()
                    } else {
                        crate::emoji::items(cat)
                    };
                    self.out.push(Cmd::RenderEmoji(format!("{px} {}", list.join(" "))));
                }
            }
        }
        std::mem::take(&mut self.out)
    }

    /// The platform drew an emoji
    pub fn emoji_loaded(&mut self, text: &str, image: EmojiImage) {
        self.emoji_images.insert(text.to_string(), image);
        if self.mode == Mode::Emoji {
            self.dirty = true;
        }
    }

    pub fn composing(&self) -> &str {
        &self.composing
    }

    // ------------------------------------------------------------------ geometry

    fn row_h(&self) -> f32 {
        let want = 54.0 * self.dp * self.prefs.height;
        if self.screen_h <= 0.0 {
            return want;
        }
        // landscape and short screens: the keyboard takes at most about half of the screen
        let rows = self.rows_for_height().max(1) as f32;
        let room = (self.screen_h * MAX_SCREEN_SHARE - self.strip_h - 4.0 * self.dp) / rows;
        want.min(room).max(30.0 * self.dp)
    }

    /// Height of the screen in px, so the keyboard can stay small in landscape
    pub fn set_screen_height(&mut self, h: f32) {
        self.screen_h = h;
    }

    fn rows_for_height(&self) -> usize {
        // the tallest enabled layout fixes the height, so switching languages never jumps
        let letters = self
            .prefs
            .languages
            .iter()
            .filter_map(|c| LangDef::by_code(c))
            .map(|l| l.rows.len())
            .max()
            .unwrap_or(3)
            .max(3);
        letters + 1 + self.prefs.number_row as usize
    }

    /// Total height in px for a width (including the bottom inset)
    pub fn measure(&mut self, width: f32, dp: f32, bottom_inset: f32) -> f32 {
        self.width = width;
        self.dp = dp;
        self.bottom_inset = bottom_inset;
        self.strip_h = 46.0 * dp;
        let h = self.strip_h + self.rows_for_height() as f32 * self.row_h() + 4.0 * dp + bottom_inset;
        if (h - self.height).abs() > 0.5 {
            self.height = h;
        }
        self.rebuild();
        h
    }

    fn key_frame(&self) -> Frame {
        Frame { w: self.width, top: self.strip_h, h: self.height - self.strip_h - self.bottom_inset, dp: self.dp }
    }

    pub fn rebuild(&mut self) {
        if self.height <= 0.0 {
            self.dirty = true;
            return;
        }
        let lang = self.lang();
        let enter = match self.editor.action {
            EnterAction::Newline => "",
            EnterAction::Go => "go",
            EnterAction::Search => "search",
            EnterAction::Send => "send",
            EnterAction::Next => "next",
            EnterAction::Done => "done",
            EnterAction::Previous => "prev",
        };
        let o = BuildOptions {
            lang,
            mode: self.mode,
            shift: self.shift,
            number_row: self.prefs.number_row && self.mode == Mode::Alphabet,
            multi_lang: self.prefs.languages.len() > 1,
            enter_label: enter,
            email: self.editor.email,
            url: self.editor.url,
        };
        self.keys = layout::build(&o, self.key_frame());
        if self.mode == Mode::Alphabet && self.shift == ShiftState::Off {
            self.rebuild_touch_model();
        }
        self.popup = None;
        self.pressed.clear();
        self.dirty = true;
    }

    fn rebuild_touch_model(&mut self) {
        let lang = self.lang();
        let geoms: Vec<KeyGeom> = self
            .keys
            .iter()
            .filter(|k| k.letter)
            .filter_map(|k| {
                let mut c = k.label.chars();
                let ch = c.next()?;
                c.next().is_none().then(|| KeyGeom { ch, cx: k.rect.cx(), cy: k.rect.cy(), w: k.rect.w, h: k.rect.h })
            })
            .collect();
        let variants: Vec<(char, Vec<char>)> = lang
            .popups
            .iter()
            .filter_map(|(b, v)| {
                let b = b.chars().next()?;
                Some((b, v.iter().filter_map(|s| s.chars().next()).collect()))
            })
            .collect();
        let old = std::mem::take(&mut self.touch);
        if !old.keys().is_empty() {
            self.calib.insert(self.prefs.languages.get(self.lang_idx).cloned().unwrap_or_default(), old.calibration().clone());
        }
        self.touch = TouchModel::new(geoms, &variants);
        if let Some(c) = self.calib.get(lang.code) {
            self.touch.set_calibration(c.clone());
        }
    }

    // ------------------------------------------------------------------ models

    /// Puts the personal dictionary of the current language in place
    fn swap_dict(&mut self) {
        let code = self.dict_code();
        if code == self.dict_lang {
            return;
        }
        let old = std::mem::take(&mut self.predictor.user);
        if !self.dict_lang.is_empty() {
            self.dicts.insert(std::mem::take(&mut self.dict_lang), old);
        }
        self.predictor.set_user(self.dicts.remove(&code).unwrap_or_default());
        self.dict_lang = code;
    }

    /// Dictionary key: the language model code, so "en" and "en-GB" share words
    fn dict_code(&self) -> String {
        let l = self.lang();
        if l.lm.is_empty() { l.code.to_string() } else { l.lm.to_string() }
    }

    /// All personal dictionaries: (language, serialized)
    pub fn user_dicts(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = self.dicts.iter().map(|(k, d)| (k.clone(), d.serialize())).collect();
        if !self.dict_lang.is_empty() {
            v.push((self.dict_lang.clone(), self.predictor.user.serialize()));
        }
        v
    }

    pub fn load_user_dict(&mut self, code: &str, text: &str) {
        let d = UserDict::parse(text);
        if code == self.dict_lang {
            self.predictor.set_user(d);
        } else {
            self.dicts.insert(code.to_string(), d);
        }
    }

    pub fn clear_user_dicts(&mut self) {
        self.dicts.clear();
        self.predictor.set_user(UserDict::default());
    }

    fn request_model(&mut self) {
        self.swap_dict();
        let lang = self.lang();
        if !lang.conv.is_empty() {
            let path = format!("cjk/{}.rcd", lang.conv);
            if !self.convs.contains_key(&path) {
                self.out.push(Cmd::LoadModel(path));
            }
        }
        if lang.lm.is_empty() {
            self.predictor.set_model(None);
            return;
        }
        let path = format!("lm/{}.rlm", lang.lm);
        match self.models.get(&path) {
            Some(m) => self.predictor.set_model(Some(m.clone())),
            None => {
                self.predictor.set_model(None);
                self.out.push(Cmd::LoadModel(path));
            }
        }
    }

    /// A language model finished loading in the background (`path` as in [`Cmd::LoadModel`])
    pub fn model_loaded(&mut self, path: &str, model: Arc<Model>) {
        // keep at most the current and one more model in memory
        let cur = format!("lm/{}.rlm", self.lang().lm);
        if self.models.len() >= 2 {
            self.models.retain(|k, _| *k == cur);
        }
        self.models.insert(path.to_string(), model);
        if cur == path {
            self.request_model();
            self.refresh_suggestions();
        }
    }

    /// A conversion dictionary finished loading
    pub fn conv_loaded(&mut self, path: &str, dict: Arc<ConvDict>) {
        let cur = format!("cjk/{}.rcd", self.lang().conv);
        if self.convs.len() >= 2 {
            self.convs.retain(|k, _| *k == cur);
        }
        self.convs.insert(path.to_string(), dict);
        if cur == path {
            self.refresh_cands();
        }
    }

    // ------------------------------------------------------------------ editor lifecycle

    pub fn start_input(&mut self, editor: Editor, restarting: bool) {
        if !restarting {
            self.reset_composition(false);
        }
        self.editor = editor;
        self.mode = if self.editor.numeric() { Mode::Symbols } else { Mode::Alphabet };
        self.shift = ShiftState::Off;
        self.correction = None;
        self.auto_space = false;
        self.rebuild();
        self.update_auto_shift();
        self.refresh_predictions();
    }

    /// Text before the cursor changed outside of our own edits (or we are told the real one)
    pub fn set_context(&mut self, before: &str, caps: bool) {
        self.ctx_before = before.to_string();
        self.caps_hint = caps;
        if let Some(c) = &self.correction {
            if !before.ends_with(&c.committed) {
                self.correction = None;
            }
        }
        if !before.ends_with(' ') {
            self.auto_space = false;
        }
        if self.composing.is_empty() {
            self.update_auto_shift();
            self.refresh_predictions();
        }
        self.dirty = true;
    }

    /// The cursor jumped: whatever we were composing is now plain text
    pub fn reset_composition(&mut self, finish_hangul: bool) {
        if finish_hangul {
            self.hangul = Hangul::default();
        }
        self.composing.clear();
        self.taps.clear();
        self.cjk_input.clear();
        self.cands.clear();
        self.conv_sel = None;
        self.hangul = Hangul::default();
        self.suggestions = Suggestions::default();
        self.correction = None;
        self.gesture = None;
        self.auto_space = false;
        self.dirty = true;
    }

    fn context(&self) -> Context {
        context_of(&self.ctx_before)
    }

    fn update_auto_shift(&mut self) {
        if self.shift == ShiftState::Caps || self.mode != Mode::Alphabet || self.lang().caseless {
            return;
        }
        let ctx = self.context();
        let at_start = ctx.prev.is_empty() && ctx.sentence_start;
        // The app's caps hint is only trusted for fields that capitalize every word (names):
        // some apps report "capitalize" after every space even in ordinary text fields
        let hint = self.caps_hint && self.editor.cap_words;
        let want = self.prefs.autocap && self.editor.cap_sentences && (hint || at_start);
        let s = if want { ShiftState::On } else { ShiftState::Off };
        if s != self.shift {
            self.shift = s;
            self.rebuild();
        }
    }

    // ------------------------------------------------------------------ output helpers

    fn emit(&mut self, c: Cmd) {
        self.out.push(c);
    }

    fn commit(&mut self, text: &str) {
        self.ctx_before.push_str(text);
        self.emit(Cmd::Commit(text.to_string()));
    }

    fn delete_before(&mut self, text: &str) {
        if self.ctx_before.ends_with(text) {
            let n = self.ctx_before.len() - text.len();
            self.ctx_before.truncate(n);
        }
        self.emit(Cmd::DeleteBefore(text.to_string()));
    }

    fn set_composing(&mut self) {
        let shown = self.composing_display();
        self.emit(Cmd::Composing(shown));
    }

    fn composing_display(&self) -> String {
        match self.composer() {
            Composer::Hangul => self.hangul.text(),
            Composer::None => self.composing.clone(),
            _ => self.cjk_display(),
        }
    }

    fn suggestions_enabled(&self) -> bool {
        self.prefs.suggestions && self.editor.wants_suggestions()
    }

    fn autocorrect_enabled(&self) -> bool {
        let script_ok = !matches!(self.lang().script, "Thai" | "Lao" | "Khmer" | "Myanmar" | "CJK" | "Ethiopic");
        self.prefs.autocorrect && self.editor.wants_suggestions() && script_ok
    }

    fn refresh_suggestions(&mut self) {
        if self.composing.is_empty() || !self.suggestions_enabled() || self.composer() != Composer::None {
            self.suggestions = Suggestions::default();
        } else {
            let ctx = self.context();
            let ac = self.autocorrect_enabled();
            let composing = self.composing.clone();
            let taps = self.taps.clone();
            self.suggestions = self.predictor.suggest(&composing, &taps, &self.touch, &ctx, ac);
        }
        self.dirty = true;
    }

    fn refresh_predictions(&mut self) {
        self.predictions = if self.suggestions_enabled() && self.composing.is_empty() && self.composer() == Composer::None {
            let ctx = self.context();
            let cap = self.shift != ShiftState::Off && !self.lang().caseless;
            if ctx.prev.is_empty() {
                Vec::new()
            } else {
                self.predictor.predict(&ctx, cap)
            }
        } else {
            Vec::new()
        };
        self.dirty = true;
    }

    // ------------------------------------------------------------------ typing

    fn is_word_char(s: &str) -> bool {
        s.chars().all(|c| c.is_alphanumeric() || is_mark(c) || c == '\'' || c == '’' || c == '-' || c == 'ʼ' || c == 'ʻ' || c == '\u{200D}')
            && !s.is_empty()
    }

    /// Types the text of a key (or popup choice); `pos` is the touch for letters
    pub fn type_text(&mut self, text: &str, pos: Option<(f32, f32)>) {
        self.correction = None;
        self.gesture = None;
        match self.composer() {
            Composer::Hangul => {
                let mut c = text.chars();
                if let (Some(ch), None) = (c.next(), c.clone().next()) {
                    if crate::compose::hangul::is_jamo(ch) {
                        let done = self.hangul.input(ch);
                        if !done.is_empty() {
                            self.commit(&done);
                        }
                        self.set_composing();
                        self.after_letter();
                        return;
                    }
                }
                let done = self.hangul.finish();
                self.commit(&(done + text));
                self.after_separator(text);
                return;
            }
            c if c.converts() && self.composer() == c => {
                self.cjk_type(text);
                self.after_letter();
                return;
            }
            _ => {}

        }

        if Self::is_word_char(text) && !(text == "-" && self.composing.is_empty()) && !(text == "'" && self.composing.is_empty()) {
            // a letter or digit continues the word
            self.auto_space = false;
            if self.lang().code == "vi" && !self.composing.is_empty() {
                let mut cs = text.chars();
                if let (Some(key), None) = (cs.next(), cs.next()) {
                    if let Some(word) = crate::compose::telex::apply(&self.composing, key) {
                        // Telex rewrote the word: its letters are now exact, not touches
                        self.composing = word;
                        self.taps = self.composing.to_lowercase().chars().map(|ch| Tap { ch, pos: None }).collect();
                        self.set_composing();
                        self.after_letter();
                        self.refresh_suggestions();
                        return;
                    }
                }
            }
            self.composing.push_str(text);
            let lower = text.to_lowercase();
            let ch = lower.chars().next().unwrap_or(' ');
            self.taps.push(Tap { ch, pos });
            self.set_composing();
            self.after_letter();
            self.refresh_suggestions();
            return;
        }

        // separator: finish the word first
        let smart = SMART_PUNCT.contains(&text);
        let had_word = !self.composing.is_empty();
        if had_word {
            self.finish_word(true, "");
        }
        if smart && self.auto_space && self.ctx_before.ends_with(' ') {
            // "word ," → "word, "
            self.delete_before(" ");
            self.commit(&format!("{text} "));
            self.auto_space = true;
        } else {
            self.commit(text);
            self.auto_space = false;
        }
        self.after_separator(text);
    }

    fn after_letter(&mut self) {
        self.static_grid_next = false;
        if self.shift == ShiftState::On {
            self.shift = ShiftState::Off;
            self.rebuild();
        }
        self.dirty = true;
    }

    fn after_separator(&mut self, text: &str) {
        self.static_grid_next = false;
        if text.chars().any(|c| SENTENCE_END.contains(&c)) || self.shift == ShiftState::On {
            self.update_auto_shift();
        }
        self.refresh_predictions();
        self.suggestions = Suggestions::default();
        self.dirty = true;
    }

    /// Commits the word being composed, applying autocorrection when allowed, plus `sep`
    fn finish_word(&mut self, allow_correction: bool, sep: &str) {
        match self.composer() {
            Composer::Hangul => {
                let t = self.hangul.finish() + sep;
                if !t.is_empty() {
                    self.commit(&t);
                }
                return;
            }
            c if c.converts() => {
                self.cjk_commit_all();
                if !sep.is_empty() {
                    self.commit(sep);
                }
                return;
            }
            _ => {}

        }
        if self.composing.is_empty() {
            if !sep.is_empty() {
                self.commit(sep);
            }
            return;
        }
        let typed = std::mem::take(&mut self.composing);
        let taps = std::mem::take(&mut self.taps);
        let corrected = if allow_correction && self.autocorrect_enabled() {
            // decide on fresh suggestions for exactly this word
            let ctx = self.context();
            self.predictor.suggest(&typed, &taps, &self.touch, &ctx, true).autocorrect
        } else {
            None
        };
        let word = corrected.clone().unwrap_or_else(|| typed.clone());
        let prev = self.context().prev.first().cloned();
        let text = format!("{word}{sep}");
        self.commit(&text);
        if corrected.is_some() {
            self.correction = Some(Correction { typed: typed.clone(), committed: text });
        } else {
            self.correction = None;
            self.touch.learn(&taps, &word);
        }
        if self.prefs.learn && !self.editor.password {
            self.predictor.learn(&word, prev.as_deref());
        }
        self.suggestions = Suggestions::default();
    }

    /// Can a slide across letters be a gesture word here?
    fn gesture_allowed(&self) -> bool {
        let script_ok = !matches!(self.lang().script, "Thai" | "Lao" | "Khmer" | "Myanmar" | "CJK" | "Ethiopic");
        self.prefs.swipe_typing
            && self.mode == Mode::Alphabet
            && self.composer() == Composer::None
            && self.predictor.model.is_some()
            && !self.editor.password
            && !self.editor.url
            && !self.editor.email
            && self.editor.class == 1
            && script_ok
    }

    fn cased(word: &str, shift: ShiftState) -> String {
        match shift {
            ShiftState::Off => word.to_string(),
            ShiftState::On => crate::engine::capitalize_first(word),
            ShiftState::Caps => word.to_uppercase(),
        }
    }

    /// Enters the word drawn by a gesture
    fn gesture_word(&mut self, path: &[(f32, f32)]) {
        if !self.composing.is_empty() {
            self.finish_word(true, " ");
        }
        let ctx = self.context();
        let alts = self.predictor.gesture(path, &self.touch, &ctx, 4);
        let Some(best) = alts.first().cloned() else {
            self.dirty = true;
            return;
        };
        if self.ctx_before.chars().last().is_some_and(|c| c.is_alphanumeric() || is_mark(c)) {
            self.commit(" ");
        }
        let shift = self.shift;
        let word = Self::cased(&best, shift);
        let prev = self.context().prev.first().cloned();
        let committed = format!("{word} ");
        self.commit(&committed);
        if self.prefs.learn && !self.editor.password {
            self.predictor.learn(&word, prev.as_deref());
        }
        self.correction = None;
        self.auto_space = true;
        self.last_space_t = f64::NEG_INFINITY;
        self.after_letter();
        self.update_auto_shift();
        self.refresh_predictions();
        self.gesture = Some(GestureCommit { committed, alts, shift });
        self.dirty = true;
    }

    pub fn space(&mut self, t: f64) {
        if self.gesture.take().is_some() {
            // a gesture word already ends with a space
            self.dirty = true;
            return;
        }
        let lang = self.lang();
        if self.cjk_active() {
            self.cjk_space();
            return;
        }
        let had_word = !self.composing.is_empty() || !self.hangul.is_empty();
        if had_word {
            self.finish_word(true, " ");
            self.auto_space = true;
            self.last_space_t = t;
        } else if self.prefs.double_space_period
            && self.auto_space
            && t - self.last_space_t < DOUBLE_SPACE_MS
            && self.ctx_before.ends_with(' ')
            && self.ctx_before.trim_end().chars().last().is_some_and(|c| c.is_alphanumeric())
            && !self.editor.password
        {
            self.delete_before(" ");
            let dot = match lang.script {
                "Devanagari" | "Bengali" | "Gurmukhi" | "Oriya" => "।",
                "Arabic" | "Hebrew" => ".",
                _ => ".",
            };
            self.commit(&format!("{dot} "));
            self.correction = None;
            self.auto_space = true;
            self.last_space_t = f64::NEG_INFINITY;
        } else {
            self.commit(" ");
            self.correction = None;
            self.auto_space = false;
            self.last_space_t = t;
        }
        self.after_separator(" ");
    }

    pub fn backspace(&mut self) {
        self.static_grid_next = true;
        if let Some(g) = self.gesture.take() {
            // the whole swiped word goes at once
            self.delete_before(&g.committed);
            self.update_auto_shift();
            self.refresh_predictions();
            self.dirty = true;
            return;
        }
        match self.composer() {
            Composer::Hangul if !self.hangul.is_empty() => {
                self.hangul.backspace();
                self.set_composing();
                self.dirty = true;
                return;
            }
            c if c.converts() && !self.cjk_input.is_empty() => {
                self.cjk_backspace();
                return;
            }
            _ => {}
        }
        if !self.composing.is_empty() {
            let mut chars: Vec<char> = self.composing.chars().collect();
            chars.pop();
            // drop dangling combining marks together with their letter
            while chars.last().is_some_and(|c| is_mark(*c)) && chars.len() > 1 && self.lang().script == "Latin" {
                chars.pop();
            }
            self.composing = chars.into_iter().collect();
            self.taps.truncate(self.composing.chars().count());
            self.set_composing();
            if self.composing.is_empty() {
                self.suggestions = Suggestions::default();
                self.refresh_predictions();
            } else {
                self.refresh_suggestions();
            }
            self.dirty = true;
            return;
        }
        if let Some(c) = self.correction.take() {
            // undo the autocorrection and remember the user's word
            self.delete_before(&c.committed);
            self.predictor.learn_rejected(&c.typed);
            self.composing = c.typed.clone();
            self.taps = c.typed.to_lowercase().chars().map(|ch| Tap { ch, pos: None }).collect();
            self.set_composing();
            self.suggestions = Suggestions { items: vec![c.typed.clone()], autocorrect: None };
            self.auto_space = false;
            self.dirty = true;
            return;
        }
        self.auto_space = false;
        // keep our mirror of the text in step; the editor reports the truth afterwards
        let mut chars: Vec<char> = self.ctx_before.chars().collect();
        chars.pop();
        self.ctx_before = chars.into_iter().collect();
        self.emit(Cmd::DelKey);
        self.update_auto_shift();
        self.refresh_predictions();
    }

    /// Deletes `n` words before the cursor (backspace swipe)
    fn delete_words(&mut self, n: usize) {
        if !self.composing.is_empty() {
            self.composing.clear();
            self.taps.clear();
            self.set_composing();
            if n <= 1 {
                return;
            }
        }
        let before = self.ctx_before.clone();
        let mut cut = before.len();
        for _ in 0..n {
            let s = &before[..cut];
            let trimmed = s.trim_end_matches(|c: char| !c.is_alphanumeric());
            let word_start = trimmed.rfind(|c: char| !c.is_alphanumeric() && !is_mark(c)).map_or(0, |i| i + trimmed[i..].chars().next().unwrap().len_utf8());
            cut = word_start;
            if cut == 0 {
                break;
            }
        }
        let tail = before[cut..].to_string();
        if !tail.is_empty() {
            self.delete_before(&tail);
        }
        self.correction = None;
        self.update_auto_shift();
        self.refresh_predictions();
    }

    pub fn enter(&mut self) {
        self.gesture = None;
        if self.cjk_active() {
            self.cjk_enter();
            return;
        }
        self.finish_word(true, "");
        if self.editor.action == EnterAction::Newline || self.editor.multi_line && self.editor.action == EnterAction::Newline {
            self.commit("\n");
            self.after_separator(".");
        } else {
            self.emit(Cmd::EditorAction);
        }
        self.correction = None;
    }

    pub fn pick_strip(&mut self, i: usize) {
        let items = self.strip_items();
        let Some(item) = items.get(i).cloned() else { return };
        match item {
            StripItem::Tool(t) => match t {
                Tool::Settings => self.emit(Cmd::OpenSettings),
                Tool::Emoji => {
                    self.mode = Mode::Emoji;
                    self.rebuild();
                }
                Tool::Paste => self.emit(Cmd::Paste),
                Tool::SwitchIme => self.emit(Cmd::SwitchIme),
                Tool::Hide => self.emit(Cmd::Hide),
            },
            StripItem::Cand { idx, .. } => self.cjk_pick(idx),
            StripItem::Expand(_) => self.toggle_cand_panel(),
            StripItem::Word { text, .. } if self.gesture.is_some() && self.composing.is_empty() => {
                let g = self.gesture.take().unwrap();
                self.delete_before(&g.committed);
                let word = Self::cased(&text, g.shift);
                let committed = format!("{word} ");
                self.commit(&committed);
                if self.prefs.learn && !self.editor.password {
                    let prev = self.context().prev.get(1).cloned();
                    self.predictor.learn(&word, prev.as_deref());
                }
                self.gesture = Some(GestureCommit { committed, ..g });
            }
            StripItem::Word { text, original, .. } => {
                let prev = self.context().prev.first().cloned();
                let typed = std::mem::take(&mut self.composing);
                self.taps.clear();
                let word = if original { typed.clone() } else { text };
                if original {
                    self.predictor.learn_rejected(&typed);
                }
                self.commit(&format!("{word} "));
                if self.prefs.learn && !self.editor.password {
                    self.predictor.learn(&word, prev.as_deref());
                }
                self.correction = None;
                self.auto_space = true;
                self.last_space_t = f64::NEG_INFINITY;
                self.after_separator(" ");
            }
        }
        self.dirty = true;
    }

    /// Cells of the strip above the keys
    pub fn strip_items(&self) -> Vec<StripItem> {
        self.strip_layout().into_iter().map(|c| c.0).collect()
    }

    /// Strip cells with their horizontal extent in px
    pub fn strip_layout(&self) -> Vec<(StripItem, f32, f32)> {
        if self.cjk_active() {
            return self.cand_strip();
        }
        let items = self.strip_words();
        let n = items.len().max(1) as f32;
        let w = self.width / n;
        items.into_iter().enumerate().map(|(i, it)| (it, i as f32 * w, (i + 1) as f32 * w)).collect()
    }

    fn strip_words(&self) -> Vec<StripItem> {
        if let Some(g) = self.gesture.as_ref().filter(|g| g.alts.len() > 1) {
            // the chosen word in the middle, alternatives around it
            let chosen = g.committed.trim_end().to_string();
            let others: Vec<String> = g.alts.iter().map(|a| Self::cased(a, g.shift)).filter(|a| *a != chosen).take(2).collect();
            let mut items: Vec<StripItem> = Vec::new();
            if let Some(a) = others.first() {
                items.push(StripItem::Word { text: a.clone(), primary: false, original: false });
            }
            items.push(StripItem::Word { text: chosen, primary: true, original: false });
            if let Some(a) = others.get(1) {
                items.push(StripItem::Word { text: a.clone(), primary: false, original: false });
            }
            return items;
        }
        if !self.composing.is_empty() && !self.suggestions.items.is_empty() {
            let primary = self.suggestions.items.len() / 2;
            let primary = if self.suggestions.items.len() == 1 { 0 } else { primary.max(1).min(self.suggestions.items.len() - 1) };
            return self
                .suggestions
                .items
                .iter()
                .enumerate()
                .map(|(i, t)| StripItem::Word {
                    text: t.clone(),
                    primary: i == primary,
                    original: t.starts_with('«') && t.ends_with('»') && self.suggestions.autocorrect.is_some(),
                })
                .collect();
        }
        if self.composing.is_empty() && !self.predictions.is_empty() {
            let mid = self.predictions.len() / 2;
            return self
                .predictions
                .iter()
                .enumerate()
                .map(|(i, t)| StripItem::Word { text: t.clone(), primary: i == mid, original: false })
                .collect();
        }
        vec![
            StripItem::Tool(Tool::Settings),
            StripItem::Tool(Tool::Emoji),
            StripItem::Tool(Tool::Paste),
            StripItem::Tool(Tool::SwitchIme),
            StripItem::Tool(Tool::Hide),
        ]
    }

    // ------------------------------------------------------------------ modes & language

    fn toggle_shift(&mut self, t: f64) {
        self.shift = match self.shift {
            ShiftState::Off => {
                self.last_shift_t = t;
                ShiftState::On
            }
            ShiftState::On if t - self.last_shift_t < DOUBLE_TAP_MS && !self.lang().caseless => ShiftState::Caps,
            _ => ShiftState::Off,
        };
        self.rebuild();
    }

    pub fn set_language(&mut self, idx: usize) {
        if idx >= self.prefs.languages.len() || idx == self.lang_idx {
            return;
        }
        let rows_before = self.lang().rows.len();
        self.finish_word(false, "");
        self.reset_composition(true);
        self.lang_idx = idx;
        self.mode = Mode::Alphabet;
        self.shift = ShiftState::Off;
        self.rebuild();
        self.request_model();
        self.update_auto_shift();
        self.refresh_predictions();
        if rows_before != self.lang().rows.len() {
            self.dirty = true;
        }
    }

    pub fn next_language(&mut self) {
        let n = self.prefs.languages.len().max(1);
        self.set_language((self.lang_idx + 1) % n);
    }

    pub fn apply_prefs(&mut self, prefs: Prefs) {
        let code = self.prefs.languages.get(self.lang_idx).cloned();
        let height_changed = prefs.height != self.prefs.height || prefs.number_row != self.prefs.number_row || prefs.languages != self.prefs.languages;
        self.prefs = prefs;
        self.lang_idx = code.and_then(|c| self.prefs.languages.iter().position(|x| *x == c)).unwrap_or(0);
        if height_changed {
            self.emit(Cmd::Relayout);
        }
        self.rebuild();
        self.request_model();
    }

    // ------------------------------------------------------------------ touch

    fn key_at(&self, x: f32, y: f32) -> Option<usize> {
        self.keys.iter().position(|k| k.hit.contains(x, y)).or_else(|| {
            if y < self.strip_h {
                return None;
            }
            self.keys
                .iter()
                .enumerate()
                .min_by(|a, b| {
                    let da = (a.1.rect.cx() - x).powi(2) + (a.1.rect.cy() - y).powi(2);
                    let db = (b.1.rect.cx() - x).powi(2) + (b.1.rect.cy() - y).powi(2);
                    da.total_cmp(&db)
                })
                .map(|k| k.0)
        })
    }

    fn strip_cell_at(&self, x: f32) -> usize {
        let cells = self.strip_layout();
        cells.iter().position(|c| x >= c.1 && x < c.2).unwrap_or(cells.len().saturating_sub(1))
    }

    pub fn emoji_area(&self) -> (f32, f32) {
        let tabs = self.row_h();
        (self.strip_h, self.height - self.bottom_inset - tabs)
    }

    pub fn emoji_cell(&self) -> f32 {
        self.width / 8.0
    }

    pub fn touch_down(&mut self, id: i32, x: f32, y: f32, t: f64) {
        // rollover: a second finger commits the first finger's key right away
        let pending: Vec<usize> = self.pointers.iter().filter(|p| !p.done && !p.gesture && p.key.is_some()).map(|p| p.id as usize).collect();
        for pid in pending {
            if let Some(i) = self.pointers.iter().position(|p| p.id as usize == pid) {
                let p = self.pointers[i].clone();
                if self.popup.is_none() && p.key.is_some_and(|k| matches!(self.keys.get(k).map(|k| &k.kind), Some(KeyKind::Char(_)))) {
                    self.release_key(&p, t);
                    self.pointers[i].done = true;
                }
            }
        }
        let mut p = Pointer {
            id,
            key: None,
            strip: None,
            emoji: false,
            x0: x,
            y0: y,
            x,
            y,
            t0: t,
            done: false,
            space_drag: false,
            cursor_sent: 0,
            bksp_drag: false,
            repeats: 0,
            next_repeat: None,
            gesture: false,
            path: vec![(x, y)],
            cursor_armed: false,
        };
        if y < self.strip_h {
            p.strip = Some(self.strip_cell_at(x));
        } else if self.mode == Mode::Candidates {
            p.emoji = true;
        } else if self.mode == Mode::Emoji {
            let (_, bottom) = self.emoji_area();
            if y < bottom {
                p.emoji = true;
            } else {
                p.key = Some(usize::MAX);
            }
        } else if let Some(k) = self.key_at(x, y) {
            p.key = Some(k);
            self.pressed.push(k);
            if self.prefs.vibrate {
                self.emit(Cmd::Haptic);
            }
            match self.keys[k].kind {
                KeyKind::Backspace => {
                    self.backspace();
                    p.next_repeat = Some(t + REPEAT_START_MS);
                }
                KeyKind::Shift => self.toggle_shift(t),
                _ => {}
            }
        }
        self.pointers.push(p);
        self.dirty = true;
    }

    pub fn touch_move(&mut self, id: i32, x: f32, y: f32, _t: f64) {
        let Some(i) = self.pointers.iter().position(|p| p.id == id) else { return };
        self.pointers[i].x = x;
        self.pointers[i].y = y;
        let p = self.pointers[i].clone();
        if p.done {
            return;
        }
        if p.emoji && self.mode == Mode::Candidates {
            self.cand_scroll = (self.cand_scroll - (y - p.y0)).max(0.0);
            self.pointers[i].y0 = y;
            if (y - p.y0).abs() > 2.0 * self.dp {
                self.pointers[i].space_drag = true;
            }
            self.dirty = true;
            return;
        }
        if p.emoji {
            self.emoji_scroll = (self.emoji_scroll - (y - p.y0)).max(0.0);
            self.pointers[i].y0 = y;
            if (y - p.y0).abs() > 0.0 {
                self.pointers[i].space_drag = true; // moved: not a tap
            }
            self.dirty = true;
            return;
        }
        if let Some(pop) = &mut self.popup {
            if pop.key == p.key.unwrap_or(usize::MAX) {
                let sel = pop
                    .cells
                    .iter()
                    .enumerate()
                    .min_by(|a, b| (a.1.cx() - x).abs().total_cmp(&(b.1.cx() - x).abs()))
                    .map(|c| c.0)
                    .unwrap_or(0);
                if sel != pop.sel {
                    pop.sel = sel;
                    self.dirty = true;
                }
                return;
            }
        }
        let Some(k) = p.key.filter(|&k| k < self.keys.len()) else { return };
        match self.keys[k].kind {
            KeyKind::Space if self.prefs.space_swipe_lang && !p.cursor_armed && self.prefs.languages.len() > 1 => {
                // a quick swipe switches the language: right to the next, left to the previous
                let dx = x - p.x0;
                if dx.abs() > LANG_SWIPE_DP * self.dp && dx.abs() > (y - p.y0).abs() {
                    let n = self.prefs.languages.len();
                    let idx = if dx > 0.0 { (self.lang_idx + 1) % n } else { (self.lang_idx + n - 1) % n };
                    self.pointers[i].done = true;
                    self.pressed.retain(|&q| q != k);
                    self.set_language(idx);
                    if self.prefs.vibrate {
                        self.emit(Cmd::Haptic);
                    }
                    self.dirty = true;
                }
            }
            KeyKind::Space => {
                let step = CURSOR_STEP_DP * self.dp;
                if !p.space_drag && (x - p.x0).abs() > step * 1.5 {
                    self.pointers[i].space_drag = true;
                    self.finish_word(false, "");
                    self.reset_composition(true);
                }
                if self.pointers[i].space_drag {
                    let want = ((x - p.x0) / step) as i32;
                    let delta = want - p.cursor_sent;
                    if delta != 0 {
                        self.emit(Cmd::Cursor(delta));
                        self.pointers[i].cursor_sent = want;
                        if self.prefs.vibrate {
                            self.emit(Cmd::Haptic);
                        }
                    }
                }
            }
            KeyKind::Backspace => {
                if p.x0 - x > WORD_STEP_DP * self.dp * 0.6 {
                    self.pointers[i].bksp_drag = true;
                    self.pointers[i].next_repeat = None;
                    self.dirty = true;
                }
            }
            KeyKind::Char(_) if self.gesture_allowed() && self.pointers.len() == 1 => {
                let mut ptr = self.pointers[i].clone();
                ptr.path.push((x, y));
                if !ptr.gesture && self.popup.is_none() {
                    let kw = self.keys[k].rect.w;
                    let far = ((x - ptr.x0).powi(2) + (y - ptr.y0).powi(2)).sqrt() > kw * 0.9;
                    if far && self.key_at(x, y).is_some_and(|nk| nk != k) {
                        ptr.gesture = true;
                        ptr.space_drag = true; // no long-press popup
                        self.pressed.retain(|&q| q != k);
                    }
                }
                if ptr.gesture {
                    self.trail = ptr.path.clone();
                    self.dirty = true;
                }
                self.pointers[i] = ptr;
            }
            KeyKind::Char(_) | KeyKind::Language | KeyKind::Emoji | KeyKind::Enter | KeyKind::Symbols | KeyKind::SymbolsMore | KeyKind::Alphabet => {
                // a finger sliding to a neighbouring key moves the press with it
                if self.popup.is_none() {
                    if let Some(nk) = self.key_at(x, y) {
                        if nk != k && !self.keys[k].hit.contains(x, y) && (y - p.y0).abs() < self.keys[k].rect.h {
                            self.pointers[i].key = Some(nk);
                            self.pointers[i].t0 = _t;
                            self.pressed.retain(|&q| q != k);
                            self.pressed.push(nk);
                            self.dirty = true;
                        }
                    }
                }
            }
            KeyKind::Shift => {}
        }
    }

    pub fn touch_up(&mut self, id: i32, x: f32, y: f32, t: f64) {
        let Some(i) = self.pointers.iter().position(|p| p.id == id) else { return };
        self.pointers[i].x = x;
        self.pointers[i].y = y;
        let p = self.pointers.remove(i);
        if let Some(k) = p.key {
            self.pressed.retain(|&q| q != k);
        }
        if p.done {
            self.dirty = true;
            return;
        }
        if let Some(c) = p.strip {
            if y < self.strip_h + 20.0 * self.dp && self.strip_cell_at(x) == c {
                self.pick_strip(c);
            }
        } else if p.emoji && self.mode == Mode::Candidates {
            if !p.space_drag {
                if let Some((idx, _)) = self.cand_panel().into_iter().find(|(_, r)| r.contains(x, y)) {
                    self.cjk_pick(idx);
                }
            }
        } else if p.emoji {
            if !p.space_drag {
                self.tap_emoji(x, y);
            }
        } else if p.key == Some(usize::MAX) {
            self.tap_emoji_tabs(x);
        } else if p.gesture {
            let mut path = p.path.clone();
            path.push((x, y));
            self.trail.clear();
            self.gesture_word(&path);
        } else {
            self.release_key(&p, t);
        }
        self.dirty = true;
    }

    pub fn touch_cancel(&mut self) {
        self.pointers.clear();
        self.trail.clear();
        self.pressed.clear();
        self.popup = None;
        self.dirty = true;
    }

    fn release_key(&mut self, p: &Pointer, t: f64) {
        let Some(k) = p.key.filter(|&k| k < self.keys.len()) else { return };
        if let Some(pop) = self.popup.take() {
            if pop.key == k {
                if let Some(item) = pop.items.get(pop.sel) {
                    match pop.kind {
                        PopupKind::Chars => {
                            let item = item.clone();
                            self.type_text(&item, None);
                        }
                        PopupKind::Languages => self.set_language(pop.sel),
                    }
                }
                return;
            }
        }
        let key = self.keys[k].clone();
        match key.kind {
            KeyKind::Char(text) => {
                let mut out = text;
                let pos = (p.x0, p.y0);
                let mut with_pos = None;
                if key.letter && self.mode == Mode::Alphabet {
                    with_pos = Some(pos);
                    if self.prefs.dynamic_grid && !self.static_grid_next && self.predictor.model.is_some() && self.composer() == Composer::None {
                        let prefix = self.composing.to_lowercase();
                        let next = self.predictor.next_letters(&prefix);
                        if let Some(c) = self.touch.resolve(pos.0, pos.1, &next) {
                            let s = c.to_string();
                            out = if self.shift != ShiftState::Off { self.lang().upper(&s) } else { s };
                        }
                    }
                }
                self.type_text(&out, with_pos);
            }
            KeyKind::Space => {
                if !p.space_drag {
                    self.space(t);
                }
            }
            KeyKind::Backspace => {
                if p.bksp_drag {
                    let n = (((p.x0 - p.x) / (WORD_STEP_DP * self.dp)).ceil() as usize).max(1);
                    self.delete_words(n);
                }
            }
            KeyKind::Enter => self.enter(),
            KeyKind::Shift => {}
            KeyKind::Symbols => {
                self.finish_word(true, "");
                self.mode = Mode::Symbols;
                self.rebuild();
            }
            KeyKind::SymbolsMore => {
                self.mode = if self.mode == Mode::Symbols2 { Mode::Symbols } else { Mode::Symbols2 };
                self.rebuild();
            }
            KeyKind::Alphabet => {
                self.mode = Mode::Alphabet;
                self.shift = ShiftState::Off;
                self.rebuild();
                self.update_auto_shift();
            }
            KeyKind::Language => self.next_language(),
            KeyKind::Emoji => {
                self.finish_word(true, "");
                self.mode = Mode::Emoji;
                self.rebuild();
            }
        }
        // symbols page returns to letters after a space or quote, like most keyboards
        if self.mode == Mode::Symbols && matches!(&self.keys.get(k).map(|k| k.kind.clone()), Some(KeyKind::Space)) {
            self.mode = Mode::Alphabet;
            self.rebuild();
        }
    }

    fn tap_emoji(&mut self, x: f32, y: f32) {
        let (top, _) = self.emoji_area();
        let cell = self.emoji_cell();
        let col = (x / cell) as usize;
        let row = ((y - top + self.emoji_scroll) / cell) as usize;
        let items = emoji::items(self.emoji_cat);
        if let Some(e) = items.get(row * 8 + col.min(7)) {
            let e = e.to_string();
            self.finish_word(false, "");
            self.commit(&e);
            if self.prefs.vibrate {
                self.emit(Cmd::Haptic);
            }
        }
    }

    /// Bottom row of the emoji panel: [ABC] [categories…] [⌫]
    pub fn emoji_tab_count(&self) -> usize {
        emoji::CATEGORIES.len() + 2
    }

    fn tap_emoji_tabs(&mut self, x: f32) {
        let n = self.emoji_tab_count();
        let i = ((x / self.width * n as f32) as usize).min(n - 1);
        if i == 0 {
            self.mode = Mode::Alphabet;
            self.rebuild();
            self.update_auto_shift();
        } else if i == n - 1 {
            self.backspace();
        } else {
            self.emoji_cat = i - 1;
            self.emoji_scroll = 0.0;
        }
        self.dirty = true;
    }

    // ------------------------------------------------------------------ timers

    /// Time (ms) of the next timer, if any
    pub fn next_wake(&self) -> Option<f64> {
        let mut w: Option<f64> = None;
        for p in &self.pointers {
            if p.done {
                continue;
            }
            if let Some(r) = p.next_repeat {
                w = Some(w.map_or(r, |x: f64| x.min(r)));
            }
            if self.popup.is_none() && !p.cursor_armed && !p.space_drag && !p.gesture {
                if let Some(k) = p.key.and_then(|k| self.keys.get(k)) {
                    if !k.popup.is_empty() || k.kind == KeyKind::Space && self.prefs.languages.len() > 1 {
                        let lp = p.t0 + LONG_PRESS_MS;
                        w = Some(w.map_or(lp, |x: f64| x.min(lp)));
                    }
                }
            }
        }
        w
    }

    pub fn tick(&mut self, t: f64) {
        for i in 0..self.pointers.len() {
            let p = self.pointers[i].clone();
            if p.done {
                continue;
            }
            if let Some(r) = p.next_repeat {
                if t >= r {
                    let n = self.pointers[i].repeats + 1;
                    self.pointers[i].repeats = n;
                    // after a while, delete whole words (accelerating backspace)
                    if n > 18 && self.composing.is_empty() {
                        self.delete_words(1);
                        self.pointers[i].next_repeat = Some(t + REPEAT_MS * 3.0);
                    } else {
                        self.backspace();
                        self.pointers[i].next_repeat = Some(t + REPEAT_MS);
                    }
                    self.dirty = true;
                }
            }
            if self.popup.is_none() && t >= p.t0 + LONG_PRESS_MS && !p.space_drag && !p.cursor_armed {
                if let Some(k) = p.key.filter(|&k| k < self.keys.len()) {
                    if self.keys[k].kind == KeyKind::Space && self.prefs.space_swipe_lang && self.prefs.languages.len() > 1 {
                        // swiping switches languages, so a long press arms cursor moving instead
                        self.pointers[i].cursor_armed = true;
                        if self.prefs.vibrate {
                            self.emit(Cmd::Haptic);
                        }
                        continue;
                    }
                    let moved = (p.x - p.x0).abs() > 10.0 * self.dp;
                    if !moved {
                        self.open_popup(k);
                    }
                }
            }
        }
    }

    fn open_popup(&mut self, k: usize) {
        let key = &self.keys[k];
        let (kind, items) = match key.kind {
            KeyKind::Space if self.prefs.languages.len() > 1 => (
                PopupKind::Languages,
                self.prefs.languages.iter().filter_map(|c| LangDef::by_code(c)).map(|l| l.name.to_string()).collect::<Vec<_>>(),
            ),
            KeyKind::Char(_) if !key.popup.is_empty() => {
                let mut items = key.popup.clone();
                // the comma's long-press also leads to emoji
                if key.func && key.label == "," {
                    items.insert(0, "🙂".into());
                }
                (PopupKind::Chars, items)
            }
            _ => return,
        };
        let cell_w = if kind == PopupKind::Languages { self.width * 0.6 } else { key.rect.w.max(36.0 * self.dp) };
        let cell_h = key.rect.h.min(52.0 * self.dp);
        let cells: Vec<layout::Rect> = if kind == PopupKind::Languages {
            // vertical list above the space bar
            let x = (key.rect.cx() - cell_w / 2.0).clamp(0.0, self.width - cell_w);
            let n = items.len();
            (0..n)
                .map(|i| layout::Rect { x, y: (key.rect.y - (n - i) as f32 * cell_h).max(0.0), w: cell_w, h: cell_h })
                .collect()
        } else {
            let per_row = ((self.width - 8.0 * self.dp) / cell_w).floor().max(1.0) as usize;
            let n = items.len().min(per_row * 2);
            let cols = n.min(per_row);
            let rows = n.div_ceil(cols);
            let total = cols as f32 * cell_w;
            let x0 = (key.rect.cx() - cell_w / 2.0).clamp(0.0, self.width - total).min(key.rect.cx() - cell_w / 2.0);
            let x0 = x0.max(0.0).min(self.width - total);
            (0..n)
                .map(|i| layout::Rect {
                    x: x0 + (i % cols) as f32 * cell_w,
                    y: (key.rect.y - (rows - i / cols) as f32 * cell_h - 4.0 * self.dp).max(0.0),
                    w: cell_w,
                    h: cell_h,
                })
                .collect()
        };
        let items: Vec<String> = items.into_iter().take(cells.len()).collect();
        let sel = if kind == PopupKind::Languages { self.lang_idx } else { 0 };
        if items.len() == 1 && kind == PopupKind::Chars && key.letter {
            // single variant: still show it so the finger can release on it
        }
        self.popup = Some(Popup { kind, items, cells, sel, key: k });
        if self.prefs.vibrate {
            self.emit(Cmd::Haptic);
        }
        self.dirty = true;
    }

    /// For tests and the view: pointers currently held
    pub fn held(&self) -> usize {
        self.pointers.len()
    }

    // ------------------------------------------------------------------ persistence

    pub fn calibration_text(&mut self) -> String {
        if let Some(code) = self.prefs.languages.get(self.lang_idx).cloned() {
            self.calib.insert(code, self.touch.calibration().clone());
        }
        let mut s = String::new();
        for (lang, m) in &self.calib {
            for (c, (dx, dy)) in m {
                s.push_str(&format!("{lang}\t{c}\t{:.3}\t{:.3}\n", dx / self.dp, dy / self.dp));
            }
        }
        s
    }

    pub fn load_calibration(&mut self, text: &str) {
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if let [lang, c, dx, dy] = f[..] {
                if let (Some(c), Ok(dx), Ok(dy)) = (c.chars().next(), dx.parse::<f32>(), dy.parse::<f32>()) {
                    self.calib.entry(lang.to_string()).or_default().insert(c, (dx * self.dp, dy * self.dp));
                }
            }
        }
        if let Some(c) = self.calib.get(self.lang().code) {
            self.touch.set_calibration(c.clone());
        }
    }
}

/// Combining vowel signs and other marks that need a base to be visible
pub fn is_mark(c: char) -> bool {
    // combining marks used by Brahmic, Thai, Khmer, Myanmar, Arabic and Hebrew vowel signs
    matches!(c as u32,
        0x0300..=0x036F | 0x0591..=0x05C7 | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06ED |
        0x0900..=0x0903 | 0x093A..=0x094F | 0x0951..=0x0957 | 0x0962..=0x0963 |
        0x0981..=0x0983 | 0x09BC..=0x09D7 | 0x09E2..=0x09E3 | 0x0A01..=0x0A03 | 0x0A3C..=0x0A51 | 0x0A70..=0x0A71 | 0x0A75 |
        0x0A81..=0x0A83 | 0x0ABC..=0x0ACD | 0x0AE2..=0x0AE3 | 0x0B01..=0x0B03 | 0x0B3C..=0x0B57 | 0x0B62..=0x0B63 |
        0x0B82 | 0x0BBE..=0x0BCD | 0x0BD7 | 0x0C00..=0x0C04 | 0x0C3C..=0x0C56 | 0x0C62..=0x0C63 |
        0x0C81..=0x0C83 | 0x0CBC..=0x0CD6 | 0x0CE2..=0x0CE3 | 0x0D00..=0x0D03 | 0x0D3B..=0x0D57 | 0x0D62..=0x0D63 |
        0x0D81..=0x0D83 | 0x0DCA..=0x0DDF | 0x0DF2..=0x0DF3 | 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E |
        0x0EB1 | 0x0EB4..=0x0EBC | 0x0EC8..=0x0ECD | 0x102B..=0x103E | 0x1056..=0x1059 | 0x17B4..=0x17D3 | 0x135D..=0x135F)
}

/// Words before the cursor, nearest first, and whether a sentence starts here
pub fn context_of(before: &str) -> Context {
    let t = before.trim_end_matches([' ', '\u{a0}']);
    if t.is_empty() || t.ends_with('\n') {
        return Context { prev: Vec::new(), sentence_start: true };
    }
    let words: Vec<&str> = t.split_whitespace().rev().take(2).collect();
    let mut prev = Vec::new();
    let mut sentence_start = false;
    for (i, w) in words.iter().enumerate() {
        if w.ends_with(SENTENCE_END) {
            if i == 0 {
                sentence_start = true;
            }
            if i == 1 {
                sentence_start = true;
            }
            break;
        }
        let clean: String = w.trim_matches(|c: char| !c.is_alphanumeric() && !is_mark(c)).to_string();
        if clean.is_empty() || w.ends_with([',', ';', ':']) && i == 1 {
            break;
        }
        prev.push(clean);
        if w.ends_with([',', ';', ':']) {
            break;
        }
    }
    if words.len() < 2 && !sentence_start && prev.len() == 1 && t.split_whitespace().count() == 1 {
        sentence_start = true;
    }
    Context { prev, sentence_start }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::cjk::ConvDict;
    use crate::engine::Model;

    fn kb(langs: &[&str]) -> Keyboard {
        let mut p = Prefs::defaults(&[]);
        p.languages = langs.iter().map(|s| s.to_string()).collect();
        p.vibrate = false;
        let mut k = Keyboard::new(p);
        k.measure(1080.0, 2.75, 0.0);
        k.start_input(Editor::from_android(0x1 | 0x4000, 6), false);
        let cmds = k.take_commands();
        load(&mut k, cmds);
        k
    }

    /// Loads the assets a command list asks for
    fn load(k: &mut Keyboard, cmds: Vec<Cmd>) {
        for c in cmds {
            if let Cmd::LoadModel(path) = c {
                let file = format!("{}/../app/src/main/assets/{path}", env!("CARGO_MANIFEST_DIR"));
                let Ok(b) = std::fs::read(file) else { continue };
                if path.ends_with(".rcd") {
                    k.conv_loaded(&path, Arc::new(ConvDict::decode(&b).unwrap()));
                } else {
                    k.model_loaded(&path, Arc::new(Model::from_bytes(&b).unwrap()));
                }
            }
        }
    }

    /// Taps the center of the key labelled `label` (case-insensitive)
    fn tap(k: &mut Keyboard, label: &str, t: &mut f64) {
        let key = k
            .keys
            .iter()
            .find(|key| key.label.to_lowercase() == label.to_lowercase())
            .unwrap_or_else(|| panic!("no key {label}"))
            .clone();
        let (x, y) = (key.rect.cx(), key.rect.cy());
        k.touch_down(1, x, y, *t);
        k.touch_up(1, x, y, *t + 40.0);
        *t += 120.0;
    }

    fn tap_kind(k: &mut Keyboard, kind: KeyKind, t: &mut f64) {
        let key = k.keys.iter().find(|key| key.kind == kind).unwrap().clone();
        k.touch_down(1, key.rect.cx(), key.rect.cy(), *t);
        k.touch_up(1, key.rect.cx(), key.rect.cy(), *t + 40.0);
        *t += 120.0;
    }

    /// Applies commands to a simulated text field
    fn apply(text: &mut String, composing: &mut Option<(usize, String)>, cmds: Vec<Cmd>) {
        for c in cmds {
            match c {
                Cmd::Composing(s) => {
                    let start = composing.as_ref().map_or(text.len(), |c| c.0);
                    text.truncate(start);
                    text.push_str(&s);
                    *composing = if s.is_empty() { None } else { Some((start, s)) };
                }
                Cmd::Commit(s) => {
                    if let Some((start, _)) = composing.take() {
                        text.truncate(start);
                    }
                    text.push_str(&s);
                }
                Cmd::DeleteBefore(s) => {
                    assert!(text.ends_with(&s), "{text:?} does not end with {s:?}");
                    let n = text.len() - s.len();
                    text.truncate(n);
                }
                Cmd::DelKey => {
                    text.pop();
                }
                _ => {}
            }
        }
    }

    fn type_str(k: &mut Keyboard, s: &str, text: &mut String, comp: &mut Option<(usize, String)>, t: &mut f64) {
        for ch in s.chars() {
            match ch {
                ' ' => tap_kind(k, KeyKind::Space, t),
                '<' => tap_kind(k, KeyKind::Backspace, t),
                c => tap(k, &c.to_string(), t),
            }
            let cmds = k.take_commands();
            apply(text, comp, cmds);
            if comp.is_none() {
                let before = text.clone();
                k.set_context(&before, false);
            }
        }
    }

    #[test]
    fn autocorrects_on_space_and_undoes_on_backspace() {
        let mut k = kb(&["ru"]);
        let (mut text, mut comp, mut t) = (String::new(), None, 0.0);
        type_str(&mut k, "пирвет ", &mut text, &mut comp, &mut t);
        assert_eq!(text, "Привет ");
        type_str(&mut k, "<", &mut text, &mut comp, &mut t);
        assert_eq!(text, "Пирвет");
        type_str(&mut k, " ", &mut text, &mut comp, &mut t);
        assert_eq!(text, "Пирвет ", "the user's word is kept after undo");
    }

    #[test]
    fn smart_punctuation_and_double_space() {
        let mut k = kb(&["ru"]);
        let (mut text, mut comp, mut t) = (String::new(), None, 0.0);
        type_str(&mut k, "как дела ", &mut text, &mut comp, &mut t);
        tap_kind(&mut k, KeyKind::Char(",".into()), &mut t);
        apply(&mut text, &mut comp, k.take_commands());
        assert_eq!(text, "Как дела, ");
        let mut k = kb(&["ru"]);
        let (mut text, mut comp, mut t) = (String::new(), None, 0.0);
        type_str(&mut k, "да  ", &mut text, &mut comp, &mut t);
        assert_eq!(text, "Да. ");
        assert_eq!(k.shift, ShiftState::On, "capital letter after the period");
    }

    #[test]
    fn predictions_after_word() {
        let mut k = kb(&["ru"]);
        let (mut text, mut comp, mut t) = (String::new(), None, 0.0);
        type_str(&mut k, "спасибо ", &mut text, &mut comp, &mut t);
        assert!(matches!(k.strip_items()[0], StripItem::Word { .. }), "{:?}", k.strip_items());
    }

    #[test]
    fn long_press_gives_variant() {
        let mut k = kb(&["ru"]);
        let key = k.keys.iter().find(|x| x.label.to_lowercase() == "е").unwrap().clone();
        k.touch_down(1, key.rect.cx(), key.rect.cy(), 0.0);
        k.tick(LONG_PRESS_MS + 1.0);
        let pop = k.popup.clone().expect("popup");
        let idx = pop.items.iter().position(|s| s.to_lowercase() == "ё").unwrap();
        let cell = pop.cells[idx].clone();
        k.touch_move(1, cell.cx(), cell.cy(), 500.0);
        k.touch_up(1, cell.cx(), cell.cy(), 520.0);
        let cmds = k.take_commands();
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Composing(s) if s.to_lowercase() == "ё")), "{cmds:?}");
    }

    #[test]
    fn backspace_repeats_and_swipe_deletes_words() {
        let mut k = kb(&["en"]);
        k.set_context("one two three", false);
        let key = k.keys.iter().find(|x| x.kind == KeyKind::Backspace).unwrap().clone();
        k.touch_down(1, key.rect.cx(), key.rect.cy(), 0.0);
        k.tick(REPEAT_START_MS + 1.0);
        k.touch_up(1, key.rect.cx(), key.rect.cy(), REPEAT_START_MS + 10.0);
        let dels = k.take_commands().iter().filter(|c| **c == Cmd::DelKey).count();
        assert_eq!(dels, 2);
        k.set_context("one two three", false);
        k.touch_down(1, key.rect.cx(), key.rect.cy(), 1000.0);
        k.touch_move(1, key.rect.cx() - 60.0 * k.dp, key.rect.cy(), 1050.0);
        k.touch_up(1, key.rect.cx() - 60.0 * k.dp, key.rect.cy(), 1100.0);
        let cmds = k.take_commands();
        // the press itself already deleted one character, the swipe then takes two words
        assert!(cmds.contains(&Cmd::DeleteBefore("two thre".into())), "{cmds:?}");
    }

    #[test]
    fn space_drag_moves_cursor() {
        let mut k = kb(&["en"]);
        let key = k.keys.iter().find(|x| x.kind == KeyKind::Space).unwrap().clone();
        k.touch_down(1, key.rect.cx(), key.rect.cy(), 0.0);
        k.touch_move(1, key.rect.cx() - 60.0 * k.dp, key.rect.cy(), 50.0);
        k.touch_up(1, key.rect.cx() - 60.0 * k.dp, key.rect.cy(), 80.0);
        let moved: i32 = k.take_commands().iter().map(|c| if let Cmd::Cursor(n) = c { *n } else { 0 }).sum();
        assert!(moved <= -4, "{moved}");
    }

    #[test]
    fn korean_and_japanese_compose() {
        let mut k = kb(&["ko"]);
        let (mut text, mut comp, mut t) = (String::new(), None, 0.0);
        type_str(&mut k, "ㅎㅏㄴ ", &mut text, &mut comp, &mut t);
        assert_eq!(text, "한 ");
    }

    #[test]
    fn japanese_converts_with_space_and_confirms_with_enter() {
        let mut k = kb(&["ja"]);
        let (mut text, mut comp, mut t) = (String::new(), None, 0.0);
        type_str(&mut k, "nihon", &mut text, &mut comp, &mut t);
        assert_eq!(text, "にほn", "a trailing n may still become な");
        type_str(&mut k, " ", &mut text, &mut comp, &mut t);
        assert_eq!(text, "日本", "space shows the first conversion");
        tap_kind(&mut k, KeyKind::Enter, &mut t);
        apply(&mut text, &mut comp, k.take_commands());
        assert_eq!(text, "日本");
        assert!(comp.is_none());
        type_str(&mut k, "tabeta ", &mut text, &mut comp, &mut t);
        tap_kind(&mut k, KeyKind::Enter, &mut t);
        apply(&mut text, &mut comp, k.take_commands());
        assert_eq!(text, "日本食べた");
    }

    #[test]
    fn chinese_pinyin_zhuyin_and_cantonese() {
        let mut k = kb(&["zh-Hans"]);
        let (mut text, mut comp, mut t) = (String::new(), None, 0.0);
        type_str(&mut k, "nihao ", &mut text, &mut comp, &mut t);
        assert_eq!(text, "你好");
        tap_kind(&mut k, KeyKind::Char("，".into()), &mut t);
        apply(&mut text, &mut comp, k.take_commands());
        assert_eq!(text, "你好，");
        type_str(&mut k, "zhongguoren", &mut text, &mut comp, &mut t);
        assert!(matches!(&k.strip_items()[0], StripItem::Cand { text, .. } if text == "中国人"), "{:?}", k.strip_items());
        // picking a shorter candidate keeps the rest of the input
        let idx = k.cands.iter().position(|c| c.text == "中国").unwrap();
        k.cjk_pick(idx);
        apply(&mut text, &mut comp, k.take_commands());
        assert_eq!(text, "你好，中国ren");
        type_str(&mut k, " ", &mut text, &mut comp, &mut t);
        assert_eq!(text, "你好，中国人");

        let mut k = kb(&["zh-TW"]);
        let (mut text, mut comp, mut t) = (String::new(), None, 0.0);
        type_str(&mut k, "ㄋㄧˇㄏㄠˇ ", &mut text, &mut comp, &mut t);
        assert_eq!(text, "你好");

        let mut k = kb(&["yue"]);
        let (mut text, mut comp, mut t) = (String::new(), None, 0.0);
        type_str(&mut k, "mgoi ", &mut text, &mut comp, &mut t);
        assert_eq!(text, "唔該");
    }

    #[test]
    fn vietnamese_telex_on_the_keyboard() {
        let mut k = kb(&["vi"]);
        k.prefs.autocorrect = false;
        let (mut text, mut comp, mut t) = (String::new(), None, 0.0);
        type_str(&mut k, "tieengs vieetj ", &mut text, &mut comp, &mut t);
        assert_eq!(text, "Tiếng việt ");
    }

    #[test]
    fn conversion_is_off_in_password_fields() {
        let mut k = kb(&["zh-Hans"]);
        k.start_input(Editor::from_android(0x81, 6), false);
        let (mut text, mut comp, mut t) = (String::new(), None, 0.0);
        type_str(&mut k, "abc", &mut text, &mut comp, &mut t);
        assert!(k.cands.is_empty());
        assert_eq!(text, "abc");
    }

    #[test]
    fn context_parsing() {
        assert_eq!(context_of("").sentence_start, true);
        let c = context_of("Я иду в ");
        assert_eq!(c.prev, ["в", "иду"]);
        let c = context_of("Привет. Как ");
        assert_eq!(c.prev, ["Как"]);
        assert!(c.sentence_start);
        assert!(context_of("Ok. ").sentence_start);
    }

    #[test]
    fn commands_encode() {
        let s = Cmd::encode(&[Cmd::Commit("a b".into()), Cmd::Cursor(-3), Cmd::Haptic]);
        assert_eq!(s, "T\u{1F}a b\u{1E}M\u{1F}-3\u{1E}H\u{1F}\u{1E}");
    }

    /// Slides one finger through the centers of the keys of `word`
    fn swipe_word(k: &mut Keyboard, word: &str, t: &mut f64) {
        let pts: Vec<(f32, f32)> = word
            .chars()
            .map(|c| {
                let key = k.keys.iter().find(|key| key.label.to_lowercase() == c.to_string()).unwrap();
                (key.rect.cx(), key.rect.cy())
            })
            .collect();
        k.touch_down(1, pts[0].0, pts[0].1, *t);
        for w in pts.windows(2) {
            for s in 1..=8 {
                let f = s as f32 / 8.0;
                *t += 8.0;
                k.touch_move(1, w[0].0 + (w[1].0 - w[0].0) * f, w[0].1 + (w[1].1 - w[0].1) * f, *t);
            }
        }
        let last = *pts.last().unwrap();
        k.touch_up(1, last.0, last.1, *t + 8.0);
        *t += 200.0;
    }

    #[test]
    fn swipe_types_words_and_backspace_removes_them() {
        let mut k = kb(&["en"]);
        let (mut text, mut comp, mut t) = (String::new(), None, 0.0);
        swipe_word(&mut k, "hello", &mut t);
        assert!(k.trail.is_empty());
        apply(&mut text, &mut comp, k.take_commands());
        assert_eq!(text, "Hello ", "autocap applies to the first swiped word");
        swipe_word(&mut k, "world", &mut t);
        apply(&mut text, &mut comp, k.take_commands());
        assert_eq!(text, "Hello world ");
        // space after a swiped word adds nothing: the word already ends with one
        tap_kind(&mut k, KeyKind::Space, &mut t);
        apply(&mut text, &mut comp, k.take_commands());
        assert_eq!(text, "Hello world ");
        swipe_word(&mut k, "there", &mut t);
        apply(&mut text, &mut comp, k.take_commands());
        tap_kind(&mut k, KeyKind::Backspace, &mut t);
        apply(&mut text, &mut comp, k.take_commands());
        assert_eq!(text, "Hello world ", "backspace removes the whole swiped word");
    }

    #[test]
    fn swipe_alternative_replaces_the_word() {
        let mut k = kb(&["en"]);
        let (mut text, mut comp, mut t) = (String::new(), None, 0.0);
        type_str(&mut k, "i ", &mut text, &mut comp, &mut t);
        swipe_word(&mut k, "too", &mut t);
        apply(&mut text, &mut comp, k.take_commands());
        let items = k.strip_items();
        assert!(items.len() >= 2, "{items:?}");
        let alt = items.iter().position(|i| matches!(i, StripItem::Word { primary: false, .. })).unwrap();
        let StripItem::Word { text: alt_text, .. } = items[alt].clone() else { unreachable!() };
        k.pick_strip(alt);
        apply(&mut text, &mut comp, k.take_commands());
        assert_eq!(text, format!("I {alt_text} "));
    }

    #[test]
    fn swipe_on_space_switches_language_and_long_press_moves_cursor() {
        let mut k = kb(&["ru", "en"]);
        let key = k.keys.iter().find(|x| x.kind == KeyKind::Space).unwrap().clone();
        let (x, y) = (key.rect.cx(), key.rect.cy());
        k.touch_down(1, x, y, 0.0);
        k.touch_move(1, x + 60.0 * k.dp, y, 60.0);
        k.touch_up(1, x + 60.0 * k.dp, y, 90.0);
        assert_eq!(k.lang().code, "en");
        assert!(!k.take_commands().iter().any(|c| matches!(c, Cmd::Commit(_))), "no space typed");
        // long press, then drag: the cursor moves and the language stays
        k.touch_down(1, x, y, 1000.0);
        k.tick(1000.0 + LONG_PRESS_MS + 10.0);
        k.touch_move(1, x - 60.0 * k.dp, y, 1500.0);
        k.touch_up(1, x - 60.0 * k.dp, y, 1550.0);
        assert_eq!(k.lang().code, "en");
        let moved: i32 = k.take_commands().iter().map(|c| if let Cmd::Cursor(n) = c { *n } else { 0 }).sum();
        assert!(moved <= -4, "{moved}");
        // with the setting off, a drag moves the cursor right away
        let mut p = k.prefs.clone();
        p.space_swipe_lang = false;
        k.apply_prefs(p);
        k.touch_down(1, x, y, 3000.0);
        k.touch_move(1, x + 60.0 * k.dp, y, 3050.0);
        k.touch_up(1, x + 60.0 * k.dp, y, 3080.0);
        assert_eq!(k.lang().code, "en");
        let moved: i32 = k.take_commands().iter().map(|c| if let Cmd::Cursor(n) = c { *n } else { 0 }).sum();
        assert!(moved >= 4, "{moved}");
    }
}
