//! Stage 4b: Find, highlighting, Select All and Copy, through the window.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use eframe::egui::{Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use folio_app::app::{App, Config};
use folio_core::Document;
use folio_core::crypt::Settings;
use folio_ops::samples;

fn harness(doc: &Document, name: &str) -> Harness<'static, App> {
    let mut a = App::new(Config {
        recent_file: None,
        threads: 2,
    });
    a.open_bytes(doc.to_bytes().unwrap(), None, name.into());
    // Read the text now, so the tests do not race the background reader.
    {
        let t = a.tabs.last_mut().unwrap();
        let (d, p) = (t.doc.clone(), t.pages.clone());
        t.index.fill_now(&d, &p);
    }
    let mut h = Harness::builder()
        .with_size(Vec2::new(1100.0, 800.0))
        .with_max_steps(12)
        .build_ui_state(|ui, app: &mut App| app.ui(ui), a);
    h.run_ok();
    assert!(h.state().renderer.wait_idle(Duration::from_secs(60)));
    h.run_ok();
    h
}

fn current(h: &Harness<'static, App>) -> usize {
    h.state().tab().unwrap().current
}

#[test]
fn ctrl_f_opens_find_and_steps_through_matches_across_pages() {
    let mut h = harness(&samples::numbered(5).unwrap(), "five.pdf");
    h.key_press_modifiers(Modifiers::COMMAND, Key::F);
    h.run_ok();
    h.get_by_label("Whole words only");
    h.get_by_label("Case-sensitive");
    h.state_mut().find.query = "page".into();
    h.run_ok();
    h.get_by_label("5 matches");
    // F3 goes to the next match and shows its page.
    h.key_press(Key::F3);
    h.run_ok();
    h.get_by_label("1 of 5");
    h.key_press(Key::F3);
    h.run_ok();
    h.get_by_label("2 of 5");
    assert_eq!(current(&h), 1);
    // Shift+F3 goes back; the end wraps to the start.
    h.key_press_modifiers(Modifiers::SHIFT, Key::F3);
    h.run_ok();
    assert_eq!(current(&h), 0);
    h.key_press_modifiers(Modifiers::SHIFT, Key::F3);
    h.run_ok();
    h.get_by_label("5 of 5");
    assert_eq!(current(&h), 4);
    // Escape closes the bar.
    h.key_press(Key::Escape);
    h.run_ok();
    assert!(h.query_by_label("Whole words only").is_none());
}

#[test]
fn find_options_whole_words_and_case() {
    let mut h = harness(&samples::rich().unwrap(), "rich.pdf");
    h.state_mut().open_find();
    h.state_mut().find.query = "hello".into();
    h.run_ok();
    h.get_by_label("1 match");
    h.state_mut().find.case_sensitive = true;
    h.run_ok();
    h.get_by_label("No matches");
    h.state_mut().find.case_sensitive = false;
    h.state_mut().find.query = "Fol".into();
    h.run_ok();
    h.get_by_label("1 match");
    h.state_mut().find.whole_words = true;
    h.run_ok();
    h.get_by_label("No matches");
}

#[test]
fn select_all_and_copy_respect_document_security() {
    let doc = samples::numbered(2).unwrap();
    let mut h = harness(&doc, "two.pdf");
    h.state_mut().select_all();
    h.run_ok();
    let sel = h.state().tab().unwrap().selection.unwrap();
    let (text, missing) = sel.text(&h.state().tab().unwrap().index);
    assert_eq!(missing, 0);
    assert_eq!(text, "Page 1\nPage 2");
    let ctx = h.ctx.clone();
    h.state_mut().copy_selection(&ctx);
    assert!(h.state().message.is_none());

    // A document that forbids copying: Find still works, Copy says why not.
    let mut locked = samples::numbered(1).unwrap();
    locked
        .encrypt(&Settings {
            permissions_password: Some("owner".into()),
            copy: false,
            ..Settings::default()
        })
        .unwrap();
    let mut h = harness(
        &Document::from_bytes(&locked.to_bytes().unwrap()).unwrap(),
        "locked.pdf",
    );
    h.state_mut().open_find();
    h.state_mut().find.query = "Page 1".into();
    h.run_ok();
    h.get_by_label("1 match");
    h.state_mut().select_all();
    let ctx = h.ctx.clone();
    h.state_mut().copy_selection(&ctx);
    assert!(
        h.state()
            .message
            .as_deref()
            .is_some_and(|m| m.contains("does not allow copying"))
    );
}

#[test]
fn edit_menu_offers_copy_select_all_and_find() {
    let mut h = harness(&samples::numbered(1).unwrap(), "one.pdf");
    h.get_by_label("Edit").click();
    h.run_ok();
    h.get_by_label("Select All Ctrl+A").click();
    h.run_ok();
    assert!(h.state().tab().unwrap().selection.is_some());
    h.get_by_label("Edit").click();
    h.run_ok();
    h.get_by_label("Find Ctrl+F").click();
    h.run_ok();
    assert!(h.state().find.open);
}
