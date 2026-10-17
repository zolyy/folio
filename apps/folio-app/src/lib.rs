//! Folio's desktop app. The window (`app`) is a thin layer over a view model
//! (`viewer`) and a background renderer (`cache`), so agents and tests can
//! drive every command without a screen (egui_kittest, AccessKit).

pub mod app;
pub mod cache;
pub mod icons;
pub mod viewer;

use std::path::PathBuf;

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
