// SPDX-License-Identifier: GPL-3.0-or-later
//! System fonts: Roboto for Latin/Cyrillic/Greek and the Noto font of each script in use.

use egui::{FontData, FontDefinitions, FontFamily};
use std::path::Path;
use std::sync::Arc;

/// Android system fonts first; the Linux paths only matter for host snapshots
const DIRS: &[&str] = &["/system/fonts", "/usr/share/fonts/noto", "/usr/share/fonts/noto-cjk", "/usr/share/fonts/TTF"];

fn read(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok().filter(|d| d.len() > 1024)
}

/// Finds the best Noto file for a script among the system fonts
fn script_font(files: &[String], script: &str) -> Option<String> {
    let prefixes: Vec<String> = match script {
        "CJK" => vec!["NotoSansCJK".into(), "NotoSansCJKjp".into(), "NotoSansSC".into(), "NotoSansKR".into(), "DroidSansFallback".into()],
        "Arabic" => vec!["NotoNaskhArabicUI".into(), "NotoNaskhArabic".into(), "NotoSansArabic".into()],
        "Hebrew" => vec!["NotoSansHebrew".into()],
        "Ethiopic" => vec!["NotoSansEthiopic".into(), "NotoSerifEthiopic".into()],
        "Sinhala" => vec!["NotoSansSinhalaUI".into(), "NotoSansSinhala".into()],
        "Symbols" => vec!["NotoSansSymbols-Regular".into(), "NotoSansSymbols2".into()],
        s => vec![format!("NotoSans{s}UI"), format!("NotoSans{s}"), format!("NotoSerif{s}")],
    };
    for pre in prefixes {
        let mut cands: Vec<&String> = files
            .iter()
            .filter(|f| f.starts_with(&pre) && (f.ends_with(".ttf") || f.ends_with(".otf") || f.ends_with(".ttc")))
            .filter(|f| !f.contains("Bold") && !f.contains("Medium") && !f.contains("Light") && !f.contains("Thin"))
            .collect();
        // prefer "Regular" over variable fonts over anything else
        cands.sort_by_key(|f| (!f.contains("Regular"), !f.contains("VF"), f.len()));
        if let Some(f) = cands.first() {
            return Some((*f).clone());
        }
    }
    None
}

/// Like [`definitions`], but each set of scripts is read from disk once per process: the
/// keyboard gets a new window (and egui context) every time it is shown, and the CJK fonts
/// alone are tens of megabytes
pub fn cached(scripts: &[&str]) -> FontDefinitions {
    use std::sync::Mutex;
    static CACHE: Mutex<Vec<(Vec<String>, FontDefinitions)>> = Mutex::new(Vec::new());
    let key: Vec<String> = scripts.iter().map(|s| s.to_string()).collect();
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, d)) = c.iter().find(|(k, _)| *k == key) {
        return d.clone();
    }
    let d = definitions(scripts);
    // keep the last few sets (switching between languages of different scripts)
    if c.len() >= 3 {
        c.remove(0);
    }
    c.push((key, d.clone()));
    d
}

/// egui fonts for the given scripts (Latin, Cyrillic and Greek need nothing beyond Roboto)
pub fn definitions(scripts: &[&str]) -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let mut files: Vec<String> = Vec::new();
    let mut dir_of = std::collections::HashMap::new();
    for d in DIRS {
        if let Ok(rd) = std::fs::read_dir(d) {
            for e in rd.flatten() {
                if let Ok(n) = e.file_name().into_string() {
                    dir_of.entry(n.clone()).or_insert(*d);
                    files.push(n);
                }
            }
        }
    }
    let path_of = |f: &str| Path::new(dir_of.get(f).copied().unwrap_or(DIRS[0])).join(f);
    let mut front: Vec<String> = Vec::new();
    for name in ["Roboto-Regular.ttf", "RobotoStatic-Regular.ttf", "Roboto-Regular.otf", "DroidSans.ttf"] {
        if let Some(data) = read(&path_of(name)).filter(|_| dir_of.get(name) == Some(&DIRS[0])) {
            defs.font_data.insert("system".into(), Arc::new(FontData::from_owned(data)));
            front.push("system".into());
            break;
        }
    }
    let mut wanted: Vec<&str> = scripts.to_vec();
    wanted.push("Symbols");
    for s in wanted {
        if matches!(s, "Latin" | "Cyrillic" | "Greek") {
            continue;
        }
        if let Some(f) = script_font(&files, s) {
            if let Some(data) = read(&path_of(&f)) {
                let key = format!("script-{s}");
                defs.font_data.insert(key.clone(), Arc::new(FontData::from_owned(data)));
                front.push(key);
            }
        }
    }
    for fam in [FontFamily::Proportional, FontFamily::Monospace] {
        let list = defs.families.entry(fam).or_default();
        for (i, k) in front.iter().enumerate() {
            list.insert(i, k.clone());
        }
    }
    defs
}
