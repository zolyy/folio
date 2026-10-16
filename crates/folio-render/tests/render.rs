//! Stage 3a: vector graphics, checked pixel by pixel and against Poppler.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use folio_core::Document;
use folio_render::{Options, Rendered, render_page, samples};

fn render(doc: &Document, page: usize, dpi: f64) -> Rendered {
    let p = doc.pages().unwrap()[page];
    render_page(
        doc,
        p,
        &Options {
            dpi,
            ..Options::default()
        },
    )
    .unwrap()
}

/// Colour at a pixel, within `tol` per channel.
fn near(r: &Rendered, x: u32, y: u32, want: [u8; 3], tol: u8) {
    let [rr, gg, bb, _] = r.rgba(x, y).unwrap();
    let ok = [rr, gg, bb]
        .iter()
        .zip(want.iter())
        .all(|(a, b)| a.abs_diff(*b) <= tol);
    assert!(ok, "pixel ({x},{y}) is {:?}, wanted {want:?}", [rr, gg, bb]);
}

const WHITE: [u8; 3] = [255, 255, 255];

#[test]
fn fills_strokes_and_even_odd() {
    let doc = samples::vector().unwrap();
    let r = render(&doc, 0, 72.0);
    assert_eq!((r.pixmap.width(), r.pixmap.height()), (612, 792));
    near(&r, 172, 670, [255, 0, 0], 0); // rg + re + f
    near(&r, 400, 92, [0, 0, 255], 0); // RG + w + S
    near(&r, 320, 392, [0, 0, 0], 0); // f* ring
    near(&r, 400, 392, WHITE, 0); // f* hole
}

#[test]
fn clipping_limits_painting() {
    let doc = samples::vector().unwrap();
    let r = render(&doc, 0, 72.0);
    near(&r, 122, 342, [0, 255, 0], 0);
    near(&r, 200, 342, WHITE, 0);
}

#[test]
fn colour_spaces_cmyk_separation_devicen() {
    let doc = samples::vector().unwrap();
    let r = render(&doc, 0, 72.0);
    near(&r, 97, 167, [255, 0, 0], 0); // 0 1 1 0 k
    near(&r, 425, 167, [0, 128, 0], 1); // Separation, type-2 function
    near(&r, 495, 167, [255, 128, 0], 1); // DeviceN, PostScript function
}

#[test]
fn constant_alpha_from_extgstate() {
    let doc = samples::vector().unwrap();
    let r = render(&doc, 0, 72.0);
    near(&r, 525, 695, [128, 128, 255], 2);
}

#[test]
fn form_xobjects_use_matrix_and_bbox() {
    let doc = samples::vector().unwrap();
    let r = render(&doc, 0, 72.0);
    near(&r, 540, 272, [255, 0, 255], 0);
    near(&r, 565, 272, WHITE, 0); // outside the form's BBox
}

#[test]
fn dashed_lines() {
    let doc = samples::vector().unwrap();
    let r = render(&doc, 0, 72.0);
    let [on, ..] = r.rgba(75, 542).unwrap();
    let [off, ..] = r.rgba(81, 542).unwrap();
    assert!(on < 64 && off > 200, "dash on={on} off={off}");
}

#[test]
fn page_rotation_and_resolution() {
    let doc = samples::vector().unwrap();
    let r = render(&doc, 1, 72.0);
    assert_eq!((r.pixmap.width(), r.pixmap.height()), (100, 200));
    near(&r, 50, 25, [255, 0, 0], 0);
    near(&r, 50, 150, WHITE, 0);
    let r = render(&doc, 0, 144.0);
    assert_eq!((r.pixmap.width(), r.pixmap.height()), (1224, 1584));
    near(&r, 344, 1340, [255, 0, 0], 0);
}

#[test]
fn unsupported_content_is_reported_not_hidden() {
    // Content Folio cannot draw is counted, never silently dropped.
    let mut bad = folio_core::Dict::new();
    bad.set("ShadingType", folio_core::Object::Int(99));
    bad.set("ColorSpace", folio_core::Object::name("DeviceRGB"));
    let mut sh = folio_core::Dict::new();
    sh.set("Sh0", bad.into());
    let mut res = folio_core::Dict::new();
    res.set("Shading", sh.into());
    let mut doc = Document::new();
    doc.add_page([100.0, 100.0], b"/Sh0 sh", res).unwrap();
    let r = render(&doc, 0, 36.0);
    assert!(
        r.skipped
            .iter()
            .any(|(k, n)| *k == "undecodable shading" && *n >= 1),
        "{:?}",
        r.skipped
    );
}

#[test]
fn absurd_sizes_are_refused_cleanly() {
    let doc = samples::vector().unwrap();
    let p = doc.pages().unwrap()[0];
    for dpi in [0.0, -5.0, f64::NAN, 100_000.0] {
        assert!(
            render_page(
                &doc,
                p,
                &Options {
                    dpi,
                    ..Options::default()
                }
            )
            .is_err()
        );
    }
    let mut big = Document::new();
    big.add_page([14_400.0, 14_400.0], b"", folio_core::Dict::new())
        .unwrap();
    let p = big.pages().unwrap()[0];
    let e = render_page(
        &big,
        p,
        &Options {
            dpi: 600.0,
            ..Options::default()
        },
    )
    .unwrap_err();
    assert!(e.to_string().contains("lower the resolution"), "{e}");
}

/// Poppler's pdftoppm as an external oracle: the two renderings must agree
/// except for anti-aliasing along edges.
#[test]
fn matches_poppler_rendering() {
    if std::process::Command::new("pdftoppm")
        .arg("-v")
        .output()
        .is_err()
    {
        eprintln!("pdftoppm not installed; oracle comparison skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("folio-oracle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let doc = samples::vector().unwrap();
    let pdf = dir.join("vector.pdf");
    doc.save(&pdf).unwrap();
    let st = std::process::Command::new("pdftoppm")
        .args(["-r", "72", "-png"])
        .arg(&pdf)
        .arg(dir.join("p"))
        .status()
        .unwrap();
    assert!(st.success());
    for page in 0..2 {
        let ours = render(&doc, page, 72.0);
        let theirs = tiny_skia::Pixmap::load_png(dir.join(format!("p-{}.png", page + 1))).unwrap();
        assert_eq!(
            (ours.pixmap.width(), ours.pixmap.height()),
            (theirs.width(), theirs.height()),
            "page {page} size"
        );
        let total = theirs.pixels().len();
        let differing = ours
            .pixmap
            .pixels()
            .iter()
            .zip(theirs.pixels())
            .filter(|(a, b)| {
                let (a, b) = (a.demultiply(), b.demultiply());
                a.red().abs_diff(b.red()) > 60
                    || a.green().abs_diff(b.green()) > 60
                    || a.blue().abs_diff(b.blue()) > 60
            })
            .count();
        let share = differing as f64 / total as f64;
        assert!(
            share < 0.005,
            "page {page}: {differing} of {total} pixels differ from Poppler ({:.3}%)",
            share * 100.0
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
