//! Stage 3c: text with every font kind, render modes, and system fonts.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use folio_core::Document;
use folio_render::{Options, Rendered, render_page, samples};

fn render(doc: &Document, system_fonts: bool) -> Rendered {
    let p = doc.pages().unwrap()[0];
    render_page(
        doc,
        p,
        &Options {
            system_fonts,
            ..Options::default()
        },
    )
    .unwrap()
}

fn near(r: &Rendered, x: u32, y: u32, want: [u8; 3], tol: u8) {
    let [rr, gg, bb, _] = r.rgba(x, y).unwrap();
    let ok = [rr, gg, bb]
        .iter()
        .zip(want.iter())
        .all(|(a, b)| a.abs_diff(*b) <= tol);
    assert!(ok, "pixel ({x},{y}) is {:?}, wanted {want:?}", [rr, gg, bb]);
}

const BLACK: [u8; 3] = [0, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];

#[test]
fn truetype_glyphs_and_advance_widths() {
    let r = render(&samples::text().unwrap(), false);
    near(&r, 85, 107, BLACK, 0); // "A": square
    near(&r, 155, 122, BLACK, 0); // "B": triangle, after a 700-unit advance
    near(&r, 125, 107, WHITE, 0); // gap between the glyphs
}

#[test]
fn bare_cff_type1_and_type3_fonts() {
    let r = render(&samples::text().unwrap(), false);
    near(&r, 85, 257, BLACK, 0); // FontFile3 /Type1C
    near(&r, 85, 407, BLACK, 0); // FontFile (eexec Type 1)
    near(&r, 85, 557, BLACK, 0); // Type 3 glyph procedure
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
}

#[test]
fn composite_identity_h_font() {
    let r = render(&samples::text().unwrap(), false);
    near(&r, 335, 407, BLACK, 0);
}

#[test]
fn text_rendering_modes_stroke_and_clip() {
    let r = render(&samples::text().unwrap(), false);
    near(&r, 335, 107, WHITE, 0); // mode 1: outline only
    let [edge, ..] = r.rgba(310, 107).unwrap();
    assert!(edge < 128, "stroked edge is {edge}");
    near(&r, 335, 257, [255, 0, 0], 0); // mode 7: clip, then a red fill
    near(&r, 400, 257, WHITE, 0);
}

#[test]
fn missing_fonts_are_reported_without_system_fonts() {
    let mut doc = Document::new();
    doc.add_text_page("Helvetica, not embedded").unwrap();
    let r = render(&doc, false);
    assert!(
        r.skipped.iter().any(|(k, _)| k.contains("not embedded")),
        "{:?}",
        r.skipped
    );
}

/// With an installed metric-compatible font (Arial, Liberation Sans...),
/// non-embedded Helvetica is drawn. Skipped where no such font exists.
#[test]
fn standard_14_fonts_use_installed_substitutes() {
    if folio_render::font::system::substitute("Helvetica", 0).is_none() {
        eprintln!("no Helvetica substitute installed; skipped");
        return;
    }
    let mut doc = Document::new();
    doc.add_text_page("HHHHHHHHHHHHHHHH").unwrap();
    let r = render(&doc, true);
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
    // Text at (72, 720), 12 pt: some dark pixels along the baseline row.
    let dark = (72..180)
        .filter(|&x| r.rgba(x, 792 - 724).unwrap()[0] < 200)
        .count();
    assert!(dark >= 8, "only {dark} dark pixels where text should be");
}

#[test]
fn text_matches_poppler() {
    if std::process::Command::new("pdftoppm")
        .arg("-v")
        .output()
        .is_err()
    {
        eprintln!("pdftoppm not installed; oracle comparison skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("folio-text-oracle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let doc = samples::text().unwrap();
    let pdf = dir.join("text.pdf");
    doc.save(&pdf).unwrap();
    let st = std::process::Command::new("pdftoppm")
        .args(["-r", "72", "-png", "-singlefile"])
        .arg(&pdf)
        .arg(dir.join("p"))
        .status()
        .unwrap();
    assert!(st.success());
    let ours = render(&doc, false);
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
    // Name the glyphs that differ, so a failure says which font kind it is.
    let width = theirs.width() as usize;
    let regions = [
        ("TrueType A", 50.0, 650.0),
        ("TrueType B", 120.0, 650.0),
        ("CFF", 50.0, 500.0),
        ("Type 1", 50.0, 350.0),
        ("Type 3", 50.0, 200.0),
        ("stroke mode", 300.0, 650.0),
        ("clip mode", 300.0, 500.0),
        ("Type 0", 300.0, 350.0),
    ];
    let mut report = Vec::new();
    for (name, ox, oy) in regions {
        let (x0, x1) = (ox as usize, ox as usize + 80);
        let (y0, y1) = ((792.0 - oy - 75.0) as usize, (792.0 - oy + 5.0) as usize);
        let n = (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| y * width + x))
            .filter(|&i| {
                let (Some(a), Some(b)) = (ours.pixmap.pixels().get(i), theirs.pixels().get(i))
                else {
                    return false;
                };
                let (a, b) = (a.demultiply(), b.demultiply());
                a.red().abs_diff(b.red()) > 60 || a.green().abs_diff(b.green()) > 60
            })
            .count();
        if n > 50 {
            report.push(format!("{name}: {n} pixels"));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        share < 0.002,
        "{differing} of {total} pixels differ from Poppler ({:.3}%): {}",
        share * 100.0,
        report.join(", ")
    );
}
