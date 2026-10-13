#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;

use folio_core::Document;

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("folio-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn check_reports_a_healthy_file_as_json() {
    let path = tmp("healthy.pdf");
    let mut doc = Document::new();
    doc.add_text_page("hi").unwrap();
    doc.save(&path).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_folio"))
        .args(["check", "--json"])
        .arg(&path)
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(out.status.success(), "{text}");
    assert!(text.contains("\"pages\":1"), "{text}");
}

#[test]
fn check_repairs_a_damaged_file_and_refuses_to_overwrite() {
    let path = tmp("damaged.pdf");
    let mut doc = Document::new();
    doc.add_text_page("hi").unwrap();
    let mut bytes = doc.to_bytes().unwrap();
    let at = bytes.windows(4).position(|w| w == b"xref").unwrap();
    bytes.truncate(at);
    std::fs::write(&path, &bytes).unwrap();

    let fixed = tmp("fixed.pdf");
    let _ = std::fs::remove_file(&fixed);
    let out = Command::new(env!("CARGO_BIN_EXE_folio"))
        .arg("check")
        .arg(&path)
        .arg("--repair")
        .arg(&fixed)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(Document::load(&fixed).unwrap().pages().unwrap().len(), 1);

    let again = Command::new(env!("CARGO_BIN_EXE_folio"))
        .arg("check")
        .arg(&path)
        .arg("--repair")
        .arg(&fixed)
        .output()
        .unwrap();
    assert_eq!(again.status.code(), Some(2));
}

#[test]
fn missing_file_is_a_clean_error() {
    let out = Command::new(env!("CARGO_BIN_EXE_folio"))
        .args(["check", "--json", "definitely-not-here.pdf"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).contains("\"ok\":false"));
}
