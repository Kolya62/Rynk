// SPDX-License-Identifier: GPL-3.0-or-later
//! Visual order for text egui draws without a shaping engine: Arabic letters joined through
//! presentation forms, right-to-left text reversed, Indic pre-base vowel signs moved in front.

/// (letter, isolated, final, initial, medial); 0 = no such form (the letter joins on one side)
const ARABIC: &[(char, u32, u32, u32, u32)] = &[
    ('آ', 0xFE81, 0xFE82, 0, 0), ('أ', 0xFE83, 0xFE84, 0, 0), ('ؤ', 0xFE85, 0xFE86, 0, 0), ('إ', 0xFE87, 0xFE88, 0, 0),
    ('ئ', 0xFE89, 0xFE8A, 0xFE8B, 0xFE8C), ('ا', 0xFE8D, 0xFE8E, 0, 0), ('ب', 0xFE8F, 0xFE90, 0xFE91, 0xFE92),
    ('ة', 0xFE93, 0xFE94, 0, 0), ('ت', 0xFE95, 0xFE96, 0xFE97, 0xFE98), ('ث', 0xFE99, 0xFE9A, 0xFE9B, 0xFE9C),
    ('ج', 0xFE9D, 0xFE9E, 0xFE9F, 0xFEA0), ('ح', 0xFEA1, 0xFEA2, 0xFEA3, 0xFEA4), ('خ', 0xFEA5, 0xFEA6, 0xFEA7, 0xFEA8),
    ('د', 0xFEA9, 0xFEAA, 0, 0), ('ذ', 0xFEAB, 0xFEAC, 0, 0), ('ر', 0xFEAD, 0xFEAE, 0, 0), ('ز', 0xFEAF, 0xFEB0, 0, 0),
    ('س', 0xFEB1, 0xFEB2, 0xFEB3, 0xFEB4), ('ش', 0xFEB5, 0xFEB6, 0xFEB7, 0xFEB8), ('ص', 0xFEB9, 0xFEBA, 0xFEBB, 0xFEBC),
    ('ض', 0xFEBD, 0xFEBE, 0xFEBF, 0xFEC0), ('ط', 0xFEC1, 0xFEC2, 0xFEC3, 0xFEC4), ('ظ', 0xFEC5, 0xFEC6, 0xFEC7, 0xFEC8),
    ('ع', 0xFEC9, 0xFECA, 0xFECB, 0xFECC), ('غ', 0xFECD, 0xFECE, 0xFECF, 0xFED0), ('ف', 0xFED1, 0xFED2, 0xFED3, 0xFED4),
    ('ق', 0xFED5, 0xFED6, 0xFED7, 0xFED8), ('ك', 0xFED9, 0xFEDA, 0xFEDB, 0xFEDC), ('ل', 0xFEDD, 0xFEDE, 0xFEDF, 0xFEE0),
    ('م', 0xFEE1, 0xFEE2, 0xFEE3, 0xFEE4), ('ن', 0xFEE5, 0xFEE6, 0xFEE7, 0xFEE8), ('ه', 0xFEE9, 0xFEEA, 0xFEEB, 0xFEEC),
    ('و', 0xFEED, 0xFEEE, 0, 0), ('ى', 0xFEEF, 0xFEF0, 0xFBE8, 0xFBE9), ('ي', 0xFEF1, 0xFEF2, 0xFEF3, 0xFEF4),
    ('پ', 0xFB56, 0xFB57, 0xFB58, 0xFB59), ('چ', 0xFB7A, 0xFB7B, 0xFB7C, 0xFB7D), ('ژ', 0xFB8A, 0xFB8B, 0, 0),
    ('گ', 0xFB92, 0xFB93, 0xFB94, 0xFB95), ('ک', 0xFB8E, 0xFB8F, 0xFB90, 0xFB91), ('ی', 0xFBFC, 0xFBFD, 0xFBFE, 0xFBFF),
    ('ٹ', 0xFB66, 0xFB67, 0xFB68, 0xFB69), ('ڈ', 0xFB88, 0xFB89, 0, 0), ('ڑ', 0xFB8C, 0xFB8D, 0, 0), ('ں', 0xFB9E, 0xFB9F, 0, 0),
    ('ہ', 0xFBA6, 0xFBA7, 0xFBA8, 0xFBA9), ('ھ', 0xFBAA, 0xFBAB, 0xFBAC, 0xFBAD), ('ے', 0xFBAE, 0xFBAF, 0, 0),
    ('ۓ', 0xFBB0, 0xFBB1, 0, 0), ('ۀ', 0xFBA4, 0xFBA5, 0, 0), ('ڤ', 0xFB6A, 0xFB6B, 0xFB6C, 0xFB6D), ('ڭ', 0xFBD3, 0xFBD4, 0xFBD5, 0xFBD6),
    ('ۆ', 0xFBD9, 0xFBDA, 0, 0), ('ۇ', 0xFBD7, 0xFBD8, 0, 0), ('ۋ', 0xFBDE, 0xFBDF, 0, 0), ('ې', 0xFBE4, 0xFBE5, 0xFBE6, 0xFBE7),
];

#[derive(Clone, Copy, PartialEq)]
enum Join {
    /// Joins on both sides
    Dual,
    /// Joins only to the letter before it (alef, dal, reh, waw…)
    Right,
    /// Harakat and other marks: ignored when looking for neighbours
    Transparent,
    None,
}

fn join(c: char) -> Join {
    if matches!(c as u32, 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06ED) {
        return Join::Transparent;
    }
    if let Some(e) = ARABIC.iter().find(|e| e.0 == c) {
        return if e.3 != 0 { Join::Dual } else { Join::Right };
    }
    // other Arabic-script letters without presentation forms still join (Pashto, Sorani…)
    match c {
        'ټ' | 'ځ' | 'څ' | 'ښ' | 'ګ' | 'ڼ' | 'ڵ' | 'ێ' | 'ۍ' => Join::Dual,
        'ډ' | 'ړ' | 'ږ' | 'ڕ' | 'ە' | 'ٱ' => Join::Right,
        _ => Join::None,
    }
}

fn is_rtl(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF)
}

fn shape_arabic(chars: &[char]) -> Vec<char> {
    let n = chars.len();
    let prev_joins = |i: usize| (0..i).rev().map(|j| chars[j]).find(|&c| join(c) != Join::Transparent).is_some_and(|c| join(c) == Join::Dual);
    let next_joins = |i: usize| (i + 1..n).map(|j| chars[j]).find(|&c| join(c) != Join::Transparent).is_some_and(|c| matches!(join(c), Join::Dual | Join::Right));
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        let c = chars[i];
        let jt = join(c);
        let p = matches!(jt, Join::Dual | Join::Right) && prev_joins(i);
        // lam + alef ligatures
        if c == 'ل' {
            if let Some(&a) = chars.get(i + 1) {
                let lig = match a {
                    'آ' => Some(0xFEF5),
                    'أ' => Some(0xFEF7),
                    'إ' => Some(0xFEF9),
                    'ا' => Some(0xFEFB),
                    _ => None,
                };
                if let Some(l) = lig {
                    out.push(char::from_u32(l + p as u32).unwrap_or(c));
                    i += 2;
                    continue;
                }
            }
        }
        let nx = jt == Join::Dual && next_joins(i);
        let shaped = ARABIC.iter().find(|e| e.0 == c).and_then(|e| {
            let code = match (p, nx) {
                (false, false) => e.1,
                (true, false) => e.2,
                (false, true) => e.3,
                (true, true) => e.4,
            };
            char::from_u32(code).filter(|_| code != 0)
        });
        out.push(shaped.unwrap_or(c));
        i += 1;
    }
    out
}

/// Pre-base vowel signs that are typed after the consonant but drawn before it
fn pre_base(c: char) -> bool {
    matches!(c as u32,
        0x093F | 0x09BF | 0x09C7 | 0x09C8 | 0x0A3F | 0x0ABF | 0x0B47 | 0x0BC6..=0x0BC8 | 0x0D46..=0x0D48 | 0x0DD9..=0x0DDB | 0x1031 | 0x17C1..=0x17C3)
}

fn is_virama(c: char) -> bool {
    matches!(c as u32, 0x094D | 0x09CD | 0x0A4D | 0x0ACD | 0x0B4D | 0x0BCD | 0x0C4D | 0x0CCD | 0x0D4D | 0x0DCA | 0x1039 | 0x17D2)
}

/// Text in the order egui should draw it
pub fn visual(text: &str) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    // pre-base vowel signs go in front of their consonant cluster
    let mut i = 0;
    while i < chars.len() {
        if pre_base(chars[i]) && i > 0 {
            let mut start = i - 1;
            while start >= 2 && is_virama(chars[start - 1]) {
                start -= 2;
            }
            let m = chars.remove(i);
            chars.insert(start, m);
        }
        i += 1;
    }
    if !chars.iter().any(|&c| is_rtl(c)) {
        return chars.into_iter().collect();
    }
    let shaped = shape_arabic(&chars);
    // reverse by clusters so marks stay on their letter
    let mut clusters: Vec<Vec<char>> = Vec::new();
    for c in shaped {
        let mark = matches!(c as u32, 0x0591..=0x05C7 | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06ED);
        match clusters.last_mut() {
            Some(cl) if mark => cl.push(c),
            _ => clusters.push(vec![c]),
        }
    }
    clusters.reverse();
    clusters.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arabic_joins_and_reverses() {
        // سلام: seen initial, lam-alef final ligature, meem isolated — drawn right to left
        let v: Vec<u32> = visual("سلام").chars().map(|c| c as u32).collect();
        assert_eq!(v, vec![0xFEE1, 0xFEFC, 0xFEB3]);
        assert_eq!(visual("שלום"), "םולש");
    }

    #[test]
    fn indic_pre_base() {
        assert_eq!(visual("कि"), "िक");
        // a conjunct moves as one: स्कि → ि स्क
        assert_eq!(visual("स्कि"), "िस्क");
        assert_eq!(visual("hello"), "hello");
    }
}
