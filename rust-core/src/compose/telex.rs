// SPDX-License-Identifier: GPL-3.0-or-later
//! Vietnamese Telex: aa→â, aw→ă, ee→ê, oo→ô, ow→ơ, uw→ư, dd→đ; tones s f r x j, z clears.

/// Vowels with their six tone forms: none, sắc, huyền, hỏi, ngã, nặng
const VOWELS: &[[char; 6]] = &[
    ['a', 'á', 'à', 'ả', 'ã', 'ạ'], ['ă', 'ắ', 'ằ', 'ẳ', 'ẵ', 'ặ'], ['â', 'ấ', 'ầ', 'ẩ', 'ẫ', 'ậ'],
    ['e', 'é', 'è', 'ẻ', 'ẽ', 'ẹ'], ['ê', 'ế', 'ề', 'ể', 'ễ', 'ệ'], ['i', 'í', 'ì', 'ỉ', 'ĩ', 'ị'],
    ['o', 'ó', 'ò', 'ỏ', 'õ', 'ọ'], ['ô', 'ố', 'ồ', 'ổ', 'ỗ', 'ộ'], ['ơ', 'ớ', 'ờ', 'ở', 'ỡ', 'ợ'],
    ['u', 'ú', 'ù', 'ủ', 'ũ', 'ụ'], ['ư', 'ứ', 'ừ', 'ử', 'ữ', 'ự'], ['y', 'ý', 'ỳ', 'ỷ', 'ỹ', 'ỵ'],
];

/// (base vowel index, tone) of a lowercase vowel
fn split(c: char) -> Option<(usize, usize)> {
    VOWELS.iter().enumerate().find_map(|(v, forms)| forms.iter().position(|&f| f == c).map(|t| (v, t)))
}

fn make(v: usize, tone: usize, upper: bool) -> char {
    let c = VOWELS[v][tone];
    if upper { c.to_uppercase().next().unwrap_or(c) } else { c }
}

fn is_vowel(c: char) -> bool {
    split(c.to_lowercase().next().unwrap_or(c)).is_some()
}

/// Index (in chars) of the vowel that carries the tone
fn tone_position(w: &[char]) -> Option<usize> {
    let vs: Vec<usize> = (0..w.len()).filter(|&i| is_vowel(w[i])).collect();
    let (&first, &last) = (vs.first()?, vs.last()?);
    // the vowel group is the last run of vowels
    let start = (0..=last).rev().take_while(|&i| is_vowel(w[i])).last().unwrap_or(first);
    let group: Vec<usize> = (start..=last).collect();
    let lower = |i: usize| w[i].to_lowercase().next().unwrap_or(w[i]);
    // ê, ơ (and â, ă, ô, ư) take the mark
    if let Some(&i) = group.iter().rev().find(|&&i| matches!(split(lower(i)).map(|x| x.0), Some(1 | 2 | 4 | 7 | 8 | 10))) {
        return Some(i);
    }
    let ends_with_consonant = last + 1 < w.len();
    let s: String = group.iter().map(|&i| lower(i)).collect();
    // "gi" and "qu" are consonants
    let group = if group.len() > 1 && start > 0 && matches!((lower(start - 1), lower(start)), ('g', 'i') | ('q', 'u')) { group[1..].to_vec() } else { group };
    Some(match group.len() {
        1 => group[0],
        2 if ends_with_consonant || matches!(s.as_str(), "oa" | "oe" | "uy" | "uê") => group[1],
        2 => group[0],
        _ => group[1],
    })
}

/// The word after typing `key` on it, when `key` is a Telex modifier; None = type it normally
pub fn apply(word: &str, key: char) -> Option<String> {
    let k = key.to_ascii_lowercase();
    let mut w: Vec<char> = word.chars().collect();
    if w.is_empty() {
        return None;
    }
    let lower = |c: char| c.to_lowercase().next().unwrap_or(c);
    match k {
        's' | 'f' | 'r' | 'x' | 'j' | 'z' => {
            let tone = match k {
                's' => 1,
                'f' => 2,
                'r' => 3,
                'x' => 4,
                'j' => 5,
                _ => 0,
            };
            let i = tone_position(&w)?;
            let (v, old) = split(lower(w[i]))?;
            if old == tone {
                if tone == 0 {
                    return None;
                }
                // pressing the same tone again cancels it and types the letter
                w[i] = make(v, 0, w[i].is_uppercase());
                w.push(key);
                return Some(w.into_iter().collect());
            }
            w[i] = make(v, tone, w[i].is_uppercase());
            Some(w.into_iter().collect())
        }
        'a' | 'e' | 'o' => {
            // doubled vowel → circumflex, on the last matching vowel of the word
            let target = match k {
                'a' => 0,
                'e' => 3,
                _ => 6,
            };
            let hat = target + if k == 'a' { 2 } else { 1 };
            let i = (0..w.len()).rev().find(|&i| split(lower(w[i])).is_some_and(|(v, _)| v == target || v == hat))?;
            if i + 3 < w.len() {
                return None;
            }
            let (v, t) = split(lower(w[i]))?;
            if v == hat {
                // "aaa" → "aa"
                w[i] = make(target, t, w[i].is_uppercase());
                w.push(key);
            } else {
                w[i] = make(hat, t, w[i].is_uppercase());
            }
            Some(w.into_iter().collect())
        }
        'w' => {
            // uo + w → ươ; otherwise the last a/o/u gets its horn or breve
            let n = w.len();
            if let Some(i) = (0..n.saturating_sub(1)).rev().find(|&i| lower(w[i]) == 'u' && split(lower(w[i + 1])).is_some_and(|x| x.0 == 6)) {
                let (_, t1) = split(lower(w[i]))?;
                let (_, t2) = split(lower(w[i + 1]))?;
                w[i] = make(10, t1, w[i].is_uppercase());
                w[i + 1] = make(8, t2, w[i + 1].is_uppercase());
                return Some(w.into_iter().collect());
            }
            let i = (0..n).rev().find(|&i| split(lower(w[i])).is_some_and(|(v, _)| matches!(v, 0 | 6 | 9)))?;
            let (v, t) = split(lower(w[i]))?;
            let nv = match v {
                0 => 1,
                6 => 8,
                _ => 10,
            };
            w[i] = make(nv, t, w[i].is_uppercase());
            Some(w.into_iter().collect())
        }
        'd' => {
            let i = (0..w.len()).rev().find(|&i| lower(w[i]) == 'd')?;
            w[i] = if w[i].is_uppercase() { 'Đ' } else { 'đ' };
            Some(w.into_iter().collect())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typ(s: &str) -> String {
        let mut w = String::new();
        for c in s.chars() {
            w = apply(&w, c).unwrap_or_else(|| format!("{w}{c}"));
        }
        w
    }

    #[test]
    fn telex() {
        assert_eq!(typ("vieetj"), "việt");
        assert_eq!(typ("nguowif"), "người");
        assert_eq!(typ("ddaays"), "đấy");
        assert_eq!(typ("hoaf"), "hoà");
        assert_eq!(typ("tooi"), "tôi");
        assert_eq!(typ("Vieetj"), "Việt");
        assert_eq!(typ("hocj"), "học");
        assert_eq!(typ("quas"), "quá");
        assert_eq!(typ("gias"), "giá");
        assert_eq!(typ("ass"), "as");
        assert_eq!(typ("tieengs"), "tiếng");
    }
}
