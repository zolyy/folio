//! The reading window driven the way a person (or an agent) would: through
//! its buttons, boxes and keyboard shortcuts, found by their accessible
//! names (egui_kittest + AccessKit, no screen needed). The harness also
//! fails on any control without a name and on any character the fonts
//! cannot draw.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use eframe::egui::{Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use folio_app::app::{App, Config, Pane, Shown};
use folio_app::viewer::{Layout, Zoom};
use folio_core::Document;
use folio_core::crypt::Settings;
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

/// Run frames until the background renderer has nothing left to draw.
fn settle(h: &mut Harness<'static, App>) {
    for _ in 0..40 {
        h.run_ok();
        assert!(h.state().renderer.wait_idle(Duration::from_secs(60)));
        h.run_ok();
        if !h.state().renderer.busy() {
            h.run_ok();
            if !h.state().renderer.busy() {
                return;
            }
        }
    }
    panic!("the renderer never went idle");
}

fn bytes(doc: &Document) -> Vec<u8> {
    doc.to_bytes().unwrap()
}

fn with_rich() -> Harness<'static, App> {
    let mut a = app();
    a.open_bytes(bytes(&samples::rich().unwrap()), None, "rich.pdf".into());
    let mut h = harness(a);
    settle(&mut h);
    h
}

fn current(h: &Harness<'static, App>) -> usize {
    h.state().tab().unwrap().current
}

#[test]
fn empty_window_invites_to_open_a_file() {
    let mut h = harness(app());
    h.run();
    h.get_by_label_contains("Open a PDF");
    h.get_by_label("File");
    assert!(h.query_by_label("Next Page").is_none());
}

#[test]
fn opens_a_document_in_a_tab_and_draws_its_pages() {
    let h = with_rich();
    assert_eq!(h.state().tabs.len(), 1);
    h.get_by_label("rich.pdf");
    h.get_by_label("Close rich.pdf");
    h.get_by_label_contains("Page 1 of 3");
    // The visible pages and their thumbnails were drawn and kept.
    assert!(h.state().renderer.cached_bytes() > 0);
    let t = h.state().tab().unwrap();
    let key = folio_app::cache::Key {
        doc: t.id,
        page: 0,
        width: 1,
        rotation: 0,
        view: 0,
    };
    let mut app = h.into_state();
    match app.renderer.get(key) {
        Some((_, Shown::Texture(_))) => {}
        Some((_, Shown::Failed(m))) => panic!("page 1 failed: {m}"),
        None => panic!("page 1 was never drawn"),
    }
}

#[test]
fn toolbar_navigates_like_acrobat() {
    let mut h = with_rich();
    h.get_by_label("Next Page").click();
    h.run();
    assert_eq!(current(&h), 1);
    // The page box shows the page label.
    assert_eq!(h.get_by_label("Page Number").value().as_deref(), Some("ii"));
    h.get_by_label_contains("(2 of 3)");
    h.get_by_label("Last Page").click();
    h.run();
    assert_eq!(current(&h), 2);
    h.get_by_label("Previous Page").click();
    h.run();
    assert_eq!(current(&h), 1);
    h.get_by_label("First Page").click();
    h.run();
    assert_eq!(current(&h), 0);
}

#[test]
fn page_box_accepts_labels_and_numbers() {
    let mut h = with_rich();
    for (typed, want) in [("A-1", 2), ("2", 1), ("nonsense", 1)] {
        h.get_by_label("Page Number").focus();
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.get_by_label("Page Number").type_text(typed);
        h.key_press(Key::Enter);
        h.run();
        assert_eq!(current(&h), want, "after typing {typed:?}");
    }
}

#[test]
fn zoom_controls_and_shortcuts() {
    let mut h = with_rich();
    assert_eq!(h.state().tab().unwrap().zoom, Zoom::FitPage);
    h.get_by_label("Zoom In").click();
    h.run();
    assert!(matches!(h.state().tab().unwrap().zoom, Zoom::Percent(_)));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Num1);
    h.run();
    assert_eq!(h.state().tab().unwrap().zoom, Zoom::Percent(100.0));
    assert_eq!(h.get_by_label("Zoom").value().as_deref(), Some("100%"));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Minus);
    h.run();
    assert_eq!(h.state().tab().unwrap().zoom, Zoom::Percent(75.0));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Equals);
    h.run();
    assert_eq!(h.state().tab().unwrap().zoom, Zoom::Percent(100.0));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Num2);
    h.run();
    assert_eq!(h.state().tab().unwrap().zoom, Zoom::FitWidth);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Num0);
    h.run();
    assert_eq!(h.state().tab().unwrap().zoom, Zoom::FitPage);
    // Typed magnification.
    h.get_by_label("Zoom").focus();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_label("Zoom").type_text("250");
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().tab().unwrap().zoom, Zoom::Percent(250.0));
    settle(&mut h);
}

#[test]
fn keyboard_moves_between_pages() {
    let mut h = with_rich();
    h.key_press(Key::End);
    h.run();
    assert_eq!(current(&h), 2);
    h.key_press(Key::Home);
    h.run();
    assert_eq!(current(&h), 0);
    h.key_press(Key::ArrowRight);
    h.run();
    assert_eq!(current(&h), 1);
    h.key_press(Key::PageUp);
    h.run();
    assert_eq!(current(&h), 0);
}

#[test]
fn rotate_view_and_page_display() {
    let mut h = with_rich();
    h.get_by_label("Rotate Clockwise").click();
    h.run();
    assert_eq!(h.state().tab().unwrap().rotation, 90);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Minus);
    h.run();
    h.get_by_label("Rotate Counterclockwise").click();
    h.run();
    assert_eq!(h.state().tab().unwrap().rotation, 270);
    settle(&mut h);
    h.get_by_label("Single Page").click();
    h.run();
    assert_eq!(h.state().tab().unwrap().layout, Layout::SinglePage);
    h.get_by_label("Next Page").click();
    settle(&mut h);
    assert_eq!(current(&h), 1);
    h.get_by_label("Scrolling").click();
    h.run();
    assert_eq!(h.state().tab().unwrap().layout, Layout::Continuous);
    assert_eq!(current(&h), 1);
}

#[test]
fn thumbnails_go_to_their_page() {
    let mut h = with_rich();
    assert_eq!(h.state().pane, Pane::Thumbnails);
    h.get_by_label("Page Thumbnails");
    h.get_by_label("Page A-1").click();
    h.run();
    assert_eq!(current(&h), 2);
    h.key_press(Key::F4);
    h.run();
    assert_eq!(h.state().pane, Pane::Hidden);
    assert!(h.query_by_label("Page A-1").is_none());
}

#[test]
fn bookmarks_go_to_their_destination() {
    let mut h = with_rich();
    h.state_mut().pane = Pane::Bookmarks;
    h.run();
    let target = {
        let t = h.state().tab().unwrap();
        t.outline[1].page.and_then(|p| t.index_of(p)).unwrap()
    };
    h.get_by_label("Capitolul 2 — ș").click();
    h.run();
    assert_eq!(current(&h), target);
    h.get_by_label("Chapter 1").click();
    h.run();
    let first = {
        let t = h.state().tab().unwrap();
        t.outline[0].page.and_then(|p| t.index_of(p)).unwrap()
    };
    assert_eq!(current(&h), first);
}

#[test]
fn document_without_bookmarks_says_so() {
    let mut a = app();
    a.open_bytes(
        bytes(&samples::numbered(2).unwrap()),
        None,
        "plain.pdf".into(),
    );
    a.pane = Pane::Bookmarks;
    let mut h = harness(a);
    settle(&mut h);
    h.get_by_label("This document has no bookmarks.");
    // Without labels the page box shows numbers.
    h.get_by_label_contains("/ 2");
}

#[test]
fn tabs_switch_and_close() {
    let mut a = app();
    a.open_bytes(bytes(&samples::rich().unwrap()), None, "one.pdf".into());
    a.open_bytes(
        bytes(&samples::numbered(4).unwrap()),
        None,
        "two.pdf".into(),
    );
    let mut h = harness(a);
    settle(&mut h);
    assert_eq!(h.state().active, 1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Tab);
    h.run();
    assert_eq!(h.state().active, 0);
    h.get_by_label("two.pdf").click();
    h.run();
    assert_eq!(h.state().active, 1);
    h.get_by_label("Close two.pdf").click();
    h.run();
    assert_eq!(h.state().tabs.len(), 1);
    assert_eq!(h.state().tab().unwrap().title, "one.pdf");
    h.key_press_modifiers(Modifiers::COMMAND, Key::W);
    h.run();
    assert!(h.state().tabs.is_empty());
    h.get_by_label_contains("Open a PDF");
}

#[test]
fn protected_document_asks_for_its_password() {
    let mut doc = samples::numbered(2).unwrap();
    doc.encrypt(&Settings {
        open_password: Some("open sesame".into()),
        ..Settings::default()
    })
    .unwrap();
    let mut a = app();
    a.open_bytes(bytes(&doc), None, "secret.pdf".into());
    assert!(a.tabs.is_empty());
    let mut h = harness(a);
    h.run();
    h.get_by_label_contains("'secret.pdf' is protected");
    h.get_by_label("Enter Password").type_text("wrong");
    h.get_by_label("OK").click();
    h.run();
    h.get_by_label("Incorrect password. Try again.");
    assert!(h.state().tabs.is_empty());
    h.get_by_label("Enter Password").type_text("open sesame");
    h.key_press(Key::Enter);
    settle(&mut h);
    assert!(h.state().password.is_none());
    assert_eq!(h.state().tabs.len(), 1);
    h.get_by_label_contains("Page 1 of 2");
}

#[test]
fn cancelled_password_opens_nothing() {
    let mut doc = samples::numbered(1).unwrap();
    doc.encrypt(&Settings {
        open_password: Some("x".into()),
        ..Settings::default()
    })
    .unwrap();
    let mut a = app();
    a.open_bytes(bytes(&doc), None, "secret.pdf".into());
    let mut h = harness(a);
    h.run();
    h.get_by_label("Cancel").click();
    h.run();
    assert!(h.state().password.is_none() && h.state().tabs.is_empty());
}

#[test]
fn damaged_file_opens_and_says_it_was_repaired() {
    let good = bytes(&samples::numbered(2).unwrap());
    let text = String::from_utf8_lossy(&good).into_owned();
    let at = text.rfind("startxref").unwrap();
    let mut bad = good[..at].to_vec();
    bad.extend_from_slice(b"startxref\n999999999\n%%EOF\n");
    let mut a = app();
    a.open_bytes(bad, None, "damaged.pdf".into());
    let mut h = harness(a);
    settle(&mut h);
    assert_eq!(h.state().tabs.len(), 1);
    h.get_by_label_contains("Repaired:");
}

#[test]
fn unreadable_file_is_reported_not_fatal() {
    let mut a = app();
    a.open_bytes(
        b"this is not a PDF at all".to_vec(),
        None,
        "junk.pdf".into(),
    );
    a.open_path(std::path::Path::new("Z:/no/such/file.pdf"));
    let mut h = harness(a);
    h.run();
    assert!(h.state().tabs.is_empty());
    assert!(h.state().message.is_some());
    h.get_by_label("Dismiss").click();
    h.run();
    assert!(h.state().message.is_none());
}

#[test]
fn open_recent_remembers_files_on_disk() {
    let dir = std::env::temp_dir().join(format!("folio-app-recent-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pdf = dir.join("kept.pdf");
    std::fs::write(&pdf, bytes(&samples::numbered(1).unwrap())).unwrap();
    let list = dir.join("recent.txt");
    let mut a = App::new(Config {
        recent_file: Some(list.clone()),
        threads: 1,
    });
    a.open_path(&pdf);
    // Opening the same file again switches to its tab instead of a copy.
    a.open_path(&pdf);
    assert_eq!(a.tabs.len(), 1);
    drop(a);
    let b = App::new(Config {
        recent_file: Some(list),
        threads: 1,
    });
    assert_eq!(b.recent, vec![pdf]);
    std::fs::remove_dir_all(&dir).ok();
}

/// Regression: at high zoom a page image larger than the display's texture
/// limit made egui panic ("maximum texture side"). Views beyond the limit
/// are now drawn at the limit and stretched.
#[test]
fn extreme_zoom_stays_within_the_display_texture_limit() {
    let mut h = with_rich();
    let limit = h.state().renderer.max_side();
    for z in ["6400", "1600", "800"] {
        let t = h.state_mut().tab_mut().unwrap();
        assert!(t.set_zoom_text(z));
        settle(&mut h);
        h.state_mut().tab_mut().unwrap().rotate_clockwise();
        settle(&mut h);
    }
    let t = h.state().tab().unwrap();
    let id = t.id;
    let mut app = h.into_state();
    let key = folio_app::cache::Key {
        doc: id,
        page: 0,
        width: limit,
        rotation: 270,
        view: 0,
    };
    match app.renderer.get(key) {
        Some((k, Shown::Texture(tex))) => {
            let [w, ht] = tex.size();
            assert!(
                w as u32 <= limit && ht as u32 <= limit,
                "{w} x {ht} for {k:?}"
            );
        }
        Some((_, Shown::Failed(m))) => panic!("{m}"),
        None => panic!("nothing drawn"),
    }
}
