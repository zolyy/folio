//! End-to-end: `folio find`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;

#[test]
fn cli_find_reports_pages_context_and_rectangles() {
    let dir = std::env::temp_dir().join(format!("folio-find-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pdf = dir.join("five.pdf");
    folio_ops::samples::numbered(5).unwrap().save(&pdf).unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_folio"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let p = pdf.to_str().unwrap();
    let text = run(&["find", p, "page 3"]);
    assert!(text.contains("page 3: …[Page 3]…"), "{text}");
    assert!(text.trim_end().ends_with("1 match"), "{text}");
    let json = run(&["--json", "find", p, "PAGE", "--case-sensitive"]);
    assert!(json.contains("\"matches\":0"), "{json}");
    let json = run(&["--json", "find", p, "page", "--pages", "2-3"]);
    assert!(json.contains("\"matches\":2"), "{json}");
    assert!(json.contains("\"rects\":[["), "{json}");
    let _ = std::fs::remove_dir_all(&dir);
}
