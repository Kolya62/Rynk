// SPDX-License-Identifier: GPL-3.0-or-later
//! Zhuyin (bopomofo) → toneless pinyin, syllable by syllable.

const INITIALS: &[(char, &str)] = &[
    ('ㄅ', "b"), ('ㄆ', "p"), ('ㄇ', "m"), ('ㄈ', "f"), ('ㄉ', "d"), ('ㄊ', "t"), ('ㄋ', "n"), ('ㄌ', "l"),
    ('ㄍ', "g"), ('ㄎ', "k"), ('ㄏ', "h"), ('ㄐ', "j"), ('ㄑ', "q"), ('ㄒ', "x"), ('ㄓ', "zh"), ('ㄔ', "ch"),
    ('ㄕ', "sh"), ('ㄖ', "r"), ('ㄗ', "z"), ('ㄘ', "c"), ('ㄙ', "s"),
];
const MEDIALS: &[char] = &['ㄧ', 'ㄨ', 'ㄩ'];
const FINALS: &[(char, &str)] = &[
    ('ㄚ', "a"), ('ㄛ', "o"), ('ㄜ', "e"), ('ㄝ', "e"), ('ㄞ', "ai"), ('ㄟ', "ei"), ('ㄠ', "ao"), ('ㄡ', "ou"),
    ('ㄢ', "an"), ('ㄣ', "en"), ('ㄤ', "ang"), ('ㄥ', "eng"), ('ㄦ', "er"),
];
const TONES: &[char] = &['ˊ', 'ˇ', 'ˋ', '˙', 'ˉ'];

pub fn is_zhuyin(c: char) -> bool {
    INITIALS.iter().any(|x| x.0 == c) || MEDIALS.contains(&c) || FINALS.iter().any(|x| x.0 == c) || TONES.contains(&c)
}

#[derive(Default, Clone, Copy)]
struct Syl {
    i: Option<char>,
    m: Option<char>,
    f: Option<char>,
    len: usize,
}

impl Syl {
    fn empty(&self) -> bool {
        self.i.is_none() && self.m.is_none() && self.f.is_none()
    }

    fn pinyin(&self) -> String {
        let ini = self.i.and_then(|c| INITIALS.iter().find(|x| x.0 == c)).map_or("", |x| x.1);
        let fin = self.f.and_then(|c| FINALS.iter().find(|x| x.0 == c)).map_or("", |x| x.1);
        let rhyme: String = match (self.m, self.f) {
            (None, None) => {
                return if matches!(ini, "zh" | "ch" | "sh" | "r" | "z" | "c" | "s") { format!("{ini}i") } else { ini.to_string() };
            }
            (None, Some(_)) => fin.to_string(),
            (Some('ㄧ'), f) => match f.map(|_| fin) {
                None => "i".into(),
                Some("ou") => "iu".into(),
                Some("en") => "in".into(),
                Some("eng") => "ing".into(),
                Some(x) => format!("i{x}"),
            },
            (Some('ㄨ'), f) => match f.map(|_| fin) {
                None => "u".into(),
                Some("ei") => "ui".into(),
                Some("en") => "un".into(),
                Some("eng") => "ong".into(),
                Some(x) => format!("u{x}"),
            },
            (Some(_), f) => match f.map(|_| fin) {
                None => "v".into(),
                Some("en") => "vn".into(),
                Some("eng") => "iong".into(),
                Some(x) => format!("v{x}"),
            },
        };
        if ini.is_empty() {
            // zero-initial spellings
            return match rhyme.as_str() {
                "i" => "yi".into(),
                "iu" => "you".into(),
                "in" => "yin".into(),
                "ing" => "ying".into(),
                "u" => "wu".into(),
                "ui" => "wei".into(),
                "un" => "wen".into(),
                "ong" => "weng".into(),
                "iong" => "yong".into(),
                r if r.starts_with('v') => format!("yu{}", &r[1..]),
                r if r.starts_with('i') => format!("y{}", &r[1..]),
                r if r.starts_with('u') => format!("w{}", &r[1..]),
                r => r.to_string(),
            };
        }
        // ü is written u after j q x
        let rhyme = if matches!(ini, "j" | "q" | "x") && rhyme.starts_with('v') { format!("u{}", &rhyme[1..]) } else { rhyme };
        let rhyme = if matches!(ini, "j" | "q" | "x") && rhyme == "iong" { "iong".into() } else { rhyme };
        format!("{ini}{rhyme}")
    }
}

/// Splits bopomofo input into syllables: (pinyin, number of input symbols)
pub fn syllables(input: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut s = Syl::default();
    let close = |s: &mut Syl, out: &mut Vec<(String, usize)>| {
        if !s.empty() {
            out.push((s.pinyin(), s.len));
        }
        *s = Syl::default();
    };
    for c in input.chars() {
        if TONES.contains(&c) {
            s.len += 1;
            close(&mut s, &mut out);
            continue;
        }
        if INITIALS.iter().any(|x| x.0 == c) {
            close(&mut s, &mut out);
            s.i = Some(c);
        } else if MEDIALS.contains(&c) {
            if s.m.is_some() || s.f.is_some() {
                close(&mut s, &mut out);
            }
            s.m = Some(c);
        } else if FINALS.iter().any(|x| x.0 == c) {
            if s.f.is_some() {
                close(&mut s, &mut out);
            }
            s.f = Some(c);
        } else {
            close(&mut s, &mut out);
            continue;
        }
        s.len += 1;
    }
    close(&mut s, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn py(s: &str) -> String {
        syllables(s).into_iter().map(|x| x.0).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn converts() {
        assert_eq!(py("ㄋㄧˇㄏㄠˇ"), "ni hao");
        assert_eq!(py("ㄓㄨㄥㄍㄨㄛˊ"), "zhong guo");
        assert_eq!(py("ㄒㄩㄝˊㄕˋ"), "xue shi");
        assert_eq!(py("ㄧㄡˇㄩˊ"), "you yu");
        assert_eq!(py("ㄌㄩˋ"), "lv");
        assert_eq!(py("ㄨㄛˇㄇㄣ˙"), "wo men");
        assert_eq!(py("ㄐㄩㄥ"), "jiong");
        assert_eq!(py("ㄓ"), "zhi");
        assert_eq!(syllables("ㄋㄧˇㄏㄠ")[0].1, 3);
    }
}
