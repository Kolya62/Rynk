// SPDX-License-Identifier: GPL-3.0-or-later
//! Spelling rules: mistakes people make knowing the word but not its spelling.
//!
//! Tap decoding covers slips of the finger (a neighbouring key, a missed or extra letter).
//! These rules cover what the writer typed on purpose: `typed` stands for `meant` in the
//! dictionary word, at a cost in log10 units comparable to tap costs. The search applies a rule
//! where the touched letters spell `typed`.

/// (typed, meant, cost)
pub type Rule = (&'static str, &'static str, f32);

const RU: &[Rule] = &[
    // -тся / -ться
    ("тся", "ться", 0.6),
    ("ться", "тся", 0.6),
    // unstressed vowels
    ("а", "о", 1.1),
    ("о", "а", 1.1),
    ("е", "и", 1.1),
    ("и", "е", 1.1),
    ("я", "е", 1.3),
    ("е", "я", 1.3),
    // spelling of sibilants: жи-ши, ча-ща, чу-щу
    ("жы", "жи", 0.4),
    ("шы", "ши", 0.4),
    ("чя", "ча", 0.4),
    ("щя", "ща", 0.4),
    ("чю", "чу", 0.4),
    ("щю", "щу", 0.4),
    ("ш", "щ", 1.0),
    ("щ", "ш", 1.0),
    // doubled consonants
    ("н", "нн", 0.7),
    ("нн", "н", 0.7),
    ("с", "сс", 0.8),
    ("сс", "с", 0.8),
    ("л", "лл", 0.9),
    ("т", "тт", 0.9),
    ("м", "мм", 0.9),
    ("р", "рр", 0.9),
    ("к", "кк", 0.9),
    ("ф", "фф", 0.9),
    // soft and hard signs
    ("", "ь", 1.0),
    ("ь", "", 1.1),
    ("ь", "ъ", 0.6),
    ("ъ", "ь", 0.8),
    ("е", "ъе", 1.0),
    // unpronounced consonants: честный, поздно, солнце, чувство
    ("сн", "стн", 0.8),
    ("зн", "здн", 0.8),
    ("нц", "лнц", 0.9),
    ("ств", "вств", 0.9),
    // voiced and voiceless pairs, prefixes без-/бес-
    ("с", "з", 1.3),
    ("з", "с", 1.3),
    ("д", "т", 1.4),
    ("т", "д", 1.4),
    ("г", "к", 1.4),
    ("к", "г", 1.4),
    ("б", "п", 1.4),
    ("п", "б", 1.4),
    ("ж", "ш", 1.4),
    ("ш", "ж", 1.4),
    ("ф", "в", 1.4),
    ("в", "ф", 1.4),
    // ц / тс / тьс
    ("ц", "тс", 1.0),
    ("тс", "ц", 1.0),
    ("ца", "тся", 1.0),
    ("ца", "ться", 1.1),
];

const EN: &[Rule] = &[
    ("ie", "ei", 0.8),
    ("ei", "ie", 0.8),
    ("", "e", 1.2),
    ("e", "", 1.2),
    ("s", "ss", 0.8),
    ("ss", "s", 0.8),
    ("l", "ll", 0.8),
    ("ll", "l", 0.8),
    ("t", "tt", 0.9),
    ("tt", "t", 0.9),
    ("c", "cc", 0.9),
    ("m", "mm", 0.9),
    ("r", "rr", 0.9),
    ("p", "pp", 0.9),
    ("f", "ph", 1.1),
    ("ance", "ence", 0.7),
    ("ence", "ance", 0.7),
    ("ant", "ent", 0.8),
    ("ent", "ant", 0.8),
    ("able", "ible", 0.8),
    ("ible", "able", 0.8),
    ("", "'", 0.3),
    ("a", "e", 1.3),
    ("e", "a", 1.3),
    ("i", "e", 1.3),
];

/// The rules of a language (by layout code)
pub fn rules(code: &str) -> &'static [Rule] {
    match code.split('-').next().unwrap_or(code) {
        "ru" => RU,
        "en" => EN,
        _ => &[],
    }
}
