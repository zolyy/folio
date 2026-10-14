//! End-to-end: `folio render`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;

#[test]
fn cli_render_writes_png_pages() {
    let dir = std::env::temp_dir().join(format!("folio-render-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pdf = dir.join("vec.pdf");
    folio_render::samples::vector().unwrap().save(&pdf).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_folio"))
        .args(["--json", "render"])
        .arg(&pdf)
        .args(["--dpi", "36"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("\"width\":306") && text.contains("\"height\":396"),
        "{text}"
    );
    for n in [1, 2] {
        let png = std::fs::read(dir.join(format!("vec_Page_{n}.png"))).unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }
    // Existing images are not overwritten without --force.
    let again = Command::new(env!("CARGO_BIN_EXE_folio"))
        .arg("render")
        .arg(&pdf)
        .output()
        .unwrap();
    assert_eq!(again.status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&dir);
}
