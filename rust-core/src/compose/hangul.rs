// SPDX-License-Identifier: GPL-3.0-or-later
//! Dubeolsik Hangul automaton: jamo keys → syllable blocks.

const CHO: &[char] = &['ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ'];
const JUNG: &[char] = &['ㅏ', 'ㅐ', 'ㅑ', 'ㅒ', 'ㅓ', 'ㅔ', 'ㅕ', 'ㅖ', 'ㅗ', 'ㅘ', 'ㅙ', 'ㅚ', 'ㅛ', 'ㅜ', 'ㅝ', 'ㅞ', 'ㅟ', 'ㅠ', 'ㅡ', 'ㅢ', 'ㅣ'];
const JONG: &[char] = &[
    '\0', 'ㄱ', 'ㄲ', 'ㄳ', 'ㄴ', 'ㄵ', 'ㄶ', 'ㄷ', 'ㄹ', 'ㄺ', 'ㄻ', 'ㄼ', 'ㄽ', 'ㄾ', 'ㄿ', 'ㅀ', 'ㅁ', 'ㅂ', 'ㅄ', 'ㅅ', 'ㅆ',
    'ㅇ', 'ㅈ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ',
];
const VOWEL_PAIRS: &[(char, char, char)] = &[
    ('ㅗ', 'ㅏ', 'ㅘ'), ('ㅗ', 'ㅐ', 'ㅙ'), ('ㅗ', 'ㅣ', 'ㅚ'), ('ㅜ', 'ㅓ', 'ㅝ'), ('ㅜ', 'ㅔ', 'ㅞ'), ('ㅜ', 'ㅣ', 'ㅟ'), ('ㅡ', 'ㅣ', 'ㅢ'),
];
const FINAL_PAIRS: &[(char, char, char)] = &[
    ('ㄱ', 'ㅅ', 'ㄳ'), ('ㄴ', 'ㅈ', 'ㄵ'), ('ㄴ', 'ㅎ', 'ㄶ'), ('ㄹ', 'ㄱ', 'ㄺ'), ('ㄹ', 'ㅁ', 'ㄻ'), ('ㄹ', 'ㅂ', 'ㄼ'),
    ('ㄹ', 'ㅅ', 'ㄽ'), ('ㄹ', 'ㅌ', 'ㄾ'), ('ㄹ', 'ㅍ', 'ㄿ'), ('ㄹ', 'ㅎ', 'ㅀ'), ('ㅂ', 'ㅅ', 'ㅄ'),
];

fn pair(t: &[(char, char, char)], a: char, b: char) -> Option<char> {
    t.iter().find(|p| p.0 == a && p.1 == b).map(|p| p.2)
}
fn split(t: &[(char, char, char)], c: char) -> Option<(char, char)> {
    t.iter().find(|p| p.2 == c).map(|p| (p.0, p.1))
}

pub fn is_jamo(c: char) -> bool {
    CHO.contains(&c) || JUNG.contains(&c)
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Hangul {
    cho: Option<char>,
    jung: Option<char>,
    jong: Option<char>,
}

impl Hangul {
    pub fn is_empty(&self) -> bool {
        self.cho.is_none() && self.jung.is_none() && self.jong.is_none()
    }

    /// The syllable being composed
    pub fn text(&self) -> String {
        match (self.cho, self.jung) {
            (Some(c), Some(v)) => {
                let ci = CHO.iter().position(|&x| x == c).unwrap() as u32;
                let vi = JUNG.iter().position(|&x| x == v).unwrap() as u32;
                let ki = self.jong.and_then(|j| JONG.iter().position(|&x| x == j)).unwrap_or(0) as u32;
                char::from_u32(0xAC00 + (ci * 21 + vi) * 28 + ki).map(String::from).unwrap_or_default()
            }
            _ => [self.cho, self.jung, self.jong].iter().flatten().collect(),
        }
    }

    fn take(&mut self) -> String {
        let t = self.text();
        *self = Hangul::default();
        t
    }

    /// Feeds a jamo; returns text that became final
    pub fn input(&mut self, c: char) -> String {
        let vowel = JUNG.contains(&c);
        if !vowel && !CHO.contains(&c) {
            let mut t = self.take();
            t.push(c);
            return t;
        }
        match (self.cho, self.jung, self.jong, vowel) {
            (None, None, None, false) => {
                self.cho = Some(c);
                String::new()
            }
            (None, None, None, true) => {
                self.jung = Some(c);
                String::new()
            }
            (Some(_), None, None, false) => {
                let t = self.take();
                self.cho = Some(c);
                t
            }
            (Some(_), None, None, true) => {
                self.jung = Some(c);
                String::new()
            }
            (Some(_), Some(_), None, false) if JONG.contains(&c) => {
                self.jong = Some(c);
                String::new()
            }
            (_, Some(v), None, true) => {
                if let Some(p) = pair(VOWEL_PAIRS, v, c) {
                    self.jung = Some(p);
                    String::new()
                } else {
                    let t = self.take();
                    self.jung = Some(c);
                    t
                }
            }
            (Some(_), Some(_), Some(j), false) => {
                if let Some(p) = pair(FINAL_PAIRS, j, c) {
                    self.jong = Some(p);
                    String::new()
                } else {
                    let t = self.take();
                    self.cho = Some(c);
                    t
                }
            }
            (Some(_), Some(_), Some(j), true) => {
                // the final consonant moves to the next syllable
                let (keep, moved) = match split(FINAL_PAIRS, j) {
                    Some((a, b)) => (Some(a), b),
                    None => (None, j),
                };
                self.jong = keep;
                let t = self.take();
                self.cho = Some(moved);
                self.jung = Some(c);
                t
            }
            _ => {
                let t = self.take();
                if vowel {
                    self.jung = Some(c);
                } else {
                    self.cho = Some(c);
                }
                t
            }
        }
    }

    /// Removes the last jamo; false when there was nothing to remove
    pub fn backspace(&mut self) -> bool {
        if let Some(j) = self.jong {
            self.jong = split(FINAL_PAIRS, j).map(|p| p.0);
        } else if let Some(v) = self.jung {
            self.jung = split(VOWEL_PAIRS, v).map(|p| p.0);
        } else if self.cho.is_some() {
            self.cho = None;
        } else {
            return false;
        }
        true
    }

    pub fn finish(&mut self) -> String {
        self.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typ(s: &str) -> String {
        let mut h = Hangul::default();
        let mut out = String::new();
        for c in s.chars() {
            out += &h.input(c);
        }
        out + &h.finish()
    }

    #[test]
    fn composes() {
        assert_eq!(typ("ㅎㅏㄴㄱㅜㄱㅇㅓ"), "한국어");
        assert_eq!(typ("ㅇㅏㄴㄴㅕㅇ"), "안녕");
        assert_eq!(typ("ㄷㅏㄹㄱㅇㅣ"), "닭이");
        assert_eq!(typ("ㄷㅏㄹㄱㅣ"), "달기");
        assert_eq!(typ("ㄱㅗㅏ"), "과");
    }

    #[test]
    fn backspace_decomposes() {
        let mut h = Hangul::default();
        for c in "ㄷㅏㄹㄱ".chars() {
            h.input(c);
        }
        h.backspace();
        assert_eq!(h.text(), "달");
    }
}
