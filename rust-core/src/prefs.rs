// SPDX-License-Identifier: GPL-3.0-or-later
//! User settings, stored as `key=value` lines in the app's files directory.

use crate::layout::LangDef;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    /// Enabled layouts in switching order
    pub languages: Vec<String>,
    pub autocorrect: bool,
    pub suggestions: bool,
    pub autocap: bool,
    pub double_space_period: bool,
    pub dynamic_grid: bool,
    pub learn: bool,
    pub vibrate: bool,
    pub number_row: bool,
    pub key_preview: bool,
    pub theme: Theme,
    /// Keyboard height multiplier
    pub height: f32,
}

impl Prefs {
    /// Defaults: the system languages that have a layout, plus English
    pub fn defaults(system_locales: &[String]) -> Self {
        let mut languages: Vec<String> = Vec::new();
        for tag in system_locales {
            if let Some(l) = LangDef::for_locale(tag) {
                if !languages.iter().any(|c| c == l.code) {
                    languages.push(l.code.to_string());
                }
            }
        }
        if !languages.iter().any(|c| c.starts_with("en")) {
            languages.push("en".into());
        }
        Prefs {
            languages,
            autocorrect: true,
            suggestions: true,
            autocap: true,
            double_space_period: true,
            dynamic_grid: true,
            learn: true,
            vibrate: true,
            number_row: false,
            key_preview: true,
            theme: Theme::System,
            height: 1.0,
        }
    }

    pub fn parse(text: &str, system_locales: &[String]) -> Self {
        let mut p = Self::defaults(system_locales);
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let b = v == "1";
            match k {
                "languages" => {
                    let l: Vec<String> = v.split(',').filter(|c| LangDef::by_code(c).is_some()).map(String::from).collect();
                    if !l.is_empty() {
                        p.languages = l;
                    }
                }
                "autocorrect" => p.autocorrect = b,
                "suggestions" => p.suggestions = b,
                "autocap" => p.autocap = b,
                "double_space_period" => p.double_space_period = b,
                "dynamic_grid" => p.dynamic_grid = b,
                "learn" => p.learn = b,
                "vibrate" => p.vibrate = b,
                "number_row" => p.number_row = b,
                "key_preview" => p.key_preview = b,
                "theme" => {
                    p.theme = match v {
                        "light" => Theme::Light,
                        "dark" => Theme::Dark,
                        _ => Theme::System,
                    }
                }
                "height" => p.height = v.parse::<f32>().unwrap_or(1.0).clamp(0.7, 1.4),
                _ => {}
            }
        }
        p
    }

    pub fn serialize(&self) -> String {
        let b = |x: bool| if x { "1" } else { "0" };
        let theme = match self.theme {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        };
        format!(
            "languages={}\nautocorrect={}\nsuggestions={}\nautocap={}\ndouble_space_period={}\ndynamic_grid={}\nlearn={}\nvibrate={}\nnumber_row={}\nkey_preview={}\ntheme={}\nheight={:.2}\n",
            self.languages.join(","),
            b(self.autocorrect),
            b(self.suggestions),
            b(self.autocap),
            b(self.double_space_period),
            b(self.dynamic_grid),
            b(self.learn),
            b(self.vibrate),
            b(self.number_row),
            b(self.key_preview),
            theme,
            self.height
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_system_plus_english() {
        let p = Prefs::defaults(&["ru-RU".into(), "uk-UA".into()]);
        assert_eq!(p.languages, ["ru", "uk", "en"]);
        let p = Prefs::defaults(&["en-GB".into()]);
        assert_eq!(p.languages, ["en-GB"]);
        let p = Prefs::defaults(&["zz".into()]);
        assert_eq!(p.languages, ["en"]);
    }

    #[test]
    fn roundtrip() {
        let mut p = Prefs::defaults(&["de-DE".into()]);
        p.autocorrect = false;
        p.theme = Theme::Dark;
        p.height = 1.2;
        assert_eq!(Prefs::parse(&p.serialize(), &[]), p);
    }
}
