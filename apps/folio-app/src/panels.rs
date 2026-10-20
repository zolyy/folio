//! Stage 4c: the Layers and Attachments panes, Document Properties (Ctrl+D),
//! Accessibility → Replace Document Colors, Full Screen Mode (Ctrl+L) and
//! Read Mode (Ctrl+H). The mapping to Acrobat is in docs/acrobat/view.md.

use eframe::egui::{self, ScrollArea};
use folio_ops::{attachments, fonts, info, security};
use folio_render::ReplaceColors;

use crate::app::App;
use crate::viewer::{Layout, Zoom};

/// Acrobat's Accessibility → Document Colors Options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorSettings {
    /// "Replace Document Colors".
    pub enabled: bool,
    /// "Use High-Contrast colors" (else "Custom Color").
    pub high_contrast: bool,
    /// Index into [`ReplaceColors::HIGH_CONTRAST`].
    pub pair: usize,
    pub custom_background: [u8; 3],
    pub custom_text: [u8; 3],
    /// "Only change the color of black text or line art".
    pub only_black: bool,
    /// "Change the color of line art as well as text".
    pub line_art: bool,
}

impl Default for ColorSettings {
    fn default() -> Self {
        ColorSettings {
            enabled: false,
            high_contrast: true,
            pair: 0,
            custom_background: [255, 255, 255],
            custom_text: [0, 0, 0],
            only_black: false,
            line_art: false,
        }
    }
}

impl ColorSettings {
    /// What pages are drawn with, or `None` for the document's own colours.
    pub fn replace(&self) -> Option<ReplaceColors> {
        if !self.enabled {
            return None;
        }
        let base = if self.high_contrast {
            ReplaceColors::HIGH_CONTRAST
                .get(self.pair)
                .map(|(_, c)| *c)
                .unwrap_or(ReplaceColors::HIGH_CONTRAST[0].1)
        } else {
            ReplaceColors::pair(self.custom_background, self.custom_text)
        };
        Some(ReplaceColors {
            line_art: self.line_art,
            only_black: self.only_black,
            ..base
        })
    }
}

impl App {
    /// Apply the colour settings to every open document.
    pub fn apply_colors(&mut self) {
        let r = self.colors.replace();
        for t in &mut self.tabs {
            t.refresh_view(r);
        }
    }

    /// View → Full Screen Mode (Ctrl+L): one page at a time, fitted, with
    /// menus, toolbars and panes hidden; Esc leaves it (as Acrobat does).
    pub fn set_full_screen(&mut self, ctx: &egui::Context, on: bool) {
        if on == self.full_screen {
            return;
        }
        self.full_screen = on;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
        let Some(t) = self.tabs.get_mut(self.active) else {
            return;
        };
        if on {
            self.saved_view = Some((t.layout, t.zoom));
            t.set_layout(Layout::SinglePage);
            t.set_zoom(Zoom::FitPage);
        } else if let Some((layout, zoom)) = self.saved_view.take() {
            t.set_layout(layout);
            t.set_zoom(zoom);
        }
    }

    /// View → Read Mode (Ctrl+H): toolbars, tabs, panes and status bar hidden.
    pub fn toggle_read_mode(&mut self) {
        self.read_mode = !self.read_mode;
    }

    pub(crate) fn layers_pane(&mut self, ui: &mut egui::Ui) {
        ui.strong("Layers");
        let replace = self.colors.replace();
        let Some(t) = self.tabs.get_mut(self.active) else {
            return;
        };
        if t.layers.is_empty() {
            ui.weak("This document has no layers.");
            return;
        }
        let reset = ui.button("Reset to Initial Visibility").clicked();
        let mut changes = Vec::new();
        ScrollArea::both()
            .id_salt(("layers", t.id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for l in &t.layers {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0 * l.depth as f32);
                        match l.ocg {
                            Some(r) => {
                                let mut on = !t.hidden_layers.contains(&r.num);
                                let resp = ui.add_enabled(
                                    !l.locked,
                                    egui::Checkbox::new(
                                        &mut on,
                                        crate::drawable(ui.ctx(), &l.name),
                                    ),
                                );
                                let resp = if l.locked {
                                    resp.on_disabled_hover_text("This layer is locked.")
                                } else {
                                    resp
                                };
                                if resp.changed() {
                                    changes.push((r.num, on));
                                }
                            }
                            None => {
                                ui.label(crate::drawable(ui.ctx(), &l.name));
                            }
                        }
                    });
                }
            });
        for (num, on) in changes {
            t.set_layer_visible(num, on);
        }
        if reset {
            t.reset_layers();
        }
        t.refresh_view(replace);
    }

    pub(crate) fn attachments_pane(&mut self, ui: &mut egui::Ui) {
        ui.strong("Attachments");
        let Some(t) = self.tabs.get(self.active) else {
            return;
        };
        let list = attachments::list(&t.doc);
        if list.is_empty() {
            ui.weak("This document has no attachments.");
            return;
        }
        let mut save = None;
        let mut go = None;
        ScrollArea::both()
            .id_salt(("attachments", t.id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (i, a) in list.iter().enumerate() {
                    ui.strong(crate::drawable(ui.ctx(), &a.name));
                    if let Some(d) = &a.description {
                        ui.label(crate::drawable(ui.ctx(), d));
                    }
                    ui.horizontal_wrapped(|ui| {
                        if let Some(size) = a.size {
                            ui.weak(info::human_size(size));
                        }
                        if let Some(p) = a.page
                            && ui
                                .link(format!(
                                    "Page {}",
                                    crate::drawable(ui.ctx(), &t.page_name(p))
                                ))
                                .clicked()
                        {
                            go = Some(p);
                        }
                        if ui.button("Save…").clicked() {
                            save = Some(i);
                        }
                    });
                    ui.separator();
                }
            });
        if let Some(p) = go
            && let Some(t) = self.tabs.get_mut(self.active)
        {
            t.go_to(p);
        }
        if let Some(i) = save
            && let Some(a) = list.get(i)
        {
            self.save_attachment(a);
        }
    }

    fn save_attachment(&mut self, a: &attachments::Attachment) {
        let Some(t) = self.tabs.get(self.active) else {
            return;
        };
        let data = match attachments::contents(&t.doc, a) {
            Ok(d) => d,
            Err(e) => {
                self.message = Some(format!("Could not read {}: {e}", a.name));
                return;
            }
        };
        let Some(saver) = self.saver.as_mut() else {
            self.message = Some("Saving is not available here.".into());
            return;
        };
        let Some(path) = saver(&a.name) else {
            return;
        };
        self.message = Some(match std::fs::write(&path, data) {
            Ok(()) => format!("Saved {}", path.display()),
            Err(e) => format!("Could not save {}: {e}", path.display()),
        });
    }

    /// File → Properties (Ctrl+D): Description, Security and Fonts tabs.
    pub(crate) fn properties_dialog(&mut self, ctx: &egui::Context) {
        if !self.properties_open {
            return;
        }
        let Some(t) = self.tabs.get(self.active) else {
            self.properties_open = false;
            return;
        };
        let mut open = true;
        let mut which = self.properties_tab;
        egui::Window::new("Document Properties")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([460.0, 420.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut which, 0, "Description");
                    ui.selectable_value(&mut which, 1, "Security");
                    ui.selectable_value(&mut which, 2, "Fonts");
                });
                ui.separator();
                ScrollArea::vertical().show(ui, |ui| match which {
                    0 => {
                        let size = t
                            .path
                            .as_deref()
                            .and_then(|p| std::fs::metadata(p).ok())
                            .map(|m| m.len());
                        match info::info(&t.doc, size) {
                            Ok(i) => {
                                if let Some(p) = &t.path {
                                    ui.label(crate::drawable(
                                        ui.ctx(),
                                        &format!("File:  {}", p.display()),
                                    ));
                                }
                                ui.monospace(crate::drawable(ui.ctx(), &i.to_text()));
                            }
                            Err(e) => {
                                ui.label(crate::drawable(ui.ctx(), &e.to_string()));
                            }
                        }
                    }
                    1 => {
                        ui.monospace(crate::drawable(
                            ui.ctx(),
                            &security::to_text(&security::summary(&t.doc)),
                        ));
                    }
                    _ => {
                        let list = fonts::list(&t.doc);
                        if list.is_empty() {
                            ui.weak("This document uses no fonts.");
                        }
                        for f in list {
                            ui.strong(crate::drawable(ui.ctx(), &f.name));
                            let mut lines = vec![format!("Type: {}", f.kind)];
                            if let Some(enc) = &f.encoding {
                                lines.push(format!("Encoding: {enc}"));
                            }
                            lines.push(
                                match (f.embedded, f.subset) {
                                    (true, true) => "Embedded Subset",
                                    (true, false) => "Embedded",
                                    _ => "Not embedded",
                                }
                                .to_string(),
                            );
                            ui.label(crate::drawable(ui.ctx(), &lines.join("   ")));
                            ui.add_space(4.0);
                        }
                    }
                });
            });
        self.properties_tab = which;
        if !open {
            self.properties_open = false;
        }
    }

    /// Edit → Accessibility → Replace Document Colors.
    pub(crate) fn colors_dialog(&mut self, ctx: &egui::Context) {
        if !self.colors_open {
            return;
        }
        let mut open = true;
        let mut c = self.colors;
        egui::Window::new("Document Colors Options")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.checkbox(&mut c.enabled, "Replace Document Colors");
                ui.add_enabled_ui(c.enabled, |ui| {
                    ui.radio_value(&mut c.high_contrast, true, "Use High-Contrast colors");
                    ui.add_enabled_ui(c.high_contrast, |ui| {
                        let current = ReplaceColors::HIGH_CONTRAST
                            .get(c.pair)
                            .map(|(n, _)| *n)
                            .unwrap_or_default();
                        egui::ComboBox::from_label("Color combination")
                            .selected_text(current)
                            .show_ui(ui, |ui| {
                                for (i, (name, _)) in
                                    ReplaceColors::HIGH_CONTRAST.iter().enumerate()
                                {
                                    ui.selectable_value(&mut c.pair, i, *name);
                                }
                            });
                    });
                    ui.radio_value(&mut c.high_contrast, false, "Custom Color");
                    ui.add_enabled_ui(!c.high_contrast, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Page Background:");
                            egui::color_picker::color_edit_button_srgb(
                                ui,
                                &mut c.custom_background,
                            )
                            .widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::Role::ColorWell,
                                    true,
                                    "Page Background color",
                                )
                            });
                            ui.label("Document Text:");
                            egui::color_picker::color_edit_button_srgb(ui, &mut c.custom_text)
                                .widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::Role::ColorWell,
                                        true,
                                        "Document Text color",
                                    )
                                });
                        });
                    });
                    ui.checkbox(
                        &mut c.only_black,
                        "Only change the color of black text or line art",
                    );
                    ui.checkbox(
                        &mut c.line_art,
                        "Change the color of line art as well as text",
                    );
                });
            });
        if c != self.colors {
            self.colors = c;
            self.apply_colors();
        }
        if !open {
            self.colors_open = false;
        }
    }
}
