//! End-to-end: every stage-1 command through the real `folio` binary.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use folio_core::Document;
use folio_ops::{samples, text};

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let d = std::env::temp_dir().join(format!("folio-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }
    fn p(&self, f: &str) -> PathBuf {
        self.0.join(f)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn folio(args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_folio"))
        .args(args)
        .output()
        .unwrap();
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), s)
}

fn ok(args: &[&str]) -> String {
    let (code, s) = folio(args);
    assert_eq!(code, 0, "folio {args:?} failed:\n{s}");
    s
}

fn texts(p: &Path) -> Vec<String> {
    let d = Document::load(p).unwrap();
    d.pages()
        .unwrap()
        .into_iter()
        .map(|pg| text::page_text(&d, pg).unwrap())
        .collect()
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn cli_info_shows_document_properties() {
    let d = Dir::new("info");
    samples::rich().unwrap().save(d.p("r.pdf")).unwrap();
    let out = ok(&["info", s(&d.p("r.pdf"))]);
    assert!(out.contains("Title:         Rich sample"), "{out}");
    assert!(out.contains("Page Labels:   i, ii, A-1"), "{out}");
    let j = ok(&["--json", "info", s(&d.p("r.pdf"))]);
    assert!(
        j.contains("\"bookmarks\":[\"Chapter 1\",\"Capitolul 2 — ș\"]"),
        "{j}"
    );
}

#[test]
fn cli_merge_with_ranges_and_bookmarks() {
    let d = Dir::new("merge");
    samples::numbered(3).unwrap().save(d.p("a.pdf")).unwrap();
    samples::numbered(2).unwrap().save(d.p("b.pdf")).unwrap();
    let a = format!("{}:3,1", s(&d.p("a.pdf")));
    ok(&["merge", "-o", s(&d.p("m.pdf")), &a, s(&d.p("b.pdf"))]);
    assert_eq!(
        texts(&d.p("m.pdf")),
        vec!["Page 3", "Page 1", "Page 1", "Page 2"]
    );
    let info = ok(&["info", s(&d.p("m.pdf"))]);
    assert!(info.contains("Bookmarks:     a | b"), "{info}");
}

#[test]
fn cli_split_three_ways_with_part_names() {
    let d = Dir::new("split");
    samples::numbered(5).unwrap().save(d.p("doc.pdf")).unwrap();
    ok(&["split", s(&d.p("doc.pdf")), "--pages-per-file", "2"]);
    assert_eq!(texts(&d.p("doc_Part3.pdf")), vec!["Page 5"]);
    samples::rich().unwrap().save(d.p("rich.pdf")).unwrap();
    let out_dir = d.p("chapters");
    std::fs::create_dir_all(&out_dir).unwrap();
    ok(&[
        "split",
        s(&d.p("rich.pdf")),
        "--bookmarks",
        "--out-dir",
        s(&out_dir),
    ]);
    assert!(out_dir.join("Chapter 1.pdf").exists());
    assert!(out_dir.join("Capitolul 2 — ș.pdf").exists());
    let (code, msg) = folio(&["split", s(&d.p("doc.pdf")), "--pages-per-file", "2"]);
    assert_eq!(code, 2, "existing parts must not be overwritten: {msg}");
    ok(&[
        "split",
        s(&d.p("doc.pdf")),
        "--max-size",
        "0.001",
        "--force",
    ]);
}

#[test]
fn cli_extract_separate_and_delete_after() {
    let d = Dir::new("extract");
    samples::numbered(4).unwrap().save(d.p("doc.pdf")).unwrap();
    ok(&[
        "extract",
        s(&d.p("doc.pdf")),
        "--pages",
        "2-3",
        "-o",
        s(&d.p("x.pdf")),
    ]);
    assert_eq!(texts(&d.p("x.pdf")), vec!["Page 2", "Page 3"]);
    let (code, _) = folio(&[
        "extract",
        s(&d.p("doc.pdf")),
        "--pages",
        "1",
        "--separate",
        "--delete-after",
    ]);
    assert_eq!(code, 2, "--delete-after needs --force");
    ok(&[
        "extract",
        s(&d.p("doc.pdf")),
        "--pages",
        "1,4",
        "--separate",
        "--delete-after",
        "--force",
    ]);
    assert_eq!(texts(&d.p("doc 4.pdf")), vec!["Page 4"]);
    assert_eq!(texts(&d.p("doc.pdf")), vec!["Page 2", "Page 3"]);
}

#[test]
fn cli_delete_rotate_reorder_insert_replace() {
    let d = Dir::new("pages");
    samples::numbered(4).unwrap().save(d.p("doc.pdf")).unwrap();
    samples::numbered(2).unwrap().save(d.p("two.pdf")).unwrap();
    ok(&[
        "delete",
        s(&d.p("doc.pdf")),
        "--pages",
        "2",
        "-o",
        s(&d.p("del.pdf")),
    ]);
    assert_eq!(texts(&d.p("del.pdf")), vec!["Page 1", "Page 3", "Page 4"]);

    let j = ok(&[
        "--json",
        "rotate",
        s(&d.p("doc.pdf")),
        "--direction",
        "ccw",
        "--subset",
        "even",
        "-o",
        s(&d.p("rot.pdf")),
    ]);
    assert!(j.contains("\"rotated\":2"), "{j}");

    ok(&[
        "reorder",
        s(&d.p("doc.pdf")),
        "--reverse",
        "-o",
        s(&d.p("rev.pdf")),
    ]);
    assert_eq!(texts(&d.p("rev.pdf"))[0], "Page 4");

    ok(&[
        "insert",
        s(&d.p("doc.pdf")),
        "--from",
        s(&d.p("two.pdf")),
        "--from-pages",
        "2",
        "--before",
        "first",
        "-o",
        s(&d.p("ins.pdf")),
    ]);
    assert_eq!(texts(&d.p("ins.pdf"))[..2], ["Page 2", "Page 1"]);
    ok(&[
        "insert",
        s(&d.p("doc.pdf")),
        "--blank",
        "--count",
        "2",
        "--after",
        "2",
        "-o",
        s(&d.p("blank.pdf")),
    ]);
    assert_eq!(
        Document::load(d.p("blank.pdf"))
            .unwrap()
            .pages()
            .unwrap()
            .len(),
        6
    );

    ok(&[
        "replace",
        s(&d.p("doc.pdf")),
        "--pages",
        "3-4",
        "--from",
        s(&d.p("two.pdf")),
        "-o",
        s(&d.p("rep.pdf")),
    ]);
    assert_eq!(
        texts(&d.p("rep.pdf")),
        vec!["Page 1", "Page 2", "Page 1", "Page 2"]
    );
}

#[test]
fn cli_crop_labels_meta_compress_text() {
    let d = Dir::new("misc");
    samples::numbered(3).unwrap().save(d.p("doc.pdf")).unwrap();
    ok(&[
        "crop",
        s(&d.p("doc.pdf")),
        "--margins",
        "1",
        "--unit",
        "in",
        "-o",
        s(&d.p("crop.pdf")),
    ]);
    let j = ok(&["--json", "info", s(&d.p("crop.pdf"))]);
    assert!(j.contains("\"width_pt\":468"), "{j}");

    ok(&[
        "labels",
        s(&d.p("doc.pdf")),
        "--pages",
        "1-2",
        "--style",
        "roman",
        "-o",
        s(&d.p("lab.pdf")),
    ]);
    let i = ok(&["info", s(&d.p("lab.pdf"))]);
    assert!(i.contains("Page Labels:   i, ii, 3"), "{i}");
    let (code, _) = folio(&[
        "labels",
        s(&d.p("doc.pdf")),
        "--pages",
        "1,3",
        "-o",
        s(&d.p("x.pdf")),
    ]);
    assert_eq!(code, 2, "non-contiguous label ranges are refused");

    ok(&[
        "meta",
        s(&d.p("doc.pdf")),
        "--title",
        "Proiect",
        "--author",
        "Zoltán",
        "-o",
        s(&d.p("meta.pdf")),
    ]);
    let m = ok(&["meta", s(&d.p("meta.pdf"))]);
    assert!(
        m.contains("Title:    Proiect") && m.contains("Author:   Zoltán"),
        "{m}"
    );

    let j = ok(&[
        "--json",
        "compress",
        s(&d.p("doc.pdf")),
        "-o",
        s(&d.p("small.pdf")),
    ]);
    assert!(j.contains("\"objects_removed\""), "{j}");

    let t = ok(&["text", s(&d.p("doc.pdf")), "--pages", "2-3"]);
    assert_eq!(t.trim(), "Page 2\n\u{c}\nPage 3");
    ok(&[
        "text",
        s(&d.p("doc.pdf")),
        "--encoding",
        "utf16",
        "-o",
        s(&d.p("t.txt")),
    ]);
    assert_eq!(&std::fs::read(d.p("t.txt")).unwrap()[..2], &[0xFF, 0xFE]);
}

#[test]
fn cli_never_overwrites_without_force() {
    let d = Dir::new("force");
    samples::numbered(2).unwrap().save(d.p("doc.pdf")).unwrap();
    std::fs::write(d.p("taken.pdf"), b"keep me").unwrap();
    let (code, msg) = folio(&[
        "delete",
        s(&d.p("doc.pdf")),
        "--pages",
        "1",
        "-o",
        s(&d.p("taken.pdf")),
    ]);
    assert_eq!(code, 2);
    assert!(msg.contains("--force"), "{msg}");
    assert_eq!(std::fs::read(d.p("taken.pdf")).unwrap(), b"keep me");
    let (code, msg) = folio(&[
        "--json",
        "delete",
        s(&d.p("doc.pdf")),
        "--pages",
        "9",
        "-o",
        s(&d.p("y.pdf")),
    ]);
    assert_eq!(code, 2);
    assert!(
        msg.contains("\"ok\":false") && msg.contains("2 pages"),
        "{msg}"
    );
}
