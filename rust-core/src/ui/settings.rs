// SPDX-License-Identifier: GPL-3.0-or-later
//! Settings screen drawn with egui widgets.

use crate::layout::LANGUAGES;
use crate::prefs::{Prefs, Theme};
use egui::{Color32, RichText};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Main,
    Languages,
}

/// Requests to the platform
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsCmd {
    /// Open the system list of keyboards to enable Rynk
    EnableIme,
    /// Show the keyboard picker
    PickIme,
    /// Leave the screen
    Finish,
    /// Personal dictionaries were cleared
    DictCleared,
}

pub struct SettingsApp {
    pub prefs: Prefs,
    pub page: Page,
    ru: bool,
    pub enabled: bool,
    pub selected: bool,
    pub out: Vec<SettingsCmd>,
    pub changed: bool,
    confirm_clear: bool,
}

const YELLOW: Color32 = Color32::from_rgb(0xFF, 0xDB, 0x4D);

impl SettingsApp {
    pub fn new(prefs: Prefs, ui_locale: &str) -> Self {
        let ru = ["ru", "uk", "be", "kk", "ky", "tg", "uz", "tt", "ba", "mn", "hy", "ka", "az"]
            .iter()
            .any(|l| ui_locale.to_lowercase().starts_with(l));
        Self { prefs, page: Page::Main, ru, enabled: false, selected: false, out: Vec::new(), changed: false, confirm_clear: false }
    }

    fn t<'a>(&self, ru: &'a str, en: &'a str) -> &'a str {
        if self.ru { ru } else { en }
    }

    /// System back: returns true when handled inside the screen
    pub fn back(&mut self) -> bool {
        if self.page != Page::Main {
            self.page = Page::Main;
            true
        } else {
            false
        }
    }

    pub fn style(ctx: &egui::Context, dark: bool) {
        let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.selection.bg_fill = YELLOW;
        v.selection.stroke.color = Color32::from_rgb(0x15, 0x15, 0x15);
        v.panel_fill = if dark { Color32::from_rgb(0x16, 0x16, 0x18) } else { Color32::from_rgb(0xF2, 0xF3, 0xF5) };
        v.widgets.inactive.corner_radius = egui::CornerRadius::same(10);
        v.widgets.hovered.corner_radius = egui::CornerRadius::same(10);
        v.widgets.active.corner_radius = egui::CornerRadius::same(10);
        ctx.set_visuals(v);
        ctx.global_style_mut(|s| {
            use egui::{FontId, TextStyle};
            s.text_styles.insert(TextStyle::Body, FontId::proportional(18.0));
            s.text_styles.insert(TextStyle::Button, FontId::proportional(18.0));
            s.text_styles.insert(TextStyle::Heading, FontId::proportional(26.0));
            s.text_styles.insert(TextStyle::Small, FontId::proportional(14.0));
            s.spacing.interact_size = egui::vec2(48.0, 44.0);
            s.spacing.item_spacing = egui::vec2(10.0, 12.0);
            s.spacing.button_padding = egui::vec2(16.0, 10.0);
            s.spacing.icon_width = 26.0;
            s.spacing.icon_width_inner = 14.0;
            s.spacing.slider_width = 200.0;
        });
    }

    pub fn ui(&mut self, root: &mut egui::Ui, top: f32, bottom: f32) {
        let frame = egui::Frame::central_panel(&root.ctx().global_style()).inner_margin(egui::Margin { left: 16, right: 16, top: top as i8, bottom: bottom.min(120.0) as i8 });
        egui::CentralPanel::default().frame(frame).show(root, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.set_width(ui.available_width());
                match self.page {
                    Page::Main => self.main(ui),
                    Page::Languages => self.languages(ui),
                }
                ui.add_space(24.0);
            });
        });
    }

    fn toggle(&mut self, ui: &mut egui::Ui, value: fn(&mut Prefs) -> &mut bool, title: &str, sub: &str) {
        let v = value(&mut self.prefs);
        let mut b = *v;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(ui.available_width() - 64.0);
                ui.label(title);
                if !sub.is_empty() {
                    ui.label(RichText::new(sub).small().weak());
                }
            });
            if ui.checkbox(&mut b, "").changed() {
                *value(&mut self.prefs) = b;
                self.changed = true;
            }
        });
        ui.separator();
    }

    fn main(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.heading("Rynk");
        ui.label(RichText::new(self.t("Клавиатура с автозаменой. Всё работает на устройстве.", "Keyboard with autocorrect. Everything runs on the device.")).weak());
        ui.add_space(12.0);

        if !self.enabled || !self.selected {
            egui::Frame::group(ui.style()).corner_radius(14).inner_margin(14).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.strong(self.t("Включение", "Setup"));
                let b1 = egui::Button::new(self.t("1. Включить Rynk", "1. Enable Rynk")).fill(if self.enabled { Color32::TRANSPARENT } else { YELLOW });
                if ui.add_sized([ui.available_width(), 48.0], b1).clicked() {
                    self.out.push(SettingsCmd::EnableIme);
                }
                let b2 = egui::Button::new(self.t("2. Выбрать Rynk", "2. Switch to Rynk")).fill(if self.enabled && !self.selected { YELLOW } else { Color32::TRANSPARENT });
                if ui.add_sized([ui.available_width(), 48.0], b2).clicked() {
                    self.out.push(SettingsCmd::PickIme);
                }
            });
            ui.add_space(12.0);
        }

        let names: Vec<&str> = self.prefs.languages.iter().filter_map(|c| crate::layout::LangDef::by_code(c)).map(|l| l.name).collect();
        let langs_title = self.t("Языки", "Languages").to_string();
        if ui.add_sized([ui.available_width(), 56.0], egui::Button::new(format!("{langs_title}: {}  ›", names.join(", ")))).clicked() {
            self.page = Page::Languages;
        }
        ui.add_space(8.0);

        ui.strong(self.t("Ввод", "Typing"));
        let s = |me: &Self, a, b| me.t(a, b).to_string();
        let (a, b) = (s(self, "Автозамена", "Autocorrect"), s(self, "Исправляет опечатки при нажатии пробела; Backspace отменяет", "Fixes typos on space; Backspace undoes it"));
        self.toggle(ui, |p| &mut p.autocorrect, &a, &b);
        let (a, b) = (s(self, "Подсказки слов", "Word suggestions"), s(self, "Варианты и следующее слово над клавиатурой", "Candidates and next word above the keys"));
        self.toggle(ui, |p| &mut p.suggestions, &a, &b);
        let (a, b) = (s(self, "Динамическая сетка", "Dynamic key grid"), s(self, "Вероятные буквы ловят касания чуть шире", "Likely letters catch touches a bit wider"));
        self.toggle(ui, |p| &mut p.dynamic_grid, &a, &b);
        let (a, b) = (s(self, "Заглавная в начале предложения", "Auto-capitalization"), String::new());
        self.toggle(ui, |p| &mut p.autocap, &a, &b);
        let (a, b) = (s(self, "Точка двойным пробелом", "Double-space period"), String::new());
        self.toggle(ui, |p| &mut p.double_space_period, &a, &b);
        let (a, b) = (s(self, "Учить мои слова", "Learn my words"), s(self, "Личный словарь хранится только на устройстве", "The personal dictionary stays on the device"));
        self.toggle(ui, |p| &mut p.learn, &a, &b);

        ui.add_space(8.0);
        ui.strong(self.t("Вид", "Look"));
        let (a, b) = (s(self, "Ряд цифр", "Number row"), String::new());
        self.toggle(ui, |p| &mut p.number_row, &a, &b);
        let (a, b) = (s(self, "Увеличение при нажатии", "Key preview"), String::new());
        self.toggle(ui, |p| &mut p.key_preview, &a, &b);
        let (a, b) = (s(self, "Вибрация", "Vibration"), String::new());
        self.toggle(ui, |p| &mut p.vibrate, &a, &b);
        ui.label(self.t("Тема", "Theme"));
        ui.horizontal(|ui| {
            for (t, ru, en) in [(Theme::System, "Как в системе", "System"), (Theme::Light, "Светлая", "Light"), (Theme::Dark, "Тёмная", "Dark")] {
                let label = if self.ru { ru } else { en };
                if ui.selectable_label(self.prefs.theme == t, label).clicked() {
                    self.prefs.theme = t;
                    self.changed = true;
                }
            }
        });
        ui.label(self.t("Высота клавиатуры", "Keyboard height"));
        if ui.add(egui::Slider::new(&mut self.prefs.height, 0.7..=1.4).show_value(false)).changed() {
            self.changed = true;
        }
        ui.separator();

        ui.add_space(8.0);
        if !self.confirm_clear {
            if ui.button(self.t("Очистить личный словарь", "Clear personal dictionary")).clicked() {
                self.confirm_clear = true;
            }
        } else {
            ui.label(self.t("Удалить выученные слова и пары?", "Delete learned words and pairs?"));
            ui.horizontal(|ui| {
                if ui.button(self.t("Удалить", "Delete")).clicked() {
                    self.out.push(SettingsCmd::DictCleared);
                    self.confirm_clear = false;
                }
                if ui.button(self.t("Отмена", "Cancel")).clicked() {
                    self.confirm_clear = false;
                }
            });
        }
        ui.add_space(16.0);
        ui.label(RichText::new("Rynk 2.0 · GPL-3.0-or-later").small().weak());
        ui.label(RichText::new(self.t("Языковые модели: CC BY-SA 4.0 (см. DATA_LICENSES)", "Language models: CC BY-SA 4.0 (see DATA_LICENSES)")).small().weak());
    }

    fn languages(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("‹").clicked() {
                self.page = Page::Main;
            }
            ui.heading(self.t("Языки", "Languages"));
        });
        ui.label(RichText::new(self.t("Порядок переключения — порядок включения. Переключение: кнопка 🌐 или долгое нажатие на пробел.", "Switching follows the order you enable them in. Switch with 🌐 or a long press on space.")).small().weak());
        ui.add_space(6.0);
        // enabled first, in their order
        let mut order: Vec<&'static crate::layout::LangDef> = self.prefs.languages.iter().filter_map(|c| crate::layout::LangDef::by_code(c)).collect();
        let mut rest: Vec<&'static crate::layout::LangDef> = LANGUAGES.iter().filter(|l| !self.prefs.languages.iter().any(|c| c == l.code)).collect();
        rest.sort_by_key(|l| l.name.to_lowercase());
        order.extend(rest);
        for l in order {
            let mut on = self.prefs.languages.iter().any(|c| c == l.code);
            let label = format!("{}   {}", l.name, RichText::new(l.code).text());
            ui.horizontal(|ui| {
                if ui.checkbox(&mut on, label).changed() {
                    if on {
                        self.prefs.languages.push(l.code.to_string());
                    } else if self.prefs.languages.len() > 1 {
                        self.prefs.languages.retain(|c| c != l.code);
                    }
                    self.changed = true;
                }
                if l.lm.is_empty() {
                    ui.label(RichText::new(self.t("без словаря", "no dictionary")).small().weak());
                }
            });
        }
    }
}
