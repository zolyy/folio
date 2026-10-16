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

#[test]
fn cli_export_jpeg_tiff_and_png_with_acrobat_settings() {
    let dir = std::env::temp_dir().join(format!("folio-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pdf = dir.join("vec.pdf");
    folio_render::samples::vector().unwrap().save(&pdf).unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_folio"))
            .arg("render")
            .arg(&pdf)
            .args(["--pages", "1", "--dpi", "36", "--force"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&[
        "--format",
        "jpeg",
        "--quality",
        "maximum",
        "--jpeg-mode",
        "progressive",
        "--scans",
        "5",
    ]);
    assert_eq!(
        &std::fs::read(dir.join("vec_Page_1.jpg")).unwrap()[..2],
        &[0xFF, 0xD8]
    );
    run(&[
        "--format",
        "tiff",
        "--tiff-compression",
        "zip",
        "--colorspace",
        "cmyk",
    ]);
    assert_eq!(
        &std::fs::read(dir.join("vec_Page_1.tif")).unwrap()[..4],
        b"II*\0"
    );
    run(&[
        "--format",
        "tiff",
        "--tiff-compression",
        "g4",
        "--colorspace",
        "mono",
    ]);
    run(&[
        "--format",
        "png",
        "--png-filter",
        "paeth",
        "--colorspace",
        "gray",
    ]);
    // Refusals with a clear message.
    let bad = Command::new(env!("CARGO_BIN_EXE_folio"))
        .arg("render")
        .arg(&pdf)
        .args(["--format", "png", "--colorspace", "cmyk", "--force"])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("CMYK"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_crop_removes_white_margins() {
    let dir = std::env::temp_dir().join(format!("folio-margins-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut doc = folio_core::Document::new();
    doc.add_page(
        [200.0, 300.0],
        b"0 g 40 50 100 120 re f",
        folio_core::Dict::new(),
    )
    .unwrap();
    doc.add_page([200.0, 300.0], b"", folio_core::Dict::new())
        .unwrap();
    let src = dir.join("m.pdf");
    doc.save(&src).unwrap();
    let out = dir.join("c.pdf");
    let r = Command::new(env!("CARGO_BIN_EXE_folio"))
        .arg("crop")
        .arg(&src)
        .arg("--remove-white-margins")
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let d = folio_core::Document::load(&out).unwrap();
    let pages = d.pages().unwrap();
    let cb = d.page_attr(pages[0], b"CropBox").unwrap();
    let v: Vec<f64> = cb
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect();
    for (got, want) in v.iter().zip([40.0, 50.0, 140.0, 170.0]) {
        assert!((got - want).abs() <= 1.0, "crop box {v:?}");
    }
    // A blank page keeps its size.
    assert!(d.page_attr(pages[1], b"CropBox").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
