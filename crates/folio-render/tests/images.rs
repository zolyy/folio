//! Stage 3b: images, checked pixel by pixel and against Poppler.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use folio_render::{Options, Rendered, render_page};

fn render(doc: &folio_core::Document) -> Rendered {
    let p = doc.pages().unwrap()[0];
    render_page(doc, p, &Options::default()).unwrap()
}

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
fn raw_rgb_image_rows_start_at_the_top() {
    let r = render(&common::image_sample());
    near(&r, 75, 117, [255, 0, 0], 0);
    near(&r, 125, 117, [0, 255, 0], 0);
    near(&r, 75, 167, [0, 0, 255], 0);
    near(&r, 125, 167, WHITE, 0);
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
}

#[test]
fn bit_depths_decode_arrays_and_indexed_colour() {
    let r = render(&common::image_sample());
    near(&r, 205, 187, [0, 0, 0], 0); // 1-bit, Decode [1 0]: bit 1 is black
    near(&r, 215, 187, WHITE, 0);
    near(&r, 325, 167, [255, 0, 0], 0); // 4-bit indexed
    near(&r, 375, 167, [0, 0, 255], 0);
    near(&r, 445, 167, [128, 128, 128], 1); // 16-bit gray
}

#[test]
fn stencil_masks_paint_the_fill_colour() {
    let r = render(&common::image_sample());
    near(&r, 75, 317, [0, 255, 0], 0);
    near(&r, 125, 317, WHITE, 0);
    near(&r, 75, 367, WHITE, 0);
    near(&r, 125, 367, [0, 255, 0], 0);
}

#[test]
fn jpeg_colour_and_gray() {
    let r = render(&common::image_sample());
    near(&r, 250, 342, [200, 50, 50], 4);
    near(&r, 345, 367, [100, 100, 100], 3);
}

#[test]
fn soft_masks_and_colour_key_masks() {
    let r = render(&common::image_sample());
    near(&r, 425, 317, [127, 127, 255], 2);
    near(&r, 495, 317, [255, 0, 0], 0);
    near(&r, 545, 317, WHITE, 0); // the green pixel is keyed out
}

#[test]
fn ccitt_group4_fax_images() {
    let r = render(&common::image_sample());
    near(&r, 60, 477, [0, 0, 0], 0);
    near(&r, 180, 477, WHITE, 0);
    near(&r, 60, 487, WHITE, 0);
}

#[test]
fn inline_images_raw_and_filtered() {
    let r = render(&common::image_sample());
    near(&r, 325, 467, [255, 0, 255], 0);
    near(&r, 375, 467, [0, 255, 255], 0);
    near(&r, 445, 467, [128, 128, 128], 0);
}

#[test]
fn broken_images_are_reported_not_fatal() {
    let mut doc = folio_core::Document::new();
    let bad = doc.add(common::image_stream(
        &[
            ("Width", common::int(4)),
            ("Height", common::int(4)),
            ("ColorSpace", common::name("DeviceRGB")),
            ("BitsPerComponent", common::int(8)),
            ("Filter", common::name("DCTDecode")),
        ],
        b"not a jpeg".to_vec(),
    ));
    let mut xo = folio_core::Dict::new();
    xo.set("X", bad.into());
    let mut res = folio_core::Dict::new();
    res.set("XObject", xo.into());
    doc.add_page([100.0, 100.0], b"q 50 0 0 50 0 0 cm /X Do Q", res)
        .unwrap();
    let r = render(&doc);
    assert!(r.skipped.iter().any(|(k, _)| *k == "undecodable image"));
}

#[test]
fn images_match_poppler() {
    if std::process::Command::new("pdftoppm")
        .arg("-v")
        .output()
        .is_err()
    {
        eprintln!("pdftoppm not installed; oracle comparison skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("folio-img-oracle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let doc = common::image_sample();
    let pdf = dir.join("images.pdf");
    doc.save(&pdf).unwrap();
    let st = std::process::Command::new("pdftoppm")
        .args(["-r", "72", "-png", "-singlefile"])
        .arg(&pdf)
        .arg(dir.join("p"))
        .status()
        .unwrap();
    assert!(st.success());
    let ours = render(&doc);
    let theirs = tiny_skia::Pixmap::load_png(dir.join("p.png")).unwrap();
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
        share < 0.01,
        "{differing} of {total} pixels differ from Poppler ({:.3}%)",
        share * 100.0
    );
    let _ = std::fs::remove_dir_all(&dir);
}
