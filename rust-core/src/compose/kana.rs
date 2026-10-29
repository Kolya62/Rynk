// SPDX-License-Identifier: GPL-3.0-or-later
//! Romaji → kana conversion (no kanji conversion).

const TABLE: &[(&str, &str)] = &[
    ("kya", "きゃ"), ("kyu", "きゅ"), ("kyo", "きょ"), ("sha", "しゃ"), ("shi", "し"), ("shu", "しゅ"), ("she", "しぇ"), ("sho", "しょ"),
    ("sya", "しゃ"), ("syu", "しゅ"), ("syo", "しょ"), ("cha", "ちゃ"), ("chi", "ち"), ("chu", "ちゅ"), ("che", "ちぇ"), ("cho", "ちょ"),
    ("tya", "ちゃ"), ("tyu", "ちゅ"), ("tyo", "ちょ"), ("tsu", "つ"), ("nya", "にゃ"), ("nyu", "にゅ"), ("nyo", "にょ"),
    ("hya", "ひゃ"), ("hyu", "ひゅ"), ("hyo", "ひょ"), ("mya", "みゃ"), ("myu", "みゅ"), ("myo", "みょ"), ("rya", "りゃ"),
    ("ryu", "りゅ"), ("ryo", "りょ"), ("gya", "ぎゃ"), ("gyu", "ぎゅ"), ("gyo", "ぎょ"), ("jya", "じゃ"), ("jyu", "じゅ"), ("jyo", "じょ"),
    ("bya", "びゃ"), ("byu", "びゅ"), ("byo", "びょ"), ("pya", "ぴゃ"), ("pyu", "ぴゅ"), ("pyo", "ぴょ"), ("dya", "ぢゃ"),
    ("thi", "てぃ"), ("dhi", "でぃ"), ("xtu", "っ"), ("ltu", "っ"), ("xya", "ゃ"), ("xyu", "ゅ"), ("xyo", "ょ"),
    ("ja", "じゃ"), ("ji", "じ"), ("ju", "じゅ"), ("je", "じぇ"), ("jo", "じょ"), ("fa", "ふぁ"), ("fi", "ふぃ"), ("fu", "ふ"), ("fe", "ふぇ"), ("fo", "ふぉ"),
    ("ka", "か"), ("ki", "き"), ("ku", "く"), ("ke", "け"), ("ko", "こ"), ("sa", "さ"), ("si", "し"), ("su", "す"), ("se", "せ"), ("so", "そ"),
    ("ta", "た"), ("ti", "ち"), ("tu", "つ"), ("te", "て"), ("to", "と"), ("na", "な"), ("ni", "に"), ("nu", "ぬ"), ("ne", "ね"), ("no", "の"),
    ("ha", "は"), ("hi", "ひ"), ("hu", "ふ"), ("he", "へ"), ("ho", "ほ"), ("ma", "ま"), ("mi", "み"), ("mu", "む"), ("me", "め"), ("mo", "も"),
    ("ya", "や"), ("yu", "ゆ"), ("yo", "よ"), ("ra", "ら"), ("ri", "り"), ("ru", "る"), ("re", "れ"), ("ro", "ろ"),
    ("wa", "わ"), ("wi", "うぃ"), ("we", "うぇ"), ("wo", "を"), ("ga", "が"), ("gi", "ぎ"), ("gu", "ぐ"), ("ge", "げ"), ("go", "ご"),
    ("za", "ざ"), ("zi", "じ"), ("zu", "ず"), ("ze", "ぜ"), ("zo", "ぞ"), ("da", "だ"), ("di", "ぢ"), ("du", "づ"), ("de", "で"), ("do", "ど"),
    ("ba", "ば"), ("bi", "び"), ("bu", "ぶ"), ("be", "べ"), ("bo", "ぼ"), ("pa", "ぱ"), ("pi", "ぴ"), ("pu", "ぷ"), ("pe", "ぺ"), ("po", "ぽ"),
    ("va", "ゔぁ"), ("vi", "ゔぃ"), ("vu", "ゔ"), ("ve", "ゔぇ"), ("vo", "ゔぉ"), ("la", "ぁ"), ("li", "ぃ"), ("lu", "ぅ"), ("le", "ぇ"), ("lo", "ぉ"),
    ("xa", "ぁ"), ("xi", "ぃ"), ("xu", "ぅ"), ("xe", "ぇ"), ("xo", "ぉ"), ("nn", "ん"), ("n'", "ん"),
    ("a", "あ"), ("i", "い"), ("u", "う"), ("e", "え"), ("o", "お"), ("-", "ー"),
];

/// Converts romaji to hiragana; unfinished trailing letters stay Latin
pub fn to_hiragana(romaji: &str, finish: bool) -> String {
    let s: Vec<char> = romaji.to_lowercase().chars().collect();
    let mut out = String::new();
    let mut i = 0;
    'outer: while i < s.len() {
        // sokuon: doubled consonant
        if i + 1 < s.len() && s[i] == s[i + 1] && s[i].is_ascii_alphabetic() && !"aeioun".contains(s[i]) {
            out.push('っ');
            i += 1;
            continue;
        }
        for len in (1..=3).rev() {
            if i + len > s.len() {
                continue;
            }
            let chunk: String = s[i..i + len].iter().collect();
            if let Some((_, k)) = TABLE.iter().find(|t| t.0 == chunk) {
                out.push_str(k);
                i += len;
                continue 'outer;
            }
        }
        // n before a consonant
        if s[i] == 'n' && i + 1 < s.len() && !"aeiouy".contains(s[i + 1]) {
            out.push('ん');
            i += 1;
            continue;
        }
        if s[i] == 'n' && i + 1 == s.len() && finish {
            out.push('ん');
            i += 1;
            continue;
        }
        if !s[i].is_ascii_alphabetic() || i + 3 <= s.len() || finish {
            out.push(s[i]);
            i += 1;
        } else {
            // pending letters of a syllable still being typed
            out.extend(&s[i..]);
            break;
        }
    }
    out
}

pub fn to_katakana(hira: &str) -> String {
    hira.chars()
        .map(|c| if ('ぁ'..='ゖ').contains(&c) { char::from_u32(c as u32 + 0x60).unwrap_or(c) } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts() {
        assert_eq!(to_hiragana("konnnichiha", true), "こんにちは");
        assert_eq!(to_hiragana("arigatou", true), "ありがとう");
        assert_eq!(to_hiragana("kitte", true), "きって");
        assert_eq!(to_hiragana("shinbun", true), "しんぶん");
        assert_eq!(to_hiragana("ky", false), "ky");
        assert_eq!(to_katakana("こーひー"), "コーヒー");
    }
}
