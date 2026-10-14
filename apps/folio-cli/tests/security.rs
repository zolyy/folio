//! End-to-end: protect, open, restrict and unprotect through `folio`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use folio_ops::samples;

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let d = std::env::temp_dir().join(format!("folio-sec-{name}-{}", std::process::id()));
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

fn folio(args: &[&str], password: Option<&str>) -> (i32, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_folio"));
    c.args(args);
    c.env_remove("FOLIO_PASSWORD");
    if let Some(p) = password {
        c.env("FOLIO_PASSWORD", p);
    }
    let out = c.output().unwrap();
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), s)
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn cli_encrypt_open_and_decrypt_with_passwords() {
    let d = Dir::new("roundtrip");
    samples::numbered(3).unwrap().save(d.p("doc.pdf")).unwrap();
    let (code, out) = folio(
        &[
            "--json",
            "encrypt",
            s(&d.p("doc.pdf")),
            "--open-password",
            "open-1",
            "--permissions-password",
            "perm-2",
            "-o",
            s(&d.p("lock.pdf")),
        ],
        None,
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("256-bit AES"), "{out}");

    // No password: exit code 3, a clear message.
    let (code, out) = folio(&["info", s(&d.p("lock.pdf"))], None);
    assert_eq!(code, 3, "{out}");
    assert!(out.contains("needs a password"), "{out}");
    let (code, out) = folio(&["--json", "info", s(&d.p("lock.pdf"))], Some("wrong"));
    assert_eq!(code, 3);
    assert!(out.contains("\"kind\":\"security\""), "{out}");

    let (code, out) = folio(&["text", s(&d.p("lock.pdf"))], Some("open-1"));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Page 2"));

    // Saving a protected file keeps it protected.
    let (code, _) = folio(
        &["rotate", s(&d.p("lock.pdf")), "-o", s(&d.p("rot.pdf"))],
        Some("open-1"),
    );
    assert_eq!(code, 0);
    assert_eq!(folio(&["info", s(&d.p("rot.pdf"))], None).0, 3);

    let (code, out) = folio(
        &["decrypt", s(&d.p("lock.pdf")), "-o", s(&d.p("open.pdf"))],
        Some("perm-2"),
    );
    assert_eq!(code, 0, "{out}");
    let (code, out) = folio(&["security", s(&d.p("open.pdf"))], None);
    assert_eq!(code, 0);
    assert!(out.contains("No Security"), "{out}");
}

#[test]
fn cli_restrictions_are_enforced_per_command() {
    let d = Dir::new("restrict");
    samples::numbered(3).unwrap().save(d.p("doc.pdf")).unwrap();
    let (code, out) = folio(
        &[
            "encrypt",
            s(&d.p("doc.pdf")),
            "--compat",
            "acrobat7",
            "--permissions-password",
            "boss",
            "--printing",
            "low",
            "--changes",
            "pages",
            "--no-copy",
            "-o",
            s(&d.p("r.pdf")),
        ],
        None,
    );
    assert_eq!(code, 0, "{out}");

    let (code, out) = folio(&["security", s(&d.p("r.pdf"))], None);
    assert_eq!(code, 0);
    for row in [
        "Encryption Level:                   128-bit AES",
        "Document Open Password:             No",
        "Permissions Password:               Yes",
        "Printing:                           Low Resolution",
        "Document Assembly:                  Allowed",
        "Changing the Document:              Not Allowed",
        "Content Copying:                    Not Allowed",
        "Page Extraction:                    Not Allowed",
    ] {
        assert!(out.contains(row), "missing `{row}` in:\n{out}");
    }

    // Allowed: assembly. Refused: copying text, extracting, changing, removing security.
    assert_eq!(
        folio(&["rotate", s(&d.p("r.pdf")), "-o", s(&d.p("a.pdf"))], None).0,
        0
    );
    for args in [
        vec!["text", s(&d.p("r.pdf"))],
        vec!["extract", s(&d.p("r.pdf")), "--pages", "1", "-o", "x.pdf"],
        vec!["meta", s(&d.p("r.pdf")), "--title", "T", "-o", "y.pdf"],
        vec!["decrypt", s(&d.p("r.pdf")), "-o", "z.pdf"],
    ] {
        let (code, out) = folio(&args, None);
        assert_eq!(code, 3, "{args:?} should be refused: {out}");
        assert!(out.contains("permissions password"), "{out}");
    }
    // With the permissions password everything works.
    assert_eq!(folio(&["text", s(&d.p("r.pdf"))], Some("boss")).0, 0);
}

#[test]
fn cli_encrypt_follows_acrobat_rules() {
    let d = Dir::new("rules");
    samples::numbered(1).unwrap().save(d.p("doc.pdf")).unwrap();
    let doc_path = d.p("doc.pdf");
    let out_path = d.p("o.pdf");
    let o = s(&out_path);
    let cases: [(&[&str], &str); 3] = [
        (
            &["--open-password", "same", "--permissions-password", "same"],
            "must be different",
        ),
        (
            &["--open-password", "x", "--no-copy"],
            "permissions password",
        ),
        (
            &[
                "--compat",
                "acrobat5",
                "--encrypt",
                "except-metadata",
                "--open-password",
                "x",
            ],
            "Acrobat 6.0",
        ),
    ];
    for (extra, msg) in cases {
        let mut args = vec!["encrypt", s(&doc_path), "-o", o];
        args.extend_from_slice(extra);
        let (code, out) = folio(&args, None);
        assert_eq!(code, 2, "{extra:?}: {out}");
        assert!(out.contains(msg), "{extra:?}: {out}");
    }
}
