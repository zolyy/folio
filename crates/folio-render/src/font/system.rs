//! Substitutes for fonts a PDF names but does not embed (most often the
//! standard 14: Helvetica, Times, Courier, Symbol...). Folio reads a
//! metric-compatible font already installed on the computer — Arial, Times
//! New Roman and Courier New on Windows and macOS, Liberation or DejaVu on
//! Linux. Nothing is copied, shipped or embedded (AGENTS.md §1); the font
//! is only read at run time to draw the page.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// Font files larger than this are not read.
const MAX_FONT_FILE: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Sans,
    Serif,
    Mono,
    Symbol,
}

/// Regular, bold, italic, bold-italic file names, in order of preference.
fn candidates(f: Family) -> &'static [[&'static str; 4]] {
    match f {
        Family::Sans => &[
            ["arial.ttf", "arialbd.ttf", "ariali.ttf", "arialbi.ttf"],
            [
                "arial.ttf",
                "arial bold.ttf",
                "arial italic.ttf",
                "arial bold italic.ttf",
            ],
            [
                "liberationsans-regular.ttf",
                "liberationsans-bold.ttf",
                "liberationsans-italic.ttf",
                "liberationsans-bolditalic.ttf",
            ],
            [
                "dejavusans.ttf",
                "dejavusans-bold.ttf",
                "dejavusans-oblique.ttf",
                "dejavusans-boldoblique.ttf",
            ],
        ],
        Family::Serif => &[
            ["times.ttf", "timesbd.ttf", "timesi.ttf", "timesbi.ttf"],
            [
                "times new roman.ttf",
                "times new roman bold.ttf",
                "times new roman italic.ttf",
                "times new roman bold italic.ttf",
            ],
            [
                "liberationserif-regular.ttf",
                "liberationserif-bold.ttf",
                "liberationserif-italic.ttf",
                "liberationserif-bolditalic.ttf",
            ],
            [
                "dejavuserif.ttf",
                "dejavuserif-bold.ttf",
                "dejavuserif-italic.ttf",
                "dejavuserif-bolditalic.ttf",
            ],
        ],
        Family::Mono => &[
            ["cour.ttf", "courbd.ttf", "couri.ttf", "courbi.ttf"],
            [
                "courier new.ttf",
                "courier new bold.ttf",
                "courier new italic.ttf",
                "courier new bold italic.ttf",
            ],
            [
                "liberationmono-regular.ttf",
                "liberationmono-bold.ttf",
                "liberationmono-italic.ttf",
                "liberationmono-bolditalic.ttf",
            ],
            [
                "dejavusansmono.ttf",
                "dejavusansmono-bold.ttf",
                "dejavusansmono-oblique.ttf",
                "dejavusansmono-boldoblique.ttf",
            ],
        ],
        Family::Symbol => &[["symbol.ttf", "symbol.ttf", "symbol.ttf", "symbol.ttf"]],
    }
}

fn font_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    if let Some(w) = env("WINDIR") {
        v.push(w.join("Fonts"));
    }
    if let Some(l) = env("LOCALAPPDATA") {
        v.push(l.join("Microsoft").join("Windows").join("Fonts"));
    }
    for d in [
        "/System/Library/Fonts",
        "/System/Library/Fonts/Supplemental",
        "/Library/Fonts",
        "/usr/share/fonts",
        "/usr/local/share/fonts",
    ] {
        v.push(PathBuf::from(d));
    }
    if let Some(h) = env("HOME") {
        v.push(h.join("Library").join("Fonts"));
        v.push(h.join(".fonts"));
        v.push(h.join(".local").join("share").join("fonts"));
    }
    v
}

/// Lower-case file name → path, for every font file under the font folders.
fn index() -> &'static HashMap<String, PathBuf> {
    static INDEX: OnceLock<HashMap<String, PathBuf>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut map = HashMap::new();
        let mut stack: Vec<(PathBuf, usize)> = font_dirs().into_iter().map(|d| (d, 0)).collect();
        let mut seen = 0usize;
        while let Some((dir, depth)) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in rd.flatten() {
                seen += 1;
                if seen > 50_000 {
                    return map;
                }
                let p = e.path();
                if p.is_dir() {
                    if depth < 5 {
                        stack.push((p, depth + 1));
                    }
                } else if let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_lowercase())
                    && (name.ends_with(".ttf") || name.ends_with(".otf"))
                {
                    map.entry(name).or_insert(p);
                }
            }
        }
        map
    })
}

fn cache() -> &'static Mutex<HashMap<PathBuf, Arc<Vec<u8>>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<Vec<u8>>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn read(path: &PathBuf) -> Option<Arc<Vec<u8>>> {
    let mut guard = match cache().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(d) = guard.get(path) {
        return Some(d.clone());
    }
    if std::fs::metadata(path).ok()?.len() > MAX_FONT_FILE {
        return None;
    }
    let data = Arc::new(std::fs::read(path).ok()?);
    guard.insert(path.clone(), data.clone());
    Some(data)
}

/// Classify a PDF font by its name and descriptor flags (§9.8.2).
pub fn classify(base_font: &str, flags: i64) -> (Family, bool, bool) {
    // Drop a subset prefix such as "ABCDEF+".
    let name = match base_font.split_once('+') {
        Some((p, rest)) if p.len() == 6 => rest,
        _ => base_font,
    };
    let lower = name.to_lowercase();
    let family = if lower.contains("symbol") {
        Family::Symbol
    } else if lower.contains("courier") || lower.contains("mono") || flags & 1 != 0 {
        Family::Mono
    } else if lower.contains("times")
        || lower.contains("serif") && !lower.contains("sans")
        || lower.contains("roman")
        || lower.contains("georgia")
        || lower.contains("garamond")
        || (flags & 2 != 0 && !lower.contains("arial") && !lower.contains("helvetica"))
    {
        Family::Serif
    } else {
        Family::Sans
    };
    let bold = ["bold", "black", "heavy", "semibold", "demi"]
        .iter()
        .any(|w| lower.contains(w))
        || flags & (1 << 18) != 0;
    let italic = lower.contains("italic") || lower.contains("oblique") || flags & (1 << 6) != 0;
    (family, bold, italic)
}

/// The installed font that best stands in for `base_font`, if any.
pub fn substitute(base_font: &str, flags: i64) -> Option<(Arc<Vec<u8>>, Family)> {
    let (family, bold, italic) = classify(base_font, flags);
    let style = usize::from(bold) + 2 * usize::from(italic);
    let idx = index();
    for set in candidates(family) {
        for pick in [style, usize::from(bold), 0] {
            if let Some(path) = set.get(pick).and_then(|n| idx.get(*n))
                && let Some(data) = read(path)
            {
                return Some((data, family));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_14_classification() {
        assert_eq!(
            classify("Helvetica-BoldOblique", 0),
            (Family::Sans, true, true)
        );
        assert_eq!(classify("Times-Roman", 0), (Family::Serif, false, false));
        assert_eq!(classify("ABCDEF+Courier", 0), (Family::Mono, false, false));
        assert_eq!(classify("Symbol", 4), (Family::Symbol, false, false));
        assert_eq!(classify("Arial,Bold", 32), (Family::Sans, true, false));
    }
}
