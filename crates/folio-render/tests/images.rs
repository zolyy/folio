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

/// A page with one JPEG image (16 × 16, one colour) filling 100 × 100 pt.
fn jpeg_page(data: Vec<u8>, cs: &str) -> folio_core::Document {
    let mut doc = folio_core::Document::new();
    let img = doc.add(common::image_stream(
        &[
            ("Width", common::int(16)),
            ("Height", common::int(16)),
            ("ColorSpace", common::name(cs)),
            ("BitsPerComponent", common::int(8)),
            ("Filter", common::name("DCTDecode")),
        ],
        data,
    ));
    let mut xo = folio_core::Dict::new();
    xo.set("I", img.into());
    let mut res = folio_core::Dict::new();
    res.set("XObject", xo.into());
    doc.add_page([100.0, 100.0], b"100 0 0 100 0 0 cm /I Do", res)
        .unwrap();
    doc
}

/// Regression (stage 3e): JPEGs with optimised Huffman tables, as Photoshop
/// and many cameras write them, came out washed-out pink instead of red with
/// the previous decoder (zune-jpeg 0.4). Checked against libjpeg via Poppler.
#[test]
fn jpeg_with_optimised_huffman_tables_keeps_its_colours() {
    use folio_render::export::{ColorMode, Format, JpegMode, Quality, Settings, encode};
    // The stage-3a page (306 × 396 at 36 dpi), saved as an optimised JPEG:
    // the image that came out pink with zune-jpeg 0.4.
    let vec_doc = folio_render::samples::vector().unwrap();
    let p = vec_doc.pages().unwrap()[0];
    let opts = Options {
        dpi: 36.0,
        ..Options::default()
    };
    let pm = render_page(&vec_doc, p, &opts).unwrap().pixmap;
    let s = Settings {
        format: Format::Jpeg(Quality::High, JpegMode::Optimized),
        color: ColorMode::Rgb,
        dpi: 36.0,
    };
    let jpeg = encode(&pm, &s).unwrap();
    let mut doc = folio_core::Document::new();
    let img = doc.add(common::image_stream(
        &[
            ("Width", common::int(306)),
            ("Height", common::int(396)),
            ("ColorSpace", common::name("DeviceRGB")),
            ("BitsPerComponent", common::int(8)),
            ("Filter", common::name("DCTDecode")),
        ],
        jpeg,
    ));
    let mut xo = folio_core::Dict::new();
    xo.set("I", img.into());
    let mut res = folio_core::Dict::new();
    res.set("XObject", xo.into());
    doc.add_page([306.0, 396.0], b"306 0 0 396 0 0 cm /I Do", res)
        .unwrap();
    let r = render(&doc);
    near(&r, 86, 335, [254, 0, 0], 6); // the red rectangle
}

/// CMYK JPEGs (Adobe APP14) must come out like Poppler/libjpeg draws them.
#[test]
fn cmyk_jpeg_matches_poppler() {
    if std::process::Command::new("pdftoppm")
        .arg("-v")
        .output()
        .is_err()
    {
        eprintln!("pdftoppm not installed; skipped");
        return;
    }
    let mut data = Vec::new();
    let enc = jpeg_encoder::Encoder::new(&mut data, 95);
    let px: Vec<u8> = (0..256).flat_map(|_| [0u8, 255, 255, 0]).collect();
    enc.encode(&px, 16, 16, jpeg_encoder::ColorType::Cmyk)
        .unwrap();
    let doc = jpeg_page(data, "DeviceCMYK");
    let dir = std::env::temp_dir().join(format!("folio-cmyk-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    doc.save(dir.join("c.pdf")).unwrap();
    let st = std::process::Command::new("pdftoppm")
        .args(["-r", "72", "-png", "-singlefile"])
        .arg(dir.join("c.pdf"))
        .arg(dir.join("p"))
        .status()
        .unwrap();
    assert!(st.success());
    let theirs = tiny_skia::Pixmap::load_png(dir.join("p.png")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let t = theirs.pixel(50, 50).unwrap().demultiply();
    let ours = render(&doc);
    let [r, g, b, _] = ours.rgba(50, 50).unwrap();
    // Poppler converts CMYK through its own approximation; the hue must agree.
    let hue = |r: u8, g: u8, b: u8| (r > 150, g > 150, b > 150);
    assert_eq!(
        hue(r, g, b),
        hue(t.red(), t.green(), t.blue()),
        "ours ({r},{g},{b}) vs Poppler ({},{},{})",
        t.red(),
        t.green(),
        t.blue()
    );
}
