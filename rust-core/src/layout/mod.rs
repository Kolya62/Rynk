// SPDX-License-Identifier: GPL-3.0-or-later
//! Language definitions and key geometry.

mod languages;
pub use languages::LANGUAGES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Composer {
    None,
    Hangul,
    /// Romaji → kana, then kana → kanji conversion
    Kana,
    /// Pinyin letters → Chinese characters
    Pinyin,
    /// Bopomofo → Chinese characters
    Zhuyin,
    /// Jyutping letters → Cantonese characters
    Jyutping,
}

impl Composer {
    /// Composers that convert a reading through a dictionary
    pub fn converts(self) -> bool {
        matches!(self, Composer::Kana | Composer::Pinyin | Composer::Zhuyin | Composer::Jyutping)
    }
}

#[derive(Debug)]
pub struct LangDef {
    pub code: &'static str,
    /// Endonym shown on the space bar and in settings
    pub name: &'static str,
    /// Language model asset `lm/<lm>.rlm`, empty when there is none
    pub lm: &'static str,
    pub rows: &'static [&'static [&'static str]],
    /// Second layer for scripts without letter case (InScript, Thai, Korean…)
    pub shift_rows: &'static [&'static [&'static str]],
    pub popups: &'static [(&'static str, &'static [&'static str])],
    pub caseless: bool,
    /// Noto script name used to pick a system font
    pub script: &'static str,
    pub composer: Composer,
    /// Turkish/Azerbaijani dotted and dotless i
    pub turkic: bool,
    pub currency: &'static str,
    /// Conversion dictionary asset `cjk/<conv>.rcd`, empty when none
    pub conv: &'static str,
}

impl LangDef {
    pub fn by_code(code: &str) -> Option<&'static LangDef> {
        LANGUAGES.iter().find(|l| l.code.eq_ignore_ascii_case(code))
    }

    pub fn popup(&self, key: &str) -> &'static [&'static str] {
        self.popups.iter().find(|p| p.0 == key).map_or(&[], |p| p.1)
    }

    /// Upper case of a key label for this language
    pub fn upper(&self, s: &str) -> String {
        if self.caseless {
            return s.to_string();
        }
        if self.turkic {
            match s {
                "i" => return "İ".into(),
                "ı" => return "I".into(),
                _ => {}
            }
        }
        let u = s.to_uppercase();
        // "ß" → "SS" would put two letters on one key
        if u.chars().count() != s.chars().count() {
            s.to_string()
        } else {
            u
        }
    }

    /// Best layout for a locale tag such as "ru-RU", "pt_BR", "sr-Latn-RS", "zh-Hans"
    pub fn for_locale(tag: &str) -> Option<&'static LangDef> {
        let tag = tag.replace('_', "-");
        let parts: Vec<&str> = tag.split('-').collect();
        let lang = parts.first()?.to_lowercase();
        if lang == "zh" {
            let rest: Vec<String> = parts[1..].iter().map(|p| p.to_uppercase()).collect();
            let code = if rest.iter().any(|p| p == "TW") {
                "zh-TW"
            } else if rest.iter().any(|p| p == "HK" || p == "MO") {
                "yue"
            } else if rest.iter().any(|p| p == "HANT") {
                "zh-Hant"
            } else {
                "zh-Hans"
            };
            return LangDef::by_code(code);
        }
        let lang = match lang.as_str() {
            "nb" | "nn" => "no".to_string(),
            "fil" => "tl".to_string(),
            "in" => "id".to_string(),
            "iw" => "he".to_string(),
            "ji" => "yi".to_string(),
            _ => lang,
        };
        for n in (1..=parts.len()).rev() {
            let mut cand = lang.clone();
            for p in &parts[1..n] {
                cand.push('-');
                cand.push_str(p);
            }
            if let Some(l) = LangDef::by_code(&cand) {
                return Some(l);
            }
        }
        LangDef::by_code(&lang)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum KeyKind {
    /// Types its text
    Char(String),
    Shift,
    Backspace,
    Enter,
    Space,
    /// To symbols, page 1
    Symbols,
    /// To symbols page 2 / back to page 1
    SymbolsMore,
    Alphabet,
    Language,
    Emoji,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub kind: KeyKind,
    /// Visible key (drawn)
    pub rect: Rect,
    /// Touch area: the key plus half of the gaps around it, so there are no dead zones
    pub hit: Rect,
    pub label: String,
    /// Small corner hint (top-row digits, popup marker)
    pub hint: Option<String>,
    pub popup: Vec<String>,
    /// Function key (darker background)
    pub func: bool,
    /// Is a letter of the alphabet (participates in the touch model)
    pub letter: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Alphabet,
    Symbols,
    Symbols2,
    Emoji,
    /// The full list of conversion candidates over the key area
    Candidates,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShiftState {
    Off,
    On,
    Caps,
}

/// Geometry input
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// Width and height of the key area in px
    pub w: f32,
    pub top: f32,
    pub h: f32,
    pub dp: f32,
}

const DIGITS: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
const DIGIT_LABELS_EXTRA: [&str; 10] = ["¹½⅓¼", "²⅔", "³¾", "⁴", "⁵", "⁶", "⁷", "⁸", "⁹", "⁰ⁿ∅"];

pub struct BuildOptions<'a> {
    pub lang: &'a LangDef,
    pub mode: Mode,
    pub shift: ShiftState,
    pub number_row: bool,
    pub multi_lang: bool,
    pub enter_label: &'a str,
    /// The field wants an e-mail / URL: show "@" / "/" instead of the comma
    pub email: bool,
    pub url: bool,
}

/// Row specification before geometry: (key, width in units)
type Row = Vec<(Key, f32)>;

fn ch(label: &str, out: &str) -> Key {
    Key {
        kind: KeyKind::Char(out.to_string()),
        rect: Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
        hit: Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
        label: label.to_string(),
        hint: None,
        popup: Vec::new(),
        func: false,
        letter: false,
    }
}

fn func(kind: KeyKind, label: &str) -> Key {
    let mut k = ch(label, "");
    k.kind = kind;
    k.func = true;
    k
}

pub fn build(o: &BuildOptions, f: Frame) -> Vec<Key> {
    let mut rows: Vec<Row> = Vec::new();
    match o.mode {
        Mode::Alphabet => alphabet_rows(o, &mut rows),
        Mode::Symbols => symbol_rows(o, &mut rows, false),
        Mode::Symbols2 => symbol_rows(o, &mut rows, true),
        Mode::Emoji | Mode::Candidates => return Vec::new(),
    }
    rows.push(bottom_row(o));
    place(rows, f)
}

fn alphabet_rows(o: &BuildOptions, rows: &mut Vec<Row>) {
    let l = o.lang;
    let use_shift_layer = l.caseless && !l.shift_rows.is_empty() && o.shift != ShiftState::Off;
    let src = if use_shift_layer { l.shift_rows } else { l.rows };
    if o.number_row {
        rows.push(
            DIGITS
                .iter()
                .zip(DIGIT_LABELS_EXTRA)
                .map(|(d, extra)| {
                    let mut k = ch(d, d);
                    k.popup = extra.chars().map(String::from).collect();
                    (k, 1.0)
                })
                .collect(),
        );
    }
    let n_rows = src.len();
    for (ri, r) in src.iter().enumerate() {
        let mut row: Row = r
            .iter()
            .enumerate()
            .map(|(ci, &s)| {
                let shown = if !l.caseless && o.shift != ShiftState::Off { l.upper(s) } else { s.to_string() };
                let mut k = ch(&shown, &shown);
                k.letter = true;
                let mut popup: Vec<String> = Vec::new();
                // top row carries the digits as hints
                if ri == 0 && !o.number_row && r.len() >= 10 && ci < 10 {
                    k.hint = Some(DIGITS[ci].to_string());
                    popup.push(DIGITS[ci].to_string());
                }
                for p in l.popup(s) {
                    let p = if !l.caseless && o.shift != ShiftState::Off { l.upper(p) } else { p.to_string() };
                    if !popup.contains(&p) {
                        popup.push(p);
                    }
                }
                // the long-press letter is shown in the corner: "ъ" on "ь", "ё" on "е"
                if k.hint.is_none() && !popup.is_empty() {
                    k.hint = Some(if popup.len() == 1 { popup[0].clone() } else { "·".into() });
                }
                k.popup = popup;
                (k, 1.0)
            })
            .collect();
        if ri == n_rows - 1 {
            row.insert(0, (func(KeyKind::Shift, ""), 1.5));
            row.push((func(KeyKind::Backspace, ""), 1.5));
        }
        rows.push(row);
    }
}

fn symbol_rows(o: &BuildOptions, rows: &mut Vec<Row>, page2: bool) {
    let cur = o.lang.currency;
    let (r1, r2, r3): (Vec<String>, Vec<String>, Vec<String>) = if !page2 {
        (
            DIGITS.iter().map(|s| s.to_string()).collect(),
            ["@", "#", cur, "_", "&", "-", "+", "(", ")", "/"].iter().map(|s| s.to_string()).collect(),
            ["*", "\"", "'", ":", ";", "!", "?"].iter().map(|s| s.to_string()).collect(),
        )
    } else {
        (
            ["~", "`", "|", "•", "√", "π", "÷", "×", "§", "∆"].iter().map(|s| s.to_string()).collect(),
            ["£", "€", "$", "¥", "^", "°", "=", "{", "}", "\\"].iter().map(|s| s.to_string()).collect(),
            ["%", "©", "®", "™", "✓", "[", "]"].iter().map(|s| s.to_string()).collect(),
        )
    };
    let pop = |s: &str| -> Vec<String> {
        let v: &[&str] = match s {
            "1" => &["¹", "½", "⅓", "¼", "⅛"],
            "2" => &["²", "⅔"],
            "3" => &["³", "¾", "⅜"],
            "0" => &["ⁿ", "∅"],
            "-" => &["–", "—", "_", "·"],
            "+" => &["±"],
            "(" => &["<", "[", "{"],
            ")" => &[">", "]", "}"],
            "*" => &["†", "‡", "★"],
            "\"" => &["«", "»", "„", "“", "”"],
            "'" => &["‘", "’", "‚", "‹", "›"],
            "!" => &["¡"],
            "?" => &["¿", "‽"],
            "#" => &["№"],
            "%" => &["‰", "℅"],
            "=" => &["≠", "≈", "∞"],
            "/" => &["\\"],
            _ if s == cur => &["$", "€", "£", "¥", "₽", "₴", "₸", "₹", "¢", "₩"],
            _ => &[],
        };
        v.iter().filter(|x| **x != s).map(|x| x.to_string()).collect()
    };
    let mk = |v: &Vec<String>| -> Row {
        v.iter()
            .map(|s| {
                let mut k = ch(s, s);
                k.popup = pop(s);
                (k, 1.0)
            })
            .collect()
    };
    rows.push(mk(&r1));
    rows.push(mk(&r2));
    let mut last = mk(&r3);
    last.insert(0, (func(KeyKind::SymbolsMore, if page2 { "?123" } else { "=\\<" }), 1.5));
    last.push((func(KeyKind::Backspace, ""), 1.5));
    rows.push(last);
}

fn bottom_row(o: &BuildOptions) -> Row {
    let mut row: Row = Vec::new();
    let to_alpha = o.mode != Mode::Alphabet;
    row.push(if to_alpha {
        (func(KeyKind::Alphabet, &abc_label(o.lang)), 1.5)
    } else {
        (func(KeyKind::Symbols, "?123"), 1.5)
    });
    if o.multi_lang {
        row.push((func(KeyKind::Language, ""), 1.0));
    } else {
        row.push((func(KeyKind::Emoji, ""), 1.0));
    }
    let (comma, comma_pop): (&str, &[&str]) = if o.email {
        ("@", &[",", "_", "-"])
    } else if o.url {
        ("/", &[",", ":", "_", "-"])
    } else {
        (",", &[";", ":", "-", "\"", "(", ")", "'"])
    };
    let plain = !o.email && !o.url;
    let comma = match o.lang.script {
        "Arabic" if plain => "،",
        "CJK" if plain && o.lang.code == "ja" => "、",
        "CJK" if plain && o.lang.composer != Composer::Hangul => "，",
        _ => comma,
    };
    let mut c = ch(comma, comma);
    c.func = true;
    c.popup = comma_pop.iter().map(|s| s.to_string()).collect();
    if !o.multi_lang {
        // emoji live on the toolbar; long-press comma also opens them
        c.hint = None;
    }
    row.push((c, 1.0));
    let mut space = func(KeyKind::Space, o.lang.name);
    space.func = false;
    row.push((space, 4.5));
    let period = match o.lang.script {
        "Devanagari" | "Bengali" | "Gurmukhi" | "Oriya" if !o.email && !o.url => "।",
        "Ethiopic" => "።",
        "Myanmar" => "။",
        "CJK" if !o.email && !o.url && o.lang.composer != Composer::Hangul => "。",
        _ => ".",
    };
    let mut p = ch(period, period);
    p.func = true;
    let mut pp: Vec<String> = [".", "?", "!", ",", "…", ":", ";", "-", "'", "\"", "(", ")"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    pp.retain(|s| s != period);
    if o.url {
        pp.insert(0, ".ru".into());
        pp.insert(0, ".com".into());
    }
    p.popup = pp;
    row.push((p, 1.0));
    let mut enter = func(KeyKind::Enter, o.enter_label);
    enter.func = true;
    row.push((enter, 1.5));
    row
}

fn abc_label(l: &LangDef) -> String {
    let letters: String = l.rows.iter().flat_map(|r| r.iter()).filter(|s| s.chars().count() == 1).take(3).cloned().collect();
    let letters = if l.code == "ja" { "あいう".to_string() } else { letters };
    if l.caseless { letters } else { l.upper(&letters) }
}

/// Lays rows out: every row shares one key unit; rows are centered; function keys absorb slack
fn place(rows: Vec<Row>, f: Frame) -> Vec<Key> {
    let pad_x = 3.0 * f.dp;
    let gap_x = 5.0 * f.dp;
    let gap_y = 9.0 * f.dp;
    let avail = f.w - 2.0 * pad_x;
    let units = rows.iter().map(|r| r.iter().map(|k| k.1).sum::<f32>()).fold(10.0f32, f32::max);
    let unit = avail / units;
    let n = rows.len() as f32;
    let row_h = (f.h - 4.0 * f.dp) / n;
    let mut keys = Vec::new();
    for (ri, row) in rows.into_iter().enumerate() {
        let y = f.top + 2.0 * f.dp + ri as f32 * row_h;
        let total: f32 = row.iter().map(|k| k.1).sum();
        let has_func = row.iter().any(|k| k.0.func && k.1 > 1.0);
        let letters: f32 = row.iter().filter(|k| !(k.0.func && k.1 > 1.0)).map(|k| k.1).sum();
        // stretch wide function keys to fill the row; otherwise center the row
        let (func_scale, mut x) = if has_func && total < units {
            let fw: f32 = row.iter().filter(|k| k.0.func && k.1 > 1.0).map(|k| k.1).sum();
            ((units - letters) / fw, pad_x)
        } else {
            (1.0, pad_x + (units - total) * unit / 2.0)
        };
        let row_len = row.len();
        for (ci, (mut key, w)) in row.into_iter().enumerate() {
            let w = if key.func && w > 1.0 { w * func_scale } else { w } * unit;
            key.rect = Rect { x: x + gap_x / 2.0, y: y + gap_y / 2.0, w: w - gap_x, h: row_h - gap_y };
            // hit boxes cover the gaps and extend to the edges of the keyboard
            let hx0 = if ci == 0 { 0.0 } else { x };
            let hx1 = if ci + 1 == row_len { f.w } else { x + w };
            key.hit = Rect { x: hx0, y, w: hx1 - hx0, h: row_h };
            keys.push(key);
            x += w;
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_builds_and_has_unique_code() {
        let mut codes = std::collections::HashSet::new();
        for l in LANGUAGES {
            assert!(codes.insert(l.code), "duplicate {}", l.code);
            for shift in [ShiftState::Off, ShiftState::On] {
                let o = BuildOptions { lang: l, mode: Mode::Alphabet, shift, number_row: false, multi_lang: true, enter_label: "", email: false, url: false };
                let keys = build(&o, Frame { w: 1080.0, top: 120.0, h: 800.0, dp: 2.75 });
                assert!(keys.iter().filter(|k| k.letter).count() >= 15, "{}", l.code);
                for k in &keys {
                    assert!(k.rect.w > 20.0, "{} key {:?} too narrow", l.code, k.label);
                    assert!(k.rect.x >= 0.0 && k.rect.x + k.rect.w <= 1080.0, "{} {:?}", l.code, k.label);
                }
            }
        }
        assert!(LANGUAGES.len() > 100);
    }

    /// Every language's model and dictionary decode, and the model's words are typeable
    #[test]
    fn every_model_loads() {
        let dir = format!("{}/../app/src/main/assets", env!("CARGO_MANIFEST_DIR"));
        let mut done = std::collections::HashSet::new();
        for l in LANGUAGES {
            if !l.lm.is_empty() && done.insert(l.lm) {
                let b = std::fs::read(format!("{dir}/lm/{}.rlm", l.lm)).unwrap();
                let m = crate::engine::lm::LanguageModel::decode(&b).unwrap_or_else(|| panic!("{}", l.lm));
                // scripts without spaces between words, and Kurmanji (no Latin-script corpus),
                // only carry a short curated list
                let min = if matches!(l.script, "Thai" | "Khmer" | "Myanmar") || l.code == "ku" { 100 } else { 1000 };
                assert!(m.words.len() > min, "{} has {} words", l.lm, m.words.len());
            }
            if !l.conv.is_empty() && done.insert(l.conv) {
                let b = std::fs::read(format!("{dir}/cjk/{}.rcd", l.conv)).unwrap();
                assert!(crate::engine::cjk::ConvDict::decode(&b).unwrap().len() > 50_000, "{}", l.conv);
            }
        }
        let without: Vec<&str> = LANGUAGES.iter().filter(|l| l.lm.is_empty() && l.conv.is_empty()).map(|l| l.code).collect();
        assert!(without.len() <= 4, "{without:?}");
    }

    #[test]
    fn locale_matching() {
        assert_eq!(LangDef::for_locale("ru_RU").unwrap().code, "ru");
        assert_eq!(LangDef::for_locale("pt-BR").unwrap().code, "pt-BR");
        assert_eq!(LangDef::for_locale("nb-NO").unwrap().code, "no");
        assert_eq!(LangDef::for_locale("sr-Latn-RS").unwrap().code, "sr-Latn");
        assert_eq!(LangDef::for_locale("de-AT").unwrap().code, "de");
        assert!(LangDef::for_locale("xx").is_none());
        assert_eq!(LangDef::for_locale("zh-CN").unwrap().code, "zh-Hans");
        assert_eq!(LangDef::for_locale("zh-Hant-TW").unwrap().code, "zh-TW");
        assert_eq!(LangDef::for_locale("zh-HK").unwrap().code, "yue");
        assert_eq!(LangDef::for_locale("zh").unwrap().code, "zh-Hans");
    }

    #[test]
    fn soft_sign_shows_hard_sign_hint() {
        let ru = LangDef::by_code("ru").unwrap();
        let o = BuildOptions { lang: ru, mode: Mode::Alphabet, shift: ShiftState::Off, number_row: false, multi_lang: false, enter_label: "", email: false, url: false };
        let keys = build(&o, Frame { w: 1080.0, top: 120.0, h: 800.0, dp: 2.75 });
        let soft = keys.iter().find(|k| k.label == "ь").unwrap();
        assert_eq!(soft.hint.as_deref(), Some("ъ"));
        assert_eq!(soft.popup, ["ъ"]);
    }

    #[test]
    fn turkish_i() {
        let tr = LangDef::by_code("tr").unwrap();
        assert_eq!(tr.upper("i"), "İ");
        assert_eq!(tr.upper("ı"), "I");
        assert_eq!(LangDef::by_code("de").unwrap().upper("ß"), "ß");
    }
}
