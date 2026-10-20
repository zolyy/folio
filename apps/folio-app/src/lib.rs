//! Folio's desktop app. The window (`app`) is a thin layer over a view model
//! (`viewer`) and a background renderer (`cache`), so agents and tests can
//! drive every command without a screen (egui_kittest, AccessKit).

pub mod app;
pub mod cache;
pub mod find;
pub mod icons;
pub mod panels;
pub mod viewer;

use std::path::PathBuf;

/// Text from a document as the window can show it: control characters
/// become spaces and characters the window's fonts cannot draw become '?'.
/// Titles, labels and names in PDFs can hold anything.
pub fn drawable(ctx: &eframe::egui::Context, s: &str) -> String {
    use eframe::egui::FontId;
    let (p, m) = (FontId::proportional(14.0), FontId::monospace(14.0));
    ctx.fonts_mut(|f| {
        s.chars()
            .take(4096)
            .map(|c| {
                if c.is_control() {
                    ' '
                } else if f.has_glyph(&p, c) && f.has_glyph(&m, c) {
                    c
                } else {
                    '?'
                }
            })
            .collect()
    })
}

/// Where per-user settings live: `%APPDATA%\Folio` on Windows,
/// `~/Library/Application Support/Folio` on macOS, `$XDG_CONFIG_HOME/folio`
/// (or `~/.config/folio`) elsewhere.
pub fn config_dir() -> Option<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if cfg!(windows) {
        env("APPDATA").map(|d| d.join("Folio"))
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support/Folio"))
    } else {
        env("XDG_CONFIG_HOME")
            .or_else(|| env("HOME").map(|h| h.join(".config")))
            .map(|d| d.join("folio"))
    }
}

/// Command-line arguments: files to open, then Acrobat's open parameters
/// `--page N` (go to page N of the last file) and `--search TEXT` (open
/// Find with TEXT and go to the first match).
pub fn apply_args(app: &mut app::App, args: impl IntoIterator<Item = std::ffi::OsString>) {
    let mut args = args.into_iter();
    let mut page: Option<usize> = None;
    let mut search: Option<String> = None;
    while let Some(a) = args.next() {
        match a.to_str() {
            Some("--page") => {
                page = args
                    .next()
                    .and_then(|v| v.to_str().and_then(|s| s.trim().parse().ok()));
            }
            Some("--search") => search = args.next().map(|v| v.to_string_lossy().into_owned()),
            _ => app.open_path(std::path::Path::new(&a)),
        }
    }
    if let Some(n) = page
        && let Some(t) = app.tabs.get_mut(app.active)
    {
        t.go_to(n.saturating_sub(1));
    }
    if let Some(q) = search.filter(|q| !q.trim().is_empty()) {
        app.find.query = q;
        app.find.pending = true;
        app.find.open = true;
    }
}
