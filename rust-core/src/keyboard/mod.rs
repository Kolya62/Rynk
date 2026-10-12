pub mod clipboard_panel;
pub mod context;
pub mod hangul;
pub mod key;
pub mod layout;
pub mod state;
pub mod toolbar;
pub mod touch;

use crate::prediction::PredictionService;
use hangul::HangulComposer;
use key::{Key, KeyAction, KeyboardMode};
use layout::{LayoutBuilder, LayoutMetrics};
use state::{HapticFeedbackType, KeyboardOutputEvent, KeyboardState, Language};
use touch::{TouchAction, TouchResult, TouchTracker};

/// Readings of a glide gesture kept for the suggestion bar
const GESTURE_CANDIDATES: usize = 5;

pub struct KeyboardEngine {
    pub state: KeyboardState,
    pub metrics: LayoutMetrics,
    pub keys: Vec<Key>,
    pub touch_tracker: TouchTracker,
    pub active_popup_key_id: Option<u32>,
    pub last_interaction_time_ms: u64,
    pub last_space_tap_time_ms: u64,
    pub last_shift_tap_time_ms: u64,
    pub prediction: PredictionService,
    pub autocorrect_enabled: bool,
    pub popup_enabled: bool,
    pub cached_suggestions: Vec<String>,
    pub suggestions_dirty: bool,
    pub prediction_version: u64,
    pub cached_suggestions_version: u64,
    pub enabled_languages: Vec<Language>,
    pub hangul_composer: HangulComposer,
    /// Editor action performed by the Enter key (`EditorInfo.IME_ACTION_*`), 0 = plain Enter / newline
    pub enter_action: i32,
    /// Letter key centers of the current layout, for the touch-aware spelling decoder
    pub key_geometry: crate::prediction::decoder::KeyGeometry,
    pub autocorrect_strength: crate::prediction::correction::AutocorrectStrength,
    pub settings: state::EngineSettings,
    pub layout_options: layout::LayoutOptions,
    /// Show the microphone in the toolbar (a voice input method is available)
    pub voice_key: bool,
    /// Where each pointer went down
    touch_down_points: std::collections::HashMap<i32, (f32, f32)>,
    /// Touch position of the key being executed, if it came from a tap
    pending_touch: Option<(f32, f32)>,
    /// Other readings of the last glide gesture, shown around it in the suggestion bar
    gesture_alternatives: Vec<String>,
}

impl KeyboardEngine {
    pub fn new(width: f32, height: f32, density: f32) -> Self {
        let metrics = LayoutMetrics::new(width, height, density);
        let state = KeyboardState::default();
        let keys =
            LayoutBuilder::build_layout(state.mode, state.language, state.shift_state, &metrics);
        let prediction = PredictionService::new();

        let mut engine = Self {
            state,
            metrics,
            keys,
            touch_tracker: TouchTracker::with_density(density),
            active_popup_key_id: None,
            last_interaction_time_ms: 0,
            last_space_tap_time_ms: 0,
            last_shift_tap_time_ms: 0,
            prediction,
            autocorrect_enabled: true,
            popup_enabled: true,
            cached_suggestions: Vec::new(),
            suggestions_dirty: true,
            prediction_version: 1,
            cached_suggestions_version: 0,
            enabled_languages: vec![Language::Russian, Language::English],
            hangul_composer: HangulComposer::new(),
            enter_action: 0,
            key_geometry: Default::default(),
            autocorrect_strength: Default::default(),
            settings: Default::default(),
            layout_options: Default::default(),
            voice_key: false,
            touch_down_points: std::collections::HashMap::new(),
            pending_touch: None,
            gesture_alternatives: Vec::new(),
        };
        engine.key_geometry = Self::geometry_of(&engine.keys);
        engine
    }

    /// Letter key centers (lowercase) and typical key size of a layout
    fn geometry_of(keys: &[Key]) -> crate::prediction::decoder::KeyGeometry {
        let mut g = crate::prediction::decoder::KeyGeometry::default();
        let mut widths = Vec::new();
        for key in keys {
            if let KeyAction::Character(c) = key.action {
                if c.is_alphabetic() {
                    for l in c.to_lowercase() {
                        g.centers.insert(l, key.center());
                    }
                    widths.push((key.width, key.height));
                }
            }
        }
        widths.sort_by(|a, b| a.0.total_cmp(&b.0));
        if let Some(&(w, h)) = widths.get(widths.len() / 2) {
            g.key_width = w;
            g.key_height = h;
        }
        g
    }

    pub fn resize(&mut self, width: f32, height: f32, density: f32) {
        self.metrics = LayoutMetrics::new(width, height, density);
        self.touch_tracker.update_density(density);
        self.rebuild_layout();
    }

    /// Number row, one-handed mode and the toolbar microphone (settings screen).
    pub fn set_layout_options(&mut self, number_row: bool, one_handed: layout::OneHanded, voice_key: bool) {
        let options = layout::LayoutOptions { number_row, one_handed, ..self.layout_options };
        self.voice_key = voice_key;
        if options != self.layout_options {
            self.layout_options = options;
            self.rebuild_layout();
        }
    }

    fn execute_edit_action(&mut self, action: key::EditAction) {
        use key::EditAction as E;
        use state::edit_command as cmd;
        // Android KeyEvent codes and META_SHIFT_ON | META_SHIFT_LEFT_ON
        const DPAD_UP: i32 = 19;
        const DPAD_DOWN: i32 = 20;
        const DPAD_LEFT: i32 = 21;
        const DPAD_RIGHT: i32 = 22;
        const MOVE_HOME: i32 = 122;
        const MOVE_END: i32 = 123;
        const META_SHIFT: i32 = 0x41;

        let meta = if self.layout_options.edit_selecting { META_SHIFT } else { 0 };
        let key = |code| KeyboardOutputEvent::KeyWithMeta { code, meta };
        let event = match action {
            E::Left => Some(key(DPAD_LEFT)),
            E::Right => Some(key(DPAD_RIGHT)),
            E::Up => Some(key(DPAD_UP)),
            E::Down => Some(key(DPAD_DOWN)),
            E::Home => Some(key(MOVE_HOME)),
            E::End => Some(key(MOVE_END)),
            E::SelectAll => Some(KeyboardOutputEvent::EditorCommand(cmd::SELECT_ALL)),
            E::Copy => Some(KeyboardOutputEvent::EditorCommand(cmd::COPY)),
            E::Cut => Some(KeyboardOutputEvent::EditorCommand(cmd::CUT)),
            E::Paste => Some(KeyboardOutputEvent::EditorCommand(cmd::PASTE)),
            E::Undo => Some(KeyboardOutputEvent::EditorCommand(cmd::UNDO)),
            E::Redo => Some(KeyboardOutputEvent::EditorCommand(cmd::REDO)),
            E::SelectMode => {
                self.layout_options.edit_selecting = !self.layout_options.edit_selecting;
                self.rebuild_layout();
                None
            }
            E::Close => {
                self.layout_options.edit_selecting = false;
                self.set_mode(KeyboardMode::Alphabet);
                None
            }
        };
        // Whatever the panel did, the engine's idea of the word being typed is stale now
        self.state.composing_text.clear();
        self.state.composing_touches.clear();
        if let Some(e) = event {
            self.state.push_event(e);
        }
        self.state.push_event(KeyboardOutputEvent::PerformHaptic(HapticFeedbackType::KeyTick));
    }

    /// Commits the gestured word as typed and adds the space after it.
    fn finish_gesture_word(&mut self) {
        let word = std::mem::take(&mut self.state.composing_text);
        self.state.composing_touches.clear();
        self.state.push_event(KeyboardOutputEvent::CommitText(" ".to_string()));
        if self.state.field_mode.allows_learning() {
            if !self.state.last_committed_word.is_empty() {
                self.prediction.learn_bigram(&self.state.last_committed_word, &word);
            }
            self.prediction.learn_word(&word, self.state.language);
        }
        self.state.commit_context_word(word);
        self.state.last_char_was_space = true;
    }

    /// Decodes a glide path into a word and enters it, with a separating space if needed.
    pub fn commit_gesture(&mut self, path: &[(f32, f32)]) {
        self.active_popup_key_id = None;
        for key in self.keys.iter_mut() {
            key.is_pressed = false;
        }
        if self.state.gesture_word && !self.state.composing_text.is_empty() {
            self.state.gesture_word = false;
            self.finish_gesture_word();
        } else if !self.state.composing_text.is_empty() {
            // Letters tapped just before: keep them as their own word
            let word = std::mem::take(&mut self.state.composing_text);
            self.state.composing_touches.clear();
            self.state.push_event(KeyboardOutputEvent::CommitText(" ".to_string()));
            self.state.commit_context_word(word);
        } else if !self.state.last_char_was_space && !self.state.last_committed_word.is_empty() {
            self.state.push_event(KeyboardOutputEvent::CommitText(" ".to_string()));
        }

        let candidates = self.prediction.dictionary.decode_gesture(
            self.state.language,
            &self.state.word_context(),
            path,
            &self.key_geometry,
            GESTURE_CANDIDATES,
        );
        let Some(best) = candidates.first() else {
            self.state.push_event(KeyboardOutputEvent::PerformHaptic(HapticFeedbackType::KeyTick));
            return;
        };
        let shift = self.state.shift_state;
        let cased = |w: &str| -> String {
            match shift {
                state::ShiftState::CapsLock => w.to_uppercase(),
                state::ShiftState::Shifted => {
                    let mut c = w.chars();
                    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
                }
                state::ShiftState::Off => w.to_string(),
            }
        };
        let word = cased(&best.word);
        self.gesture_alternatives = candidates.iter().skip(1).map(|c| cased(&c.word)).collect();
        self.state.push_event(KeyboardOutputEvent::CommitText(word.clone()));
        self.state.composing_text = word;
        self.state.composing_touches.clear();
        self.state.gesture_word = true;
        self.state.last_char_was_space = false;
        self.state.last_autocorrect_original = None;
        self.state.last_autocorrect_replacement = None;
        if self.state.shift_state == state::ShiftState::Shifted {
            self.state.shift_state = state::ShiftState::Off;
            self.rebuild_layout();
        }
        self.suggestions_dirty = true;
        self.state.push_event(KeyboardOutputEvent::PerformHaptic(HapticFeedbackType::KeyClick));
    }

    /// Switches one-handed mode from the keyboard itself; the app persists it.
    pub fn set_one_handed(&mut self, mode: layout::OneHanded) {
        if self.layout_options.one_handed != mode {
            self.layout_options.one_handed = mode;
            self.rebuild_layout();
            self.state.push_event(KeyboardOutputEvent::OneHandedChanged(mode.to_id()));
        }
    }

    /// Applies the settings screen: behavior flags, double-space action and autocorrect level
    /// (0 off, 1 mild, 2 normal, 3 aggressive).
    pub fn apply_settings(&mut self, flags: i32, double_space: i32, autocorrect_level: i32) {
        self.settings = state::EngineSettings::from_flags(flags, double_space);
        self.touch_tracker.space_swipe_switches_language = self.settings.space_swipe_switches_language;
        self.rebuild_layout();
        self.autocorrect_enabled = autocorrect_level > 0;
        self.autocorrect_strength = crate::prediction::correction::AutocorrectStrength {
            split_words: flags & state::EngineSettings::SPLIT_WORDS != 0,
            ..crate::prediction::correction::AutocorrectStrength::from_level(autocorrect_level)
        };
        self.suggestions_dirty = true;
    }

    /// Re-anchors the engine on the editor's text (input start, cursor moved by the user):
    /// adopts the word directly before the cursor as the word being typed and the preceding
    /// words as context. `caps` is the editor's cursor caps mode (auto-capitalization request).
    pub fn set_editor_context(&mut self, text_before_cursor: &str, caps: bool) {
        self.hangul_composer.reset();
        self.state.last_autocorrect_original = None;
        self.state.last_autocorrect_replacement = None;
        self.state.rejected_autocorrect_word = None;
        self.last_space_tap_time_ms = 0;

        if self.state.field_mode.allows_suggestions() {
            let ctx = context::EditorContext::parse(text_before_cursor);
            self.state.composing_text = ctx.partial_word;
            self.state.clear_context(ctx.sentence_start);
            // Oldest first, so the sentence-start marker ages out correctly
            for w in [ctx.prev2, ctx.prev1].into_iter().flatten() {
                self.state.commit_context_word(w);
            }
            self.state.last_char_was_space = ctx.ends_with_space;
        } else {
            // Never keep text from password-like fields
            self.state.composing_text.clear();
            self.state.clear_context(false);
            self.state.last_char_was_space = false;
        }

        if self.state.shift_state != state::ShiftState::CapsLock && self.state.mode == KeyboardMode::Alphabet {
            let wanted = if caps && self.settings.auto_capitalization {
                state::ShiftState::Shifted
            } else {
                state::ShiftState::Off
            };
            if wanted != self.state.shift_state {
                self.state.shift_state = wanted;
                self.rebuild_layout();
            }
        }
        self.prediction_version = self.prediction_version.wrapping_add(1);
        self.suggestions_dirty = true;
    }

    pub fn set_input_field_mode(&mut self, mode: state::InputFieldMode) {
        if self.state.field_mode != mode {
            self.state.field_mode = mode;
            if mode.is_sensitive() {
                self.cached_suggestions.clear();
                self.suggestions_dirty = false;
                self.state.composing_text.clear();
                self.state.clear_context(false);
                self.state.clipboard_preview = None;
                self.hangul_composer.reset();
            } else {
                self.suggestions_dirty = true;
            }

            if mode == state::InputFieldMode::Number
                || mode == state::InputFieldMode::Phone
                || mode == state::InputFieldMode::NumberPassword
            {
                if self.state.mode != KeyboardMode::Numbers {
                    self.set_mode(KeyboardMode::Numbers);
                }
            } else if self.state.mode == KeyboardMode::Numbers {
                self.set_mode(KeyboardMode::Alphabet);
            }
            self.touch_tracker.glide_enabled = self.settings.glide_typing
                && self.state.mode == KeyboardMode::Alphabet
                && self.state.field_mode.allows_suggestions();
        }
    }

    pub fn rebuild_layout(&mut self) {
        self.keys = LayoutBuilder::build_layout_with(
            self.state.mode,
            self.state.language,
            self.state.shift_state,
            &self.metrics,
            &self.layout_options,
        );
        if self.state.mode == KeyboardMode::Alphabet {
            self.key_geometry = Self::geometry_of(&self.keys);
        }
        self.touch_tracker.glide_enabled = self.settings.glide_typing
            && self.state.mode == KeyboardMode::Alphabet
            && self.state.field_mode.allows_suggestions();
    }

    pub fn update_suggestions(&mut self) {
        if !self.state.field_mode.allows_suggestions() {
            self.cached_suggestions.clear();
            self.suggestions_dirty = false;
            self.cached_suggestions_version = self.prediction_version;
            return;
        }

        if self.suggestions_dirty || self.cached_suggestions_version != self.prediction_version {
            // Sentence-start predictions would hide the clipboard chip; the chip wins
            let idle_with_clipboard = self.state.composing_text.is_empty()
                && self.state.last_committed_word.is_empty()
                && self.state.clipboard_preview.is_some();
            let predictions_off = self.state.composing_text.is_empty() && !self.settings.next_word_predictions;
            if self.state.gesture_word && !self.state.composing_text.is_empty() {
                // Gestured word in the center, other readings around it
                let mut chips = Vec::with_capacity(3);
                chips.extend(self.gesture_alternatives.first().cloned());
                chips.push(self.state.composing_text.clone());
                chips.extend(self.gesture_alternatives.get(1).cloned());
                self.cached_suggestions = chips;
                self.suggestions_dirty = false;
                self.cached_suggestions_version = self.prediction_version;
                return;
            }
            self.cached_suggestions = if idle_with_clipboard || predictions_off {
                Vec::new()
            } else {
                self.prediction.get_suggestions_typed(
                    &self.state.composing_text,
                    &self.state.composing_touches,
                    Some(&self.key_geometry),
                    self.autocorrect_strength,
                    &self.state.word_context(),
                    self.state.language,
                )
            };
            // Next-word predictions follow the shift state (sentence start, manual shift)
            if self.state.composing_text.is_empty() && self.state.shift_state.is_uppercase() {
                let all_caps = self.state.shift_state == state::ShiftState::CapsLock;
                for w in &mut self.cached_suggestions {
                    *w = if all_caps {
                        w.to_uppercase()
                    } else {
                        let mut chars = w.chars();
                        chars
                            .next()
                            .map(|f| f.to_uppercase().chain(chars).collect())
                            .unwrap_or_default()
                    };
                }
            }
            self.suggestions_dirty = false;
            self.cached_suggestions_version = self.prediction_version;
        }
    }

    pub fn get_or_update_suggestions(&mut self) -> &[String] {
        self.update_suggestions();
        &self.cached_suggestions
    }

    pub fn set_enabled_languages(&mut self, langs: Vec<Language>) {
        if !langs.is_empty() {
            self.enabled_languages = langs;
            self.prediction
                .dictionary
                .ensure_languages_loaded(&self.enabled_languages);
            if !self.enabled_languages.contains(&self.state.language) {
                self.set_language(self.enabled_languages[0]);
            }
        }
    }

    pub fn next_language(&mut self) {
        if self.enabled_languages.is_empty() {
            self.enabled_languages = vec![Language::Russian, Language::English];
        }
        let current_idx = self
            .enabled_languages
            .iter()
            .position(|&l| l == self.state.language)
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % self.enabled_languages.len();
        self.set_language(self.enabled_languages[next_idx]);
        self.state.push_event(KeyboardOutputEvent::PerformHaptic(
            HapticFeedbackType::KeyTick,
        ));
    }

    pub fn prev_language(&mut self) {
        if self.enabled_languages.is_empty() {
            self.enabled_languages = vec![Language::Russian, Language::English];
        }
        let current_idx = self
            .enabled_languages
            .iter()
            .position(|&l| l == self.state.language)
            .unwrap_or(0);
        let prev_idx = if current_idx == 0 {
            self.enabled_languages.len() - 1
        } else {
            current_idx - 1
        };
        self.set_language(self.enabled_languages[prev_idx]);
        self.state.push_event(KeyboardOutputEvent::PerformHaptic(
            HapticFeedbackType::KeyTick,
        ));
    }

    pub fn set_language(&mut self, lang: Language) {
        if self.state.language != lang {
            self.state.language = lang;
            self.prediction.dictionary.ensure_language_loaded(lang);
            self.suggestions_dirty = true;
            self.rebuild_layout();
        }
    }

    pub fn toggle_language(&mut self) {
        self.next_language();
    }

    pub fn set_mode(&mut self, mode: KeyboardMode) {
        if self.state.mode != mode {
            self.state.mode = mode;
            self.rebuild_layout();
            self.state.push_event(KeyboardOutputEvent::PerformHaptic(
                HapticFeedbackType::KeyTick,
            ));
        }
    }

    pub fn on_touch(&mut self, action: TouchAction, pointer_id: i32, x: f32, y: f32, time_ms: u64) {
        self.last_interaction_time_ms = time_ms;
        match action {
            TouchAction::Down => {
                self.touch_down_points.insert(pointer_id, (x, y));
            }
            TouchAction::Cancel => self.touch_down_points.clear(),
            _ => {}
        }
        if action == TouchAction::Cancel {
            self.active_popup_key_id = None;
            for key in self.keys.iter_mut() {
                key.is_pressed = false;
            }
        }

        let touch_res = self
            .touch_tracker
            .handle_touch(action, pointer_id, x, y, time_ms, &self.keys);

        match touch_res {
            TouchResult::KeyPress { key_id, haptic } => {
                self.active_popup_key_id = Some(key_id);
                if let Some(key) = self.keys.iter_mut().find(|k| k.id == key_id) {
                    key.is_pressed = true;
                    key.press_time_ms = time_ms;
                }
                self.state
                    .push_event(KeyboardOutputEvent::PerformHaptic(haptic));
            }

            TouchResult::KeyRelease { key_id, action } => {
                self.active_popup_key_id = None;
                if let Some(key) = self.keys.iter_mut().find(|k| k.id == key_id) {
                    key.is_pressed = false;
                }
                self.pending_touch = self.touch_down_points.remove(&pointer_id);
                self.execute_key_action(action);
                self.pending_touch = None;
            }

            TouchResult::AlternateKeyRelease { character } => {
                self.active_popup_key_id = None;
                for key in self.keys.iter_mut() {
                    key.is_pressed = false;
                }
                self.suggestions_dirty = true;
                let final_ch = if self.state.shift_state.is_uppercase() {
                    character.to_uppercase().next().unwrap_or(character)
                } else {
                    character
                };
                self.execute_key_action(KeyAction::Character(final_ch));
            }

            TouchResult::LongPressTriggered {
                key_id: _,
                alternates: _,
            } => {
                self.state.push_event(KeyboardOutputEvent::PerformHaptic(
                    HapticFeedbackType::LongPress,
                ));
            }

            TouchResult::CursorMove { delta } => {
                self.state
                    .push_event(KeyboardOutputEvent::MoveCursor(delta));
                self.state.push_event(KeyboardOutputEvent::PerformHaptic(
                    HapticFeedbackType::KeyTick,
                ));
            }

            TouchResult::CursorModeStarted => {
                self.active_popup_key_id = None;
                for key in self.keys.iter_mut() {
                    key.is_pressed = false;
                }
                self.state
                    .push_event(KeyboardOutputEvent::PerformHaptic(HapticFeedbackType::LongPress));
            }

            TouchResult::BackspaceSelect { words } => {
                for key in self.keys.iter_mut() {
                    key.is_pressed = false;
                }
                if self.settings.backspace_swipe_deletes_word {
                    self.active_popup_key_id = None;
                    self.state.push_event(KeyboardOutputEvent::SelectWordsBack(words));
                    self.state.push_event(KeyboardOutputEvent::PerformHaptic(HapticFeedbackType::KeyTick));
                }
            }

            TouchResult::GestureStarted => {
                self.active_popup_key_id = None;
                for key in self.keys.iter_mut() {
                    key.is_pressed = false;
                }
            }

            TouchResult::Gesture { path } => self.commit_gesture(&path),

            TouchResult::BackspaceSelectCommit => {
                for key in self.keys.iter_mut() {
                    key.is_pressed = false;
                }
                if self.settings.backspace_swipe_deletes_word {
                    self.state.push_event(KeyboardOutputEvent::DeleteSelection);
                    self.state.composing_text.clear();
                    self.state.composing_touches.clear();
                    self.state.clear_context(false);
                    self.suggestions_dirty = true;
                    self.state
                        .push_event(KeyboardOutputEvent::PerformHaptic(HapticFeedbackType::KeyHeavyClick));
                }
            }

            TouchResult::SwitchLanguageSwipe { is_next } => {
                self.active_popup_key_id = None;
                for key in self.keys.iter_mut() {
                    key.is_pressed = false;
                }
                if !self.settings.space_swipe_switches_language {
                    // Swiping on space does nothing then
                } else if is_next {
                    self.next_language();
                } else {
                    self.prev_language();
                }
            }

            TouchResult::None => {}
        }

        if action == TouchAction::Up || action == TouchAction::Cancel {
            // Guarantee no key remains stuck pressed when fingers lift
            for key in self.keys.iter_mut() {
                let is_still_held = self
                    .touch_tracker
                    .pointers
                    .iter()
                    .any(|p| p.active_key_id == Some(key.id));
                if !is_still_held {
                    key.is_pressed = false;
                }
            }
            if self.touch_tracker.pointers.is_empty() {
                self.active_popup_key_id = None;
            }
        }
    }

    /// Forgets every touch in progress: no pointer, no pressed key, no popup. Used when the
    /// keyboard is shown again and when a new first finger lands, so a touch whose release never
    /// arrived (keyboard hidden mid-press, finger lifted over the navigation bar) cannot leave
    /// keys looking pressed.
    pub fn reset_touch_state(&mut self) {
        self.touch_tracker.pointers.clear();
        self.touch_down_points.clear();
        self.pending_touch = None;
        self.active_popup_key_id = None;
        for key in self.keys.iter_mut() {
            key.is_pressed = false;
        }
    }

    pub fn tick(&mut self, current_time_ms: u64) {
        match self.touch_tracker.check_long_press(current_time_ms, &self.keys) {
            Some(TouchResult::LongPressTriggered { .. }) => {
                self.state.push_event(KeyboardOutputEvent::PerformHaptic(HapticFeedbackType::LongPress));
            }
            Some(TouchResult::CursorModeStarted) => {
                self.active_popup_key_id = None;
                for key in self.keys.iter_mut() {
                    key.is_pressed = false;
                }
                self.state.push_event(KeyboardOutputEvent::PerformHaptic(HapticFeedbackType::LongPress));
            }
            _ => {}
        }
    }

    pub fn execute_key_action(&mut self, action: KeyAction) {
        self.suggestions_dirty = true;
        // Any key ends the gesture word; the handlers below decide what that means
        let after_gesture = std::mem::take(&mut self.state.gesture_word) && !self.state.composing_text.is_empty();
        match action {
            KeyAction::Character(ch) if after_gesture && ch.is_alphanumeric() => {
                // Typing on after a gestured word starts a new word
                self.finish_gesture_word();
                self.execute_key_action(KeyAction::Character(ch));
            }
            KeyAction::Backspace if after_gesture => {
                // Backspace right after a gesture removes the whole word
                let units = self.state.composing_text.encode_utf16().count() as u32;
                self.state.push_event(KeyboardOutputEvent::DeleteSurroundingText { before: units, after: 0 });
                self.state.composing_text.clear();
                self.state.composing_touches.clear();
                self.state.push_event(KeyboardOutputEvent::PerformHaptic(HapticFeedbackType::KeyTick));
            }
            KeyAction::Character(ch) => {
                self.last_space_tap_time_ms = 0;
                self.state.last_autocorrect_original = None;
                self.state.last_autocorrect_replacement = None;
                self.state.rejected_autocorrect_word = None;

                // Korean Hangul Syllable Composition Engine
                if self.state.language == Language::Korean && HangulComposer::is_hangul_jamo(ch) {
                    self.state.last_char_was_space = false;
                    match self.hangul_composer.feed_jamo(ch) {
                        hangul::HangulAction::Commit(c) => {
                            self.state.composing_text.push(c);
                            self.state
                                .push_event(KeyboardOutputEvent::CommitText(c.to_string()));
                        }
                        hangul::HangulAction::Replace(c) => {
                            self.state.composing_text.pop();
                            self.state.composing_text.push(c);
                            self.state
                                .push_event(KeyboardOutputEvent::DeleteSurroundingText {
                                    before: 1,
                                    after: 0,
                                });
                            self.state
                                .push_event(KeyboardOutputEvent::CommitText(c.to_string()));
                        }
                        hangul::HangulAction::Split(c1, c2) => {
                            self.state.composing_text.pop();
                            self.state.composing_text.push(c1);
                            self.state.composing_text.push(c2);
                            self.state
                                .push_event(KeyboardOutputEvent::DeleteSurroundingText {
                                    before: 1,
                                    after: 0,
                                });
                            self.state
                                .push_event(KeyboardOutputEvent::CommitText(format!("{}{}", c1, c2)));
                        }
                    }
                    let next_shift = self.state.shift_state.on_char_typed();
                    if next_shift != self.state.shift_state {
                        self.state.shift_state = next_shift;
                        self.rebuild_layout();
                    }
                    self.prediction_version = self.prediction_version.wrapping_add(1);
                    self.suggestions_dirty = true;
                    self.update_suggestions();
                    return;
                }

                self.hangul_composer.reset();

                let is_punctuation =
                    ch == '.' || ch == ',' || ch == '!' || ch == '?' || ch == ';' || ch == ':';
                if is_punctuation
                    && self.state.mode == KeyboardMode::Alphabet
                    && self.state.field_mode.allows_smart_punctuation()
                    && self.settings.smart_punctuation
                {
                    // Smart Punctuation:
                    // If preceding character was a space, swallow it before punctuation
                    if self.state.last_char_was_space {
                        self.state
                            .push_event(KeyboardOutputEvent::DeleteSurroundingText {
                                before: 1,
                                after: 0,
                            });
                    }
                    if !self.state.composing_text.is_empty() {
                        self.state.commit_context_word(self.state.composing_text.clone());
                        self.state.composing_text.clear();
                    }
                    // Commit punctuation followed by auto-spacing
                    self.state
                        .push_event(KeyboardOutputEvent::CommitText(format!("{} ", ch)));
                    self.state.last_char_was_space = true;

                    // Auto-capitalize after sentence ending punctuation
                    if ch == '.' || ch == '!' || ch == '?' {
                        self.state.clear_context(true);
                        if self.settings.auto_capitalization {
                            self.state.shift_state = state::ShiftState::Shifted;
                            self.rebuild_layout();
                        }
                    }
                } else {
                    self.state.last_char_was_space = false;
                    if self.state.composing_text.is_empty() {
                        // A new word: touches of the previous one are stale
                        self.state.composing_touches.clear();
                    }
                    self.state.composing_text.push(ch);
                    self.state.composing_touches.push(self.pending_touch);
                    self.state
                        .push_event(KeyboardOutputEvent::CommitText(ch.to_string()));
                    let next_shift = self.state.shift_state.on_char_typed();
                    if next_shift != self.state.shift_state {
                        self.state.shift_state = next_shift;
                        self.rebuild_layout();
                    }
                }
            }

            KeyAction::Shift => {
                let now = self.last_interaction_time_ms;
                let is_double_tap = now.saturating_sub(self.last_shift_tap_time_ms) < 280;

                if self.state.shift_state == state::ShiftState::CapsLock {
                    self.state.shift_state = state::ShiftState::Off;
                    self.last_shift_tap_time_ms = 0;
                } else if is_double_tap {
                    self.state.shift_state = state::ShiftState::CapsLock;
                    self.last_shift_tap_time_ms = 0;
                } else {
                    self.state.shift_state = if self.state.shift_state == state::ShiftState::Off {
                        state::ShiftState::Shifted
                    } else {
                        state::ShiftState::Off
                    };
                    self.last_shift_tap_time_ms = now;
                }

                self.rebuild_layout();
                self.state.push_event(KeyboardOutputEvent::PerformHaptic(
                    HapticFeedbackType::KeyTick,
                ));
            }

            KeyAction::Backspace => {
                self.last_space_tap_time_ms = 0;
                self.state.last_char_was_space = false;
                let undo = (
                    self.state.last_autocorrect_original.take(),
                    self.state.last_autocorrect_replacement.take(),
                );
                if let (Some(orig), Some(repl), true) = (undo.0, undo.1, self.settings.undo_autocorrect) {
                    // Undo autocorrect: restore original typed text + space, keeping cursor after space
                    let repl_len = (repl.encode_utf16().count() + 1) as u32;
                    self.state
                        .push_event(KeyboardOutputEvent::DeleteSurroundingText {
                            before: repl_len,
                            after: 0,
                        });
                    self.state
                        .push_event(KeyboardOutputEvent::CommitText(format!("{} ", orig)));
                    self.state.composing_text.clear();
                    self.state.last_committed_word = orig.clone();
                    self.state.last_char_was_space = true;
                    self.state.rejected_autocorrect_word = Some(orig.to_lowercase());
                    self.prediction.add_user_word_in(&orig, self.state.language);
                    self.suggestions_dirty = true;
                    self.state.push_event(KeyboardOutputEvent::PerformHaptic(
                        HapticFeedbackType::KeyClick,
                    ));
                    return;
                }

                self.state.last_autocorrect_original = None;
                self.state.last_autocorrect_replacement = None;
                self.state.rejected_autocorrect_word = None;

                // Korean Hangul Syllable Decomposition
                if self.state.language == Language::Korean && self.hangul_composer.is_active() {
                    match self.hangul_composer.feed_backspace() {
                        hangul::HangulBackspaceResult::Replace(c) => {
                            self.state.composing_text.pop();
                            self.state.composing_text.push(c);
                            self.state
                                .push_event(KeyboardOutputEvent::DeleteSurroundingText {
                                    before: 1,
                                    after: 0,
                                });
                            self.state
                                .push_event(KeyboardOutputEvent::CommitText(c.to_string()));
                            self.prediction_version = self.prediction_version.wrapping_add(1);
                            self.suggestions_dirty = true;
                            self.update_suggestions();
                            self.state.push_event(KeyboardOutputEvent::PerformHaptic(
                                HapticFeedbackType::KeyTick,
                            ));
                            return;
                        }
                        hangul::HangulBackspaceResult::Delete => {
                            self.state.composing_text.pop();
                            self.state
                                .push_event(KeyboardOutputEvent::DeleteSurroundingText {
                                    before: 1,
                                    after: 0,
                                });
                            self.prediction_version = self.prediction_version.wrapping_add(1);
                            self.suggestions_dirty = true;
                            self.update_suggestions();
                            self.state.push_event(KeyboardOutputEvent::PerformHaptic(
                                HapticFeedbackType::KeyTick,
                            ));
                            return;
                        }
                        hangul::HangulBackspaceResult::None => {}
                    }
                }

                // Grapheme-cluster aware deletion
                if let Some(utf16_units) = self.state.pop_last_grapheme() {
                    self.state
                        .push_event(KeyboardOutputEvent::DeleteSurroundingText {
                            before: utf16_units,
                            after: 0,
                        });
                } else {
                    self.state.clear_context(false);
                    self.state
                        .push_event(KeyboardOutputEvent::DeleteSurroundingText {
                            before: 1,
                            after: 0,
                        });
                }
                self.state.push_event(KeyboardOutputEvent::PerformHaptic(
                    HapticFeedbackType::KeyTick,
                ));
            }

            KeyAction::Enter => {
                self.hangul_composer.reset();
                self.last_space_tap_time_ms = 0;
                self.state.last_char_was_space = false;
                self.state.last_autocorrect_original = None;
                self.state.last_autocorrect_replacement = None;
                self.state.rejected_autocorrect_word = None;

                if !self.state.composing_text.is_empty() {
                    self.state.commit_context_word(self.state.composing_text.clone());
                    self.state.composing_text.clear();
                }
                self.state.clear_context(true);
                if self.enter_action != 0 {
                    self.state
                        .push_event(KeyboardOutputEvent::PerformEditorAction(self.enter_action));
                } else if self.state.field_mode == state::InputFieldMode::Multiline {
                    self.state
                        .push_event(KeyboardOutputEvent::CommitText("\n".to_string()));
                } else {
                    self.state.push_event(KeyboardOutputEvent::SendKeyEvent(66));
                    // KEYCODE_ENTER
                }
                self.state.push_event(KeyboardOutputEvent::PerformHaptic(
                    HapticFeedbackType::KeyHeavyClick,
                ));
            }

            KeyAction::Space => {
                self.hangul_composer.reset();
                let now = self.last_interaction_time_ms;
                let is_double_tap = self.last_space_tap_time_ms != 0
                    && now.saturating_sub(self.last_space_tap_time_ms) < 300
                    && self.state.composing_text.is_empty();

                if is_double_tap
                    && self.settings.double_space == state::DoubleSpaceAction::Period
                    && self.state.field_mode.allows_smart_punctuation()
                {
                    // "word  " -> "word. "
                    self.state
                        .push_event(KeyboardOutputEvent::DeleteSurroundingText { before: 1, after: 0 });
                    self.state.push_event(KeyboardOutputEvent::CommitText(". ".to_string()));
                    self.state.clear_context(true);
                    if self.settings.auto_capitalization {
                        self.state.shift_state = state::ShiftState::Shifted;
                        self.rebuild_layout();
                    }
                    self.last_space_tap_time_ms = 0;
                    self.state.last_char_was_space = true;
                    self.state.push_event(KeyboardOutputEvent::PerformHaptic(HapticFeedbackType::KeyTick));
                    return;
                }

                if is_double_tap && self.settings.double_space == state::DoubleSpaceAction::SwitchLanguage {
                    // Double tap on spacebar: switch language
                    self.state
                        .push_event(KeyboardOutputEvent::DeleteSurroundingText {
                            before: 1,
                            after: 0,
                        });
                    self.next_language();
                    self.last_space_tap_time_ms = 0;
                    self.state.last_char_was_space = false;
                    self.state.push_event(KeyboardOutputEvent::PerformHaptic(
                        HapticFeedbackType::KeyHeavyClick,
                    ));
                    return;
                }

                self.last_space_tap_time_ms = now;

                if !self.state.composing_text.is_empty() {
                    let clean = self.state.composing_text.trim().to_lowercase();

                    let is_rejected =
                        self.state.rejected_autocorrect_word.as_deref() == Some(&clean);
                    self.state.rejected_autocorrect_word = None;

                    // Check dictionaries + user dictionary + whether autocorrect was rejected:
                    let dictionary = &self.prediction.dictionary;
                    let is_valid_word = is_rejected
                        || after_gesture
                        || (dictionary.contains_word_for_lang(&clean, self.state.language)
                            && dictionary.dominant_alternative(&clean, self.state.language).is_none());

                    let mut word_to_commit = self.state.composing_text.clone();
                    let mut did_autocorrect = false;

                    if !is_valid_word
                        && self.autocorrect_enabled
                        && self.state.field_mode.allows_autocorrect()
                    {
                        let obs = crate::prediction::correction::observations(
                            &self.state.composing_text,
                            &self.state.composing_touches,
                        );
                        let choice = self.prediction.dictionary.autocorrect_choice(
                            self.state.language,
                            &self.state.word_context(),
                            &self.state.composing_text,
                            &obs,
                            Some(&self.key_geometry),
                            self.autocorrect_strength,
                        );
                        if let Some(word) = choice {
                            word_to_commit = crate::prediction::suggestions::SuggestionEngine::match_case(
                                &self.state.composing_text,
                                &word,
                            );
                            did_autocorrect = true;
                        }
                    }

                    if did_autocorrect {
                        let before_count = self.state.composing_text.encode_utf16().count() as u32;
                        if before_count > 0 {
                            self.state
                                .push_event(KeyboardOutputEvent::DeleteSurroundingText {
                                    before: before_count,
                                    after: 0,
                                });
                        }
                        self.state
                            .push_event(KeyboardOutputEvent::CommitText(format!(
                                "{} ",
                                word_to_commit
                            )));
                        self.state.last_autocorrect_original =
                            Some(self.state.composing_text.clone());
                        self.state.last_autocorrect_replacement = Some(word_to_commit.clone());
                    } else {
                        // Word was already committed character-by-character!
                        // Simply commit a space without altering or deleting anything!
                        self.state
                            .push_event(KeyboardOutputEvent::CommitText(" ".to_string()));
                        self.state.last_autocorrect_original = None;
                        self.state.last_autocorrect_replacement = None;
                    }

                    // A missing-space fix commits two words
                    let words: Vec<String> = word_to_commit.split(' ').map(str::to_string).collect();
                    for word in words {
                        if self.state.field_mode.allows_learning() {
                            if !self.state.last_committed_word.is_empty() {
                                self.prediction.learn_bigram(&self.state.last_committed_word, &word);
                            }
                            let lang = self.state.language;
                            if is_rejected {
                                self.prediction.add_user_word_in(&word, lang);
                            } else if is_valid_word || did_autocorrect {
                                self.prediction.learn_word(&word, lang);
                            }
                        }
                        self.state.commit_context_word(word);
                    }
                    self.state.composing_text.clear();
                    self.state.last_char_was_space = true;
                } else {
                    self.state.last_autocorrect_original = None;
                    self.state.last_autocorrect_replacement = None;
                    self.state.rejected_autocorrect_word = None;
                    self.state
                        .push_event(KeyboardOutputEvent::CommitText(" ".to_string()));
                    self.state.last_char_was_space = true;
                }
                self.state.push_event(KeyboardOutputEvent::PerformHaptic(
                    HapticFeedbackType::KeyTick,
                ));
            }

            KeyAction::SwitchLanguage => {
                self.hangul_composer.reset();
                self.toggle_language();
            }

            KeyAction::SwitchMode(mode) => {
                self.set_mode(mode);
            }

            KeyAction::SwitchEmoji => {
                self.set_mode(KeyboardMode::Emoji);
            }

            KeyAction::Settings => {
                self.state.push_event(KeyboardOutputEvent::OpenSettings);
            }

            KeyAction::HideKeyboard => {
                self.state.push_event(KeyboardOutputEvent::HideKeyboard);
                self.state.push_event(KeyboardOutputEvent::PerformHaptic(
                    HapticFeedbackType::KeyClick,
                ));
            }

            KeyAction::SwitchInputMethod => {
                self.state
                    .push_event(KeyboardOutputEvent::SwitchInputMethod);
                self.state.push_event(KeyboardOutputEvent::PerformHaptic(
                    HapticFeedbackType::KeyClick,
                ));
            }

            KeyAction::OneHandedSwitchSide => {
                let other = match self.layout_options.one_handed {
                    layout::OneHanded::Left => layout::OneHanded::Right,
                    _ => layout::OneHanded::Left,
                };
                self.set_one_handed(other);
                self.state.push_event(KeyboardOutputEvent::PerformHaptic(HapticFeedbackType::KeyTick));
            }

            KeyAction::Edit(edit) => self.execute_edit_action(edit),

            KeyAction::OneHandedOff => {
                self.set_one_handed(layout::OneHanded::Off);
                self.state.push_event(KeyboardOutputEvent::PerformHaptic(HapticFeedbackType::KeyTick));
            }

            KeyAction::None => {}
        }

        self.prediction_version = self.prediction_version.wrapping_add(1);
        self.suggestions_dirty = true;
        self.update_suggestions();
    }
}
