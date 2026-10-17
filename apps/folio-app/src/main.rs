//! `folio-app [FILE.pdf ...]`: the reading window.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use folio_app::app::{App, Config};
use folio_app::cache::Renderer;

struct Window(App);

impl eframe::App for Window {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.ui(ui);
    }
}

fn main() -> eframe::Result {
    let mut app = App::new(Config {
        recent_file: folio_app::config_dir().map(|d| d.join("recent.txt")),
        threads: Renderer::<()>::default_threads(),
    });
    app.picker = Some(Box::new(|| {
        rfd::FileDialog::new()
            .add_filter("PDF files", &["pdf"])
            .add_filter("All files", &["*"])
            .pick_files()
            .unwrap_or_default()
    }));
    for arg in std::env::args_os().skip(1) {
        app.open_path(std::path::Path::new(&arg));
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Folio")
            .with_inner_size([1200.0, 860.0])
            .with_min_inner_size([480.0, 360.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native("Folio", options, Box::new(|_cc| Ok(Box::new(Window(app)))))
}
