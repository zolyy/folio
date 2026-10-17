//! The reading window: menus, toolbar, document tabs, the navigation pane
//! (page thumbnails, bookmarks), the page view and the status bar.
//! Everything is drawn from `App` state; tests drive it with egui_kittest.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Key, KeyboardShortcut, Modifiers, Panel, Pos2,
    Rect, Role, ScrollArea, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Vec2,
    WidgetInfo,
};
use folio_core::{Document, Error};

use crate::cache::{Done, Key as PageKey, MAX_SIDE, Renderer};
use crate::icons::{self, Icon};
use crate::viewer::{Layout, PAGE_GAP, Tab, Zoom};

/// Files kept in File → Open Recent (Folio choice).
pub const RECENT_MAX: usize = 10;
/// Largest file the window opens (the engine's own cap).
const MAX_FILE: u64 = folio_core::document::MAX_FILE;
/// Thumbnail box in the navigation pane, in screen points.
const THUMB_W: f32 = 104.0;
const THUMB_H: f32 = 134.0;
const THUMB_ROW: f32 = THUMB_H + 26.0;
/// One arrow-key step in the page view, in screen points (Folio choice).
const ARROW_STEP: f32 = 48.0;

/// A rendered page as the window keeps it.
#[derive(Clone)]
pub enum Shown {
    Texture(TextureHandle),
    Failed(Arc<str>),
}

/// The navigation pane at the left (Acrobat: View → Show/Hide → Side panels).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Hidden,
    Thumbnails,
    Bookmarks,
}

/// A protected document waiting for its password.
#[derive(Debug)]
pub struct PasswordPrompt {
    pub name: String,
    pub path: Option<PathBuf>,
    bytes: Vec<u8>,
    pub input: String,
    pub wrong: bool,
    /// Put the cursor in the password box on the next frame.
    focus: bool,
}

pub struct Config {
    /// Where File → Open Recent is remembered; `None` keeps it in memory.
    pub recent_file: Option<PathBuf>,
    pub threads: usize,
}

type Picker = Box<dyn FnMut() -> Vec<PathBuf>>;

pub struct App {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub pane: Pane,
    pub recent: Vec<PathBuf>,
    pub password: Option<PasswordPrompt>,
    /// The last error, shown in the status bar until dismissed.
    pub message: Option<String>,
    /// The page view's size at the last frame, in screen points.
    pub viewport: [f32; 2],
    /// Shows the system Open dialog; `None` in tests.
    pub picker: Option<Picker>,
    pub renderer: Renderer<Shown>,
    recent_file: Option<PathBuf>,
    ctx: Arc<OnceLock<egui::Context>>,
    next_id: u64,
    page_box: String,
    zoom_box: String,
    focus_page_box: bool,
    scroll_delta: f32,
    title: String,
    wanted: HashSet<PageKey>,
}

impl App {
    pub fn new(config: Config) -> Self {
        let ctx: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());
        let notify_ctx = ctx.clone();
        let renderer = Renderer::new(
            config.threads,
            Arc::new(move || {
                if let Some(c) = notify_ctx.get() {
                    c.request_repaint();
                }
            }),
        );
        let recent = config
            .recent_file
            .as_deref()
            .map(load_recent)
            .unwrap_or_default();
        App {
            tabs: Vec::new(),
            active: 0,
            pane: Pane::Thumbnails,
            recent,
            password: None,
            message: None,
            viewport: [800.0, 600.0],
            picker: None,
            renderer,
            recent_file: config.recent_file,
            ctx,
            next_id: 1,
            page_box: String::new(),
            zoom_box: String::new(),
            focus_page_box: false,
            scroll_delta: 0.0,
            title: String::new(),
            wanted: HashSet::new(),
        }
    }

    pub fn tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    pub fn tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active)
    }

    // ---- opening and closing -------------------------------------------

    /// File → Open for one file. A file already open is brought to front.
    pub fn open_path(&mut self, path: &Path) {
        if let Some(i) = self
            .tabs
            .iter()
            .position(|t| t.path.as_deref() == Some(path))
        {
            self.active = i;
            return;
        }
        let name = file_name(path);
        let bytes = match std::fs::metadata(path) {
            Ok(m) if m.len() > MAX_FILE => {
                self.message = Some(format!("{name} is larger than Folio can open (2 GB)."));
                return;
            }
            Ok(_) => std::fs::read(path),
            Err(e) => Err(e),
        };
        match bytes {
            Ok(b) => self.open_bytes(b, Some(path.to_path_buf()), name),
            Err(e) => self.message = Some(format!("Could not open {name}: {e}")),
        }
    }

    /// Open a document from memory (dropped files, tests).
    pub fn open_bytes(&mut self, bytes: Vec<u8>, path: Option<PathBuf>, name: String) {
        match Document::from_bytes(&bytes) {
            Ok(doc) => self.add_tab(doc, path, name),
            Err(Error::Password(_)) => {
                self.password = Some(PasswordPrompt {
                    name,
                    path,
                    bytes,
                    input: String::new(),
                    wrong: false,
                    focus: true,
                });
            }
            Err(e) => self.message = Some(format!("Could not open {name}: {e}")),
        }
    }

    /// Try the password typed into the prompt.
    pub fn submit_password(&mut self) {
        let Some(mut p) = self.password.take() else {
            return;
        };
        match Document::from_bytes_with_password(&p.bytes, &p.input) {
            Ok(doc) => self.add_tab(doc, p.path, p.name),
            Err(Error::Password(_)) => {
                p.wrong = true;
                p.focus = true;
                p.input.clear();
                self.password = Some(p);
            }
            Err(e) => self.message = Some(format!("Could not open {}: {e}", p.name)),
        }
    }

    fn add_tab(&mut self, doc: Document, path: Option<PathBuf>, name: String) {
        if let Some(p) = &path {
            self.remember(p.clone());
        }
        let id = self.next_id;
        self.next_id += 1;
        let tab = Tab::new(doc, path, name, id);
        if tab.page_count() == 0 {
            self.message = Some(format!("{} has no pages to show.", tab.title));
        }
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
    }

    /// File → Close (Ctrl+W).
    pub fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        let t = self.tabs.remove(i);
        self.renderer.forget(t.id);
        if self.active > i || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
    }

    fn remember(&mut self, path: PathBuf) {
        self.recent.retain(|p| p != &path);
        self.recent.insert(0, path);
        self.recent.truncate(RECENT_MAX);
        if let Some(f) = &self.recent_file {
            save_recent(f, &self.recent);
        }
    }

    fn pick_and_open(&mut self) {
        let files = match self.picker.as_mut() {
            Some(p) => p(),
            None => return,
        };
        for f in files {
            self.open_path(&f);
        }
    }

    // ---- the frame ------------------------------------------------------

    /// Draw one frame of the whole window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let _ = self.ctx.set(ctx.clone());
        self.take_dropped_files(&ctx);
        self.collect(&ctx);
        self.shortcuts(&ctx);
        self.wanted.clear();

        Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        if !self.tabs.is_empty() {
            Panel::top("tabs").show(ui, |ui| self.tab_bar(ui));
        }
        Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        if self.pane != Pane::Hidden && !self.tabs.is_empty() {
            Panel::left("pane")
                .resizable(true)
                .default_size(THUMB_W + 48.0)
                .min_size(THUMB_W + 24.0)
                .show(ui, |ui| match self.pane {
                    Pane::Thumbnails => self.thumbnails(ui),
                    _ => self.bookmarks(ui),
                });
        }
        let page_bg = if ui.visuals().dark_mode {
            Color32::from_gray(48)
        } else {
            Color32::from_gray(200)
        };
        egui::CentralPanel::no_frame()
            .frame(egui::Frame::new().fill(page_bg))
            .show(ui, |ui| self.page_view(ui));

        self.password_dialog(&ctx);
        self.renderer.retain(&self.wanted);
        self.update_title(&ctx);
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match self.tab() {
            Some(t) => format!("{} - Folio", t.title),
            None => "Folio".to_owned(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            let path = f.path().to_path_buf();
            if path.as_os_str().is_empty() {
                match f.bytes() {
                    Ok(b) => self.open_bytes(b, None, "Untitled.pdf".into()),
                    Err(e) => self.message = Some(format!("Could not read the dropped file: {e}")),
                }
            } else {
                self.open_path(&path);
            }
        }
    }

    /// Turn finished renders into textures.
    fn collect(&mut self, ctx: &egui::Context) {
        let max_side = ctx.input(|i| i.max_texture_side);
        self.renderer.set_max_side(max_side);
        self.renderer.collect(|k, d| match d {
            Done::Image { width, height, .. }
                if width as usize > max_side || height as usize > max_side =>
            {
                Shown::Failed(
                    format!("{width} x {height} is larger than this display can show").into(),
                )
            }
            Done::Image {
                width,
                height,
                rgba,
            } => {
                let img = egui::ColorImage::from_rgba_premultiplied(
                    [width as usize, height as usize],
                    &rgba,
                );
                Shown::Texture(ctx.load_texture(
                    format!("page-{}-{}-{}-{}", k.doc, k.page, k.width, k.rotation),
                    img,
                    TextureOptions::LINEAR,
                ))
            }
            Done::Failed(m) => Shown::Failed(m.into()),
        });
    }

    // ---- keyboard (Acrobat's shortcuts) ---------------------------------

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.password.is_some() {
            return;
        }
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let take = |m: Modifiers, k: Key| {
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)))
        };
        let vp = self.viewport;
        if take(cmd, Key::O) {
            self.pick_and_open();
        }
        if take(cmd_shift, Key::Tab) && !self.tabs.is_empty() {
            self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
        }
        if take(cmd, Key::Tab) && !self.tabs.is_empty() {
            self.active = (self.active + 1) % self.tabs.len();
        }
        if take(cmd, Key::W) {
            self.close_tab(self.active);
        }
        if take(Modifiers::NONE, Key::F4) {
            self.pane = match self.pane {
                Pane::Hidden => Pane::Thumbnails,
                _ => Pane::Hidden,
            };
        }
        if take(cmd_shift, Key::N) {
            self.focus_page_box = true;
        }
        let rot_cw = take(cmd_shift, Key::Plus) || take(cmd_shift, Key::Equals);
        let rot_ccw = take(cmd_shift, Key::Minus);
        let zin = take(cmd, Key::Plus) || take(cmd, Key::Equals);
        let zout = take(cmd, Key::Minus);
        let fit_page = take(cmd, Key::Num0);
        let actual = take(cmd, Key::Num1);
        let fit_width = take(cmd, Key::Num2);
        let typing = ctx.egui_wants_keyboard_input();
        let plain = |k: Key| !typing && take(Modifiers::NONE, k);
        let next = plain(Key::ArrowRight) || plain(Key::PageDown);
        let prev = plain(Key::ArrowLeft) || plain(Key::PageUp);
        let first = plain(Key::Home);
        let last = plain(Key::End);
        let down = plain(Key::ArrowDown);
        let up = plain(Key::ArrowUp);
        let Some(t) = self.tabs.get_mut(self.active) else {
            return;
        };
        if rot_cw {
            t.rotate_clockwise();
        }
        if rot_ccw {
            t.rotate_counterclockwise();
        }
        if zin {
            t.zoom_in(vp);
        }
        if zout {
            t.zoom_out(vp);
        }
        if fit_page {
            t.set_zoom(Zoom::FitPage);
        }
        if actual {
            t.set_zoom(Zoom::Percent(100.0));
        }
        if fit_width {
            t.set_zoom(Zoom::FitWidth);
        }
        if next {
            t.next_page();
        }
        if prev {
            t.previous_page();
        }
        if first {
            t.first_page();
        }
        if last {
            t.last_page();
        }
        if down {
            self.scroll_delta -= ARROW_STEP;
        }
        if up {
            self.scroll_delta += ARROW_STEP;
        }
    }

    // ---- menus and toolbar ----------------------------------------------

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let vp = self.viewport;
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui
                    .add(egui::Button::new("Open…").shortcut_text("Ctrl+O"))
                    .clicked()
                {
                    ui.close();
                    self.pick_and_open();
                }
                ui.menu_button("Open Recent", |ui| {
                    if self.recent.is_empty() {
                        ui.label("No recent files");
                    }
                    let mut chosen = None;
                    for p in &self.recent {
                        if ui
                            .button(file_name(p))
                            .on_hover_text(p.display().to_string())
                            .clicked()
                        {
                            chosen = Some(p.clone());
                        }
                    }
                    if let Some(p) = chosen {
                        ui.close();
                        self.open_path(&p);
                    }
                });
                if ui
                    .add_enabled(
                        !self.tabs.is_empty(),
                        egui::Button::new("Close").shortcut_text("Ctrl+W"),
                    )
                    .clicked()
                {
                    ui.close();
                    self.close_tab(self.active);
                }
                ui.separator();
                if ui.button("Exit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            let has = !self.tabs.is_empty();
            ui.add_enabled_ui(has, |ui| {
                ui.menu_button("View", |ui| {
                    let Some(t) = self.tabs.get_mut(self.active) else {
                        return;
                    };
                    ui.menu_button("Page Navigation", |ui| {
                        if ui
                            .add(egui::Button::new("First Page").shortcut_text("Home"))
                            .clicked()
                        {
                            t.first_page();
                        }
                        if ui
                            .add(egui::Button::new("Previous Page").shortcut_text("Left Arrow"))
                            .clicked()
                        {
                            t.previous_page();
                        }
                        if ui
                            .add(egui::Button::new("Next Page").shortcut_text("Right Arrow"))
                            .clicked()
                        {
                            t.next_page();
                        }
                        if ui
                            .add(egui::Button::new("Last Page").shortcut_text("End"))
                            .clicked()
                        {
                            t.last_page();
                        }
                        if ui
                            .add(egui::Button::new("Go to Page…").shortcut_text("Ctrl+Shift+N"))
                            .clicked()
                        {
                            self.focus_page_box = true;
                            ui.close();
                        }
                    });
                    ui.menu_button("Zoom", |ui| {
                        if ui
                            .add(egui::Button::new("Zoom In").shortcut_text("Ctrl+="))
                            .clicked()
                        {
                            t.zoom_in(vp);
                        }
                        if ui
                            .add(egui::Button::new("Zoom Out").shortcut_text("Ctrl+-"))
                            .clicked()
                        {
                            t.zoom_out(vp);
                        }
                        ui.separator();
                        if ui
                            .add(egui::Button::new("Zoom to Page Level").shortcut_text("Ctrl+0"))
                            .clicked()
                        {
                            t.set_zoom(Zoom::FitPage);
                        }
                        if ui
                            .add(egui::Button::new("Actual Size").shortcut_text("Ctrl+1"))
                            .clicked()
                        {
                            t.set_zoom(Zoom::Percent(100.0));
                        }
                        if ui
                            .add(egui::Button::new("Fit Width").shortcut_text("Ctrl+2"))
                            .clicked()
                        {
                            t.set_zoom(Zoom::FitWidth);
                        }
                    });
                    ui.menu_button("Rotate View", |ui| {
                        if ui
                            .add(egui::Button::new("Clockwise").shortcut_text("Ctrl+Shift+Plus"))
                            .clicked()
                        {
                            t.rotate_clockwise();
                        }
                        if ui
                            .add(
                                egui::Button::new("Counterclockwise")
                                    .shortcut_text("Ctrl+Shift+Minus"),
                            )
                            .clicked()
                        {
                            t.rotate_counterclockwise();
                        }
                    });
                    ui.menu_button("Page Display", |ui| {
                        let mut layout = t.layout;
                        ui.radio_value(&mut layout, Layout::SinglePage, "Single Page View");
                        ui.radio_value(&mut layout, Layout::Continuous, "Enable Scrolling");
                        if layout != t.layout {
                            t.set_layout(layout);
                        }
                    });
                    ui.menu_button("Show/Hide", |ui| {
                        ui.radio_value(&mut self.pane, Pane::Thumbnails, "Page Thumbnails");
                        ui.radio_value(&mut self.pane, Pane::Bookmarks, "Bookmarks");
                        ui.radio_value(&mut self.pane, Pane::Hidden, "Hide Navigation Pane (F4)");
                    });
                });
            });
            ui.menu_button("Window", |ui| {
                if self.tabs.is_empty() {
                    ui.label("No open documents");
                }
                for (i, t) in self.tabs.iter().enumerate() {
                    if ui.radio(i == self.active, &t.title).clicked() {
                        self.active = i;
                        ui.close();
                    }
                }
            });
            ui.menu_button("Help", |ui| {
                ui.label(format!("Folio {}", env!("CARGO_PKG_VERSION")));
                ui.label("An open-source PDF workbench. MIT OR Apache-2.0.");
            });
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let vp = self.viewport;
        ui.horizontal(|ui| {
            if icons::button(ui, Icon::Open, "Open", true).clicked() {
                self.pick_and_open();
            }
            ui.separator();
            let focus_page_box = std::mem::take(&mut self.focus_page_box);
            let Some(t) = self.tabs.get_mut(self.active) else {
                return;
            };
            let n = t.page_count();
            let not_first = t.current > 0;
            let not_last = t.current + 1 < n;
            if icons::button(ui, Icon::First, "First Page", not_first).clicked() {
                t.first_page();
            }
            if icons::button(ui, Icon::Previous, "Previous Page", not_first).clicked() {
                t.previous_page();
            }
            let page_edit = ui.add(
                egui::TextEdit::singleline(&mut self.page_box)
                    .desired_width(48.0)
                    .align(Align2::CENTER_CENTER),
            );
            page_edit.widget_info(|| WidgetInfo::labeled(Role::TextInput, true, "Page Number"));
            if focus_page_box {
                page_edit.request_focus();
            }
            if page_edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                t.go_to_text(&self.page_box);
            }
            if !page_edit.has_focus() {
                self.page_box = t.page_name(t.current);
            }
            if t.labels.is_some() {
                ui.label(format!("({} of {n})", t.current + 1));
            } else {
                ui.label(format!("/ {n}"));
            }
            if icons::button(ui, Icon::Next, "Next Page", not_last).clicked() {
                t.next_page();
            }
            if icons::button(ui, Icon::Last, "Last Page", not_last).clicked() {
                t.last_page();
            }
            ui.separator();
            if icons::button(ui, Icon::ZoomOut, "Zoom Out", true).clicked() {
                t.zoom_out(vp);
            }
            let zoom_edit = ui.add(
                egui::TextEdit::singleline(&mut self.zoom_box)
                    .desired_width(56.0)
                    .align(Align2::CENTER_CENTER),
            );
            zoom_edit.widget_info(|| WidgetInfo::labeled(Role::TextInput, true, "Zoom"));
            if zoom_edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                t.set_zoom_text(&self.zoom_box);
            }
            if !zoom_edit.has_focus() {
                self.zoom_box = format_percent(t.percent(vp));
            }
            if icons::button(ui, Icon::ZoomIn, "Zoom In", true).clicked() {
                t.zoom_in(vp);
            }
            let presets = ui.menu_button("   ", |ui| {
                for z in crate::viewer::ZOOM_STEPS {
                    if ui.button(format_percent(z)).clicked() {
                        t.set_zoom(Zoom::Percent(z));
                    }
                }
            });
            presets
                .response
                .widget_info(|| WidgetInfo::labeled(Role::Button, true, "Zoom Presets"));
            let color = ui.visuals().text_color();
            icons::paint(ui.painter(), presets.response.rect, Icon::Presets, color);
            if icons::button(ui, Icon::FitPage, "Fit Page", true).clicked() {
                t.set_zoom(Zoom::FitPage);
            }
            if icons::button(ui, Icon::FitWidth, "Fit Width", true).clicked() {
                t.set_zoom(Zoom::FitWidth);
            }
            ui.separator();
            if icons::button(
                ui,
                Icon::RotateCounterclockwise,
                "Rotate Counterclockwise",
                true,
            )
            .clicked()
            {
                t.rotate_counterclockwise();
            }
            if icons::button(ui, Icon::RotateClockwise, "Rotate Clockwise", true).clicked() {
                t.rotate_clockwise();
            }
            ui.separator();
            let single = t.layout == Layout::SinglePage;
            if ui
                .selectable_label(single, "Single Page")
                .on_hover_text("Single Page View")
                .clicked()
            {
                t.set_layout(Layout::SinglePage);
            }
            if ui
                .selectable_label(!single, "Scrolling")
                .on_hover_text("Enable Scrolling")
                .clicked()
            {
                t.set_layout(Layout::Continuous);
            }
        });
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let mut close = None;
        ui.horizontal_wrapped(|ui| {
            for (i, t) in self.tabs.iter().enumerate() {
                let r = ui.selectable_label(i == self.active, &t.title);
                let r = match &t.path {
                    Some(p) => r.on_hover_text(p.display().to_string()),
                    None => r,
                };
                if r.clicked() {
                    self.active = i;
                }
                let x = ui.small_button("×");
                x.widget_info(|| {
                    WidgetInfo::labeled(Role::Button, true, format!("Close {}", t.title))
                });
                if x.clicked() {
                    close = Some(i);
                }
                ui.add_space(6.0);
            }
        });
        if let Some(i) = close {
            self.close_tab(i);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(t) = self.tabs.get(self.active) {
                if let Some([w, h]) = t.sizes.get(t.current) {
                    ui.label(format!(
                        "Page {} of {} · {:.2} × {:.2} in",
                        t.current + 1,
                        t.page_count(),
                        w / 72.0,
                        h / 72.0
                    ));
                }
                if !t.repairs.is_empty() {
                    ui.separator();
                    ui.label(format!("Repaired: {} problem(s)", t.repairs.len()))
                        .on_hover_text(t.repairs.join("\n"));
                }
            }
            if let Some(m) = self.message.clone() {
                ui.separator();
                ui.colored_label(ui.visuals().error_fg_color, m);
                if ui.small_button("Dismiss").clicked() {
                    self.message = None;
                }
            }
        });
    }

    // ---- navigation pane -------------------------------------------------

    fn thumbnails(&mut self, ui: &mut egui::Ui) {
        ui.strong("Page Thumbnails");
        let ppp = ui.ctx().pixels_per_point();
        let Self {
            tabs,
            active,
            renderer,
            wanted,
            ..
        } = self;
        let Some(t) = tabs.get_mut(*active) else {
            return;
        };
        let n = t.page_count();
        let mut area = ScrollArea::vertical()
            .id_salt(("thumbs", t.id))
            .auto_shrink([false, false]);
        if t.scroll_to.is_some() {
            area = area.vertical_scroll_offset((t.current as f32 - 1.0).max(0.0) * THUMB_ROW);
        }
        let mut go = None;
        area.show_rows(ui, THUMB_ROW, n, |ui, rows| {
            for i in rows {
                let (row, resp) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), THUMB_ROW),
                    Sense::click(),
                );
                let name = t.page_name(i);
                resp.widget_info(|| {
                    WidgetInfo::labeled(Role::Button, true, format!("Page {name}"))
                });
                let [w, h] = t.display_size(i, 1.0);
                let s = (THUMB_W / w).min(THUMB_H / h);
                let size = Vec2::new(w * s, h * s);
                let rect = Rect::from_center_size(
                    Pos2::new(row.center().x, row.top() + 4.0 + THUMB_H / 2.0),
                    size,
                );
                let key = page_key(t, i, size.x * ppp, renderer.max_side());
                paint_page(ui, renderer, wanted, t, i, key, rect);
                if i == t.current {
                    ui.painter().rect_stroke(
                        rect.expand(3.0),
                        CornerRadius::same(2),
                        Stroke::new(2.0, ui.visuals().selection.bg_fill),
                        StrokeKind::Outside,
                    );
                }
                ui.painter().text(
                    Pos2::new(row.center().x, row.bottom() - 12.0),
                    Align2::CENTER_CENTER,
                    &name,
                    FontId::proportional(12.0),
                    ui.visuals().text_color(),
                );
                if resp.clicked() {
                    go = Some(i);
                }
            }
        });
        if let Some(i) = go {
            t.go_to(i);
        }
    }

    fn bookmarks(&mut self, ui: &mut egui::Ui) {
        ui.strong("Bookmarks");
        let Some(t) = self.tabs.get_mut(self.active) else {
            return;
        };
        if t.outline.is_empty() {
            ui.weak("This document has no bookmarks.");
            return;
        }
        let mut go = None;
        ScrollArea::both()
            .id_salt(("bookmarks", t.id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                bookmark_level(ui, &t.outline, egui::Id::unique(("bm", t.id)), &mut go);
            });
        if let Some(page) = go
            && let Some(i) = t.index_of(page)
        {
            t.go_to(i);
        }
    }

    // ---- the page view -----------------------------------------------------

    fn page_view(&mut self, ui: &mut egui::Ui) {
        let size = ui.available_size();
        self.viewport = [size.x, size.y];
        let ppp = ui.ctx().pixels_per_point();
        let scroll_delta = std::mem::take(&mut self.scroll_delta);
        let Self {
            tabs,
            active,
            renderer,
            wanted,
            recent,
            ..
        } = self;
        let Some(t) = tabs.get_mut(*active) else {
            let mut open = None;
            ui.vertical_centered(|ui| {
                ui.add_space(size.y * 0.3);
                ui.heading("Open a PDF with File > Open (Ctrl+O), or drop one here.");
                if !recent.is_empty() {
                    ui.add_space(12.0);
                    ui.label("Recent files");
                    for p in recent.iter() {
                        if ui.link(file_name(p)).clicked() {
                            open = Some(p.clone());
                        }
                    }
                }
            });
            if let Some(p) = open {
                self.open_path(&p);
            }
            return;
        };
        if t.page_count() == 0 {
            return;
        }
        let scale = t.scale([size.x, size.y]);
        let (tops, total) = match t.layout {
            Layout::Continuous => t.page_tops(scale),
            Layout::SinglePage => {
                let h = t.display_size(t.current, scale)[1];
                (vec![PAGE_GAP], h + 2.0 * PAGE_GAP)
            }
        };
        let pages: Vec<usize> = match t.layout {
            Layout::Continuous => (0..t.page_count()).collect(),
            Layout::SinglePage => vec![t.current],
        };
        let widest = pages
            .iter()
            .map(|&i| t.display_size(i, scale)[0])
            .fold(0.0f32, f32::max);
        let content_w = (widest + 2.0 * PAGE_GAP).max(size.x);
        let mut area = ScrollArea::both()
            .id_salt(("pages", t.id, t.layout == Layout::SinglePage))
            .auto_shrink([false, false]);
        let jump = t.scroll_to.take();
        if let Some(p) = jump {
            let y = match t.layout {
                Layout::Continuous => tops.get(p).copied().unwrap_or(0.0) - PAGE_GAP,
                Layout::SinglePage => 0.0,
            };
            area = area.vertical_scroll_offset(y.max(0.0));
        }
        area.show_viewport(ui, |ui, visible| {
            if scroll_delta != 0.0 {
                ui.scroll_with_delta(Vec2::new(0.0, scroll_delta));
            }
            let origin = ui.max_rect().min;
            ui.set_min_size(Vec2::new(content_w, total));
            let first = t
                .page_at(&tops, visible.top())
                .min(pages.len().saturating_sub(1));
            let mut shown = Vec::new();
            for (slot, &i) in pages.iter().enumerate().skip(first) {
                let top = tops.get(slot).copied().unwrap_or(0.0);
                if top > visible.bottom() {
                    break;
                }
                shown.push((slot, i, top));
            }
            for &(_, i, top) in &shown {
                let [w, h] = t.display_size(i, scale);
                let rect = Rect::from_min_size(
                    origin + Vec2::new((content_w - w) / 2.0, top),
                    Vec2::new(w, h),
                );
                let key = page_key(t, i, w * ppp, renderer.max_side());
                paint_page(ui, renderer, wanted, t, i, key, rect);
            }
            // Draw the next pages ahead of the scroll, once the visible ones are queued.
            if t.layout == Layout::Continuous
                && let Some(&(slot, _, _)) = shown.last()
            {
                for &i in pages.iter().skip(slot + 1).take(2) {
                    let w = t.display_size(i, scale)[0];
                    let key = page_key(t, i, w * ppp, renderer.max_side());
                    wanted.insert(key);
                    if let Some(&p) = t.pages.get(i) {
                        renderer.request(key, &t.doc, p);
                    }
                }
            }
            // The current page is the one in the middle of the window.
            if jump.is_none() && t.layout == Layout::Continuous {
                t.current = t.page_at(&tops, visible.center().y).min(t.page_count() - 1);
            }
        });
    }

    // ---- dialogs ------------------------------------------------------------

    fn password_dialog(&mut self, ctx: &egui::Context) {
        let Some(p) = self.password.as_mut() else {
            return;
        };
        let mut submit = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("password")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.heading("Password");
            ui.label(format!(
                "'{}' is protected. Enter the Document Open Password.",
                p.name
            ));
            let r = ui.add(
                egui::TextEdit::singleline(&mut p.input)
                    .password(true)
                    .desired_width(f32::INFINITY),
            );
            r.widget_info(|| WidgetInfo::labeled(Role::PasswordInput, true, "Enter Password"));
            if std::mem::take(&mut p.focus) {
                r.request_focus();
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                submit = true;
            }
            if p.wrong {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    "Incorrect password. Try again.",
                );
            }
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    submit = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if cancel {
            self.password = None;
        } else if submit {
            self.submit_password();
        }
    }
}

/// The render request for a page shown `width_px` pixels wide: views
/// wider than the display's texture limit are drawn at the limit and stretched.
fn page_key(t: &Tab, page: usize, width_px: f32, max_side: u32) -> PageKey {
    let [w, h] = t.display_size(page, 1.0);
    let fit = max_side as f32 * (w / h).min(1.0);
    let w = if width_px.is_finite() {
        width_px.round()
    } else {
        1.0
    };
    PageKey {
        doc: t.id,
        page,
        width: w.min(fit).clamp(1.0, MAX_SIDE as f32) as u32,
        rotation: t.rotation,
    }
}

/// Draw a page into `rect`: the rendered image when it is ready (or a
/// lower-resolution one meanwhile), white paper until then.
fn paint_page(
    ui: &egui::Ui,
    renderer: &mut Renderer<Shown>,
    wanted: &mut HashSet<PageKey>,
    t: &Tab,
    page: usize,
    key: PageKey,
    rect: Rect,
) {
    wanted.insert(key);
    if let Some(&p) = t.pages.get(page) {
        renderer.request(key, &t.doc, p);
    }
    let painter = ui.painter();
    painter.rect_filled(
        rect.translate(Vec2::splat(2.0)),
        0.0,
        Color32::from_black_alpha(60),
    );
    painter.rect_filled(rect, 0.0, Color32::WHITE);
    match renderer.get(key) {
        Some((_, Shown::Texture(tex))) => {
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            painter.image(tex.id(), rect, uv, Color32::WHITE);
        }
        Some((_, Shown::Failed(m))) => {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                format!("This page could not be drawn.\n{m}"),
                FontId::proportional(12.0),
                Color32::DARK_RED,
            );
        }
        None => {}
    }
}

fn bookmark_level(
    ui: &mut egui::Ui,
    nodes: &[folio_ops::outline::OutlineNode],
    id: egui::Id,
    go: &mut Option<folio_core::ObjRef>,
) {
    for (i, n) in nodes.iter().enumerate() {
        let id = id.with(i);
        let title = if n.title.trim().is_empty() {
            "(untitled)"
        } else {
            n.title.as_str()
        };
        let mut link = |ui: &mut egui::Ui| {
            let r = ui.selectable_label(false, title);
            if r.clicked()
                && let Some(p) = n.page
            {
                *go = Some(p);
            }
        };
        if n.children.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(ui.spacing().indent);
                link(ui);
            });
        } else {
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, n.open)
                .show_header(ui, link)
                .body(|ui| bookmark_level(ui, &n.children, id, go));
        }
    }
}

/// "125%", "8.33%": up to two decimals, no trailing zeros.
pub fn format_percent(p: f32) -> String {
    let s = format!("{p:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    format!("{s}%")
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

fn load_recent(f: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(f)
        .map(|s| {
            s.lines()
                .filter(|l| !l.trim().is_empty())
                .take(RECENT_MAX)
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

fn save_recent(f: &Path, list: &[PathBuf]) {
    if let Some(dir) = f.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text: String = list.iter().map(|p| format!("{}\n", p.display())).collect();
    // Losing the recent list is not worth interrupting the reader for.
    let _ = std::fs::write(f, text);
}
