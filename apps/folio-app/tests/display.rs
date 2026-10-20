//! Stage 4c in the reading window: two-page layouts, Full Screen and Read
//! Mode, the Layers and Attachments panes, Document Properties and Replace
//! Document Colors — driven through the UI where a person would.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use eframe::egui::{Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use folio_app::app::{App, Config, Pane};
use folio_app::viewer::{Layout, Tab, Zoom};
use folio_core::text_string::encode_text;
use folio_core::{Dict, Document, Object, Stream};
use folio_ops::samples;

fn app() -> App {
    App::new(Config {
        recent_file: None,
        threads: 2,
    })
}

fn harness(app: App) -> Harness<'static, App> {
    Harness::builder()
        .with_size(Vec2::new(1100.0, 800.0))
        .with_max_steps(12)
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app)
}

fn settle(h: &mut Harness<'static, App>) {
    for _ in 0..40 {
        h.run_ok();
        assert!(h.state().renderer.wait_idle(Duration::from_secs(60)));
        h.run_ok();
        if !h.state().renderer.busy() {
            return;
        }
    }
    panic!("the renderer never went idle");
}

/// Two layers ("Rosu" on and locked, "Albastru" off) and one attachment.
fn layered() -> Document {
    let mut doc = Document::new();
    let ocg = |doc: &mut Document, name: &str| {
        let mut d = Dict::new();
        d.set("Type", Object::name("OCG"));
        d.set("Name", Object::Str(name.as_bytes().to_vec()));
        doc.add(d.into())
    };
    let red = ocg(&mut doc, "Rosu");
    let blue = ocg(&mut doc, "Albastru");
    let mut config = Dict::new();
    config.set("Order", Object::Array(vec![red.into(), blue.into()]));
    config.set("OFF", Object::Array(vec![blue.into()]));
    config.set("Locked", Object::Array(vec![red.into()]));
    let mut ocp = Dict::new();
    ocp.set("OCGs", Object::Array(vec![red.into(), blue.into()]));
    ocp.set("D", config.into());
    let mut sd = Dict::new();
    sd.set("Type", Object::name("EmbeddedFile"));
    let ef = doc.add(Object::Stream(Stream {
        dict: sd,
        data: b"salut".to_vec(),
    }));
    let mut efd = Dict::new();
    efd.set("F", ef.into());
    let mut fs = Dict::new();
    fs.set("Type", Object::name("Filespec"));
    fs.set("UF", Object::Str(encode_text("salut.txt")));
    fs.set("EF", efd.into());
    let fs = doc.add(fs.into());
    let mut tree = Dict::new();
    tree.set(
        "Names",
        Object::Array(vec![Object::Str(b"salut.txt".to_vec()), fs.into()]),
    );
    let mut names = Dict::new();
    names.set("EmbeddedFiles", tree.into());
    let cat = doc.catalog_mut().unwrap();
    cat.set("OCProperties", ocp.into());
    cat.set("Names", names.into());
    let mut props = Dict::new();
    props.set("P1", red.into());
    props.set("P2", blue.into());
    let mut res = Dict::new();
    res.set("Properties", props.into());
    doc.add_page(
        [100.0, 50.0],
        b"/OC /P1 BDC 1 0 0 rg 10 10 30 30 re f EMC /OC /P2 BDC 0 0 1 rg 60 10 30 30 re f EMC",
        res,
    )
    .unwrap();
    doc
}

fn tab(doc: Document) -> Tab {
    Tab::new(doc, None, "t.pdf".into(), 1)
}

// ---- layouts (viewer model) ----------------------------------------------

#[test]
fn two_page_view_pairs_pages_and_steps_by_spread() {
    let mut t = tab(samples::numbered(5).unwrap());
    t.set_layout(Layout::TwoPage);
    assert_eq!(t.spreads(), vec![vec![0, 1], vec![2, 3], vec![4]]);
    let (placed, _, _) = t.placements(1.0);
    assert_eq!(
        placed.iter().map(|p| p.page).collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert!(
        placed[0].x < 0.0 && placed[1].x > 0.0,
        "left and right of the centre"
    );
    t.next_page();
    assert_eq!(t.current, 2);
    t.next_page();
    t.next_page();
    assert_eq!(t.current, 4, "stays on the last spread");
    t.previous_page();
    assert_eq!(t.current, 2);
}

#[test]
fn show_cover_page_puts_the_first_page_alone_on_the_right() {
    let mut t = tab(samples::numbered(4).unwrap());
    t.set_layout(Layout::TwoPageScrolling);
    t.set_cover_page(true);
    assert_eq!(t.spreads(), vec![vec![0], vec![1, 2], vec![3]]);
    let (placed, total, width) = t.placements(1.0);
    assert_eq!(placed.len(), 4);
    assert!(placed[0].x > 0.0, "the cover is on the right");
    assert!(placed[3].x < 0.0, "a last single page is on the left");
    assert_eq!(placed[1].top, placed[2].top, "a spread shares one row");
    assert!(total > placed[3].top && width > 2.0 * 612.0);
}

#[test]
fn fit_page_fits_the_whole_spread() {
    let mut t = tab(samples::numbered(2).unwrap());
    let single = t.scale([1000.0, 1000.0]);
    t.set_layout(Layout::TwoPage);
    t.set_zoom(Zoom::FitPage);
    let two = t.scale([1000.0, 1000.0]);
    assert!(two < single, "two pages side by side need a smaller scale");
}

// ---- layers (viewer model) -----------------------------------------------

#[test]
fn layer_visibility_changes_what_is_drawn_and_locks_hold() {
    let mut t = tab(layered());
    assert_eq!(t.layers.len(), 2);
    let red = t.layers[0].ocg.unwrap().num;
    let blue = t.layers[1].ocg.unwrap().num;
    assert!(t.hidden_layers.contains(&blue));
    assert_eq!(t.view.key(), 0, "the default state draws like the document");
    t.set_layer_visible(blue, true);
    assert!(!t.hidden_layers.contains(&blue));
    assert_ne!(t.view.key(), 0);
    t.set_layer_visible(red, false);
    assert!(
        !t.hidden_layers.contains(&red),
        "a locked layer cannot be hidden"
    );
    t.reset_layers();
    assert!(t.hidden_layers.contains(&blue));
    assert_eq!(t.view.key(), 0);
}

// ---- the window ------------------------------------------------------------

fn with(doc: Document, name: &str) -> Harness<'static, App> {
    let mut a = app();
    a.open_bytes(doc.to_bytes().unwrap(), None, name.into());
    let mut h = harness(a);
    settle(&mut h);
    h
}

#[test]
fn full_screen_hides_the_chrome_and_esc_restores_the_view() {
    let mut h = with(samples::numbered(3).unwrap(), "n.pdf");
    h.key_press_modifiers(Modifiers::COMMAND, Key::L);
    h.run();
    assert!(h.state().full_screen);
    assert!(h.query_by_label("File").is_none(), "menus are hidden");
    assert_eq!(h.state().tab().unwrap().layout, Layout::SinglePage);
    h.key_press(Key::Escape);
    h.run();
    assert!(!h.state().full_screen);
    h.get_by_label("File");
    assert_eq!(h.state().tab().unwrap().layout, Layout::Continuous);
}

#[test]
fn read_mode_hides_toolbars_and_panes() {
    let mut h = with(samples::numbered(2).unwrap(), "n.pdf");
    h.get_by_label("Page Thumbnails");
    h.key_press_modifiers(Modifiers::COMMAND, Key::H);
    h.run();
    assert!(h.state().read_mode);
    assert!(h.query_by_label("Page Thumbnails").is_none());
    h.get_by_label("File");
    h.key_press_modifiers(Modifiers::COMMAND, Key::H);
    h.run();
    h.get_by_label("Page Thumbnails");
}

#[test]
fn document_properties_dialog_shows_description_security_and_fonts() {
    let mut h = with(samples::rich().unwrap(), "rich.pdf");
    h.key_press_modifiers(Modifiers::COMMAND, Key::D);
    h.run();
    h.get_by_label("Document Properties");
    h.get_by_label_contains("Rich sample");
    h.get_by_label("Security").click();
    h.run();
    h.get_by_label_contains("No Security");
    h.get_by_label("Fonts").click();
    h.run();
    h.get_by_label("Helvetica");
}

#[test]
fn layers_pane_toggles_a_layer() {
    let mut h = with(layered(), "layers.pdf");
    h.state_mut().pane = Pane::Layers;
    h.run();
    h.get_by_label("Reset to Initial Visibility");
    h.get_by_label("Albastru").click();
    h.run();
    let blue = h.state().tab().unwrap().layers[1].ocg.unwrap().num;
    assert!(!h.state().tab().unwrap().hidden_layers.contains(&blue));
    settle(&mut h);
    h.get_by_label("Reset to Initial Visibility").click();
    h.run();
    assert!(h.state().tab().unwrap().hidden_layers.contains(&blue));
}

#[test]
fn attachments_pane_lists_and_saves() {
    let dir = std::env::temp_dir().join(format!("folio-app-att-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("out.txt");
    let mut h = with(layered(), "att.pdf");
    let t2 = target.clone();
    h.state_mut().saver = Some(Box::new(move |name: &str| {
        assert_eq!(name, "salut.txt");
        Some(t2.clone())
    }));
    h.state_mut().pane = Pane::Attachments;
    h.run();
    h.get_by_label("salut.txt");
    h.get_by_label("Save…").click();
    h.run();
    assert_eq!(std::fs::read(&target).unwrap(), b"salut");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn replace_document_colors_reaches_every_tab() {
    let mut h = with(samples::numbered(1).unwrap(), "a.pdf");
    h.state_mut().open_bytes(
        samples::numbered(1).unwrap().to_bytes().unwrap(),
        None,
        "b.pdf".into(),
    );
    h.run();
    h.state_mut().colors.enabled = true;
    h.state_mut().colors.line_art = true;
    h.state_mut().apply_colors();
    for t in &h.state().tabs {
        let r = t.view.replace_colors.unwrap();
        assert_eq!(r.background, [0, 0, 0]);
        assert!(r.line_art);
    }
    // A document opened afterwards gets the same colours.
    h.state_mut().open_bytes(
        samples::numbered(1).unwrap().to_bytes().unwrap(),
        None,
        "c.pdf".into(),
    );
    assert!(h.state().tabs[2].view.replace_colors.is_some());
    settle(&mut h);
}

#[test]
fn thumbnails_can_be_enlarged_and_reduced() {
    let mut h = with(samples::numbered(2).unwrap(), "n.pdf");
    h.get_by_label("+").click();
    h.run();
    assert!(h.state().thumb_scale > 1.0);
    h.get_by_label("−").click();
    h.get_by_label("−").click();
    h.run();
    assert!(h.state().thumb_scale < 1.0);
}

#[test]
fn colors_dialog_opens_from_the_edit_menu() {
    let mut h = with(samples::numbered(1).unwrap(), "a.pdf");
    h.get_by_label("Edit").click();
    h.run();
    h.get_by_label("Accessibility: Replace Document Colors…")
        .click();
    h.run();
    h.get_by_label("Document Colors Options");
    h.get_by_label("Replace Document Colors").click();
    h.run();
    assert!(h.state().colors.enabled);
    assert!(h.state().tab().unwrap().view.replace_colors.is_some());
    h.get_by_label("Custom Color").click();
    h.run();
    assert!(!h.state().colors.high_contrast);
}

/// Regression (stage 4c gate, mutant 297): text from a document that the
/// window's fonts cannot draw must never reach egui as is.
#[test]
fn undrawable_text_in_titles_names_and_labels_is_shown_safely() {
    let odd = "x\u{E123}\u{10FFFD}\u{0007}\u{FFFF}y";
    let mut doc = layered();
    // Layer, attachment, bookmark, page label and document title.
    for o in doc.objects.values_mut() {
        if let Some(d) = o.as_dict_mut() {
            if d.get_name(b"Type") == Some(b"OCG") {
                d.set("Name", Object::Str(encode_text(odd)));
            }
            if d.get_name(b"Type") == Some(b"Filespec") {
                d.set("UF", Object::Str(encode_text(odd)));
                d.set("Desc", Object::Str(encode_text(odd)));
            }
        }
    }
    let page = doc.pages().unwrap()[0];
    let mut bm = Dict::new();
    bm.set("Title", Object::Str(encode_text(odd)));
    bm.set(
        "Dest",
        Object::Array(vec![page.into(), Object::name("Fit")]),
    );
    let root = doc.add(Object::Null);
    bm.set("Parent", root.into());
    let bm = doc.add(bm.into());
    let mut outlines = Dict::new();
    outlines.set("First", bm.into());
    outlines.set("Last", bm.into());
    doc.objects.insert(root.num, outlines.into());
    let mut lab = Dict::new();
    lab.set("S", Object::name("D"));
    lab.set("P", Object::Str(encode_text(odd)));
    let mut labels = Dict::new();
    labels.set("Nums", Object::Array(vec![Object::Int(0), lab.into()]));
    let cat = doc.catalog_mut().unwrap();
    cat.set("Outlines", root.into());
    cat.set("PageLabels", labels.into());
    doc.info_mut()
        .unwrap()
        .set("Title", Object::Str(encode_text(odd)));
    let mut h = with(doc, "odd.pdf");
    for pane in [
        Pane::Thumbnails,
        Pane::Bookmarks,
        Pane::Layers,
        Pane::Attachments,
    ] {
        h.state_mut().pane = pane;
        h.state_mut().properties_open = true;
        for tab in 0..3 {
            h.state_mut().properties_tab = tab;
            h.run();
        }
    }
    h.get_by_label("Document Properties");
}
