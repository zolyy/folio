//! Stage 3d: shadings, patterns, soft masks, transparency groups and
//! annotation appearances, checked by pixel and against Poppler.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use folio_render::{Options, Rendered, render_page, samples};

fn render() -> Rendered {
    let doc = samples::shading().unwrap();
    let p = doc.pages().unwrap()[0];
    render_page(&doc, p, &Options::default()).unwrap()
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
fn axial_shading_with_extend() {
    let r = render();
    near(&r, 150, 142, [128, 0, 127], 4); // halfway red → blue
    near(&r, 52, 142, [252, 0, 3], 4);
    near(&r, 248, 142, [3, 0, 252], 4);
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
}

#[test]
fn radial_shading_from_a_point() {
    let r = render();
    near(&r, 350, 142, WHITE, 6); // centre: s = 0
    let [edge, ..] = r.rgba(395, 142).unwrap();
    assert!(edge < 40, "near the rim should be dark, was {edge}");
}

#[test]
fn function_based_shading_with_matrix() {
    let r = render();
    near(&r, 460, 142, [26, 128, 0], 4); // f(0.1, 0.5) = (0.1, 0.5, 0)
    near(&r, 540, 142, [230, 128, 0], 4);
}

#[test]
fn gouraud_triangle_and_coons_patch_meshes() {
    let r = render();
    near(&r, 100, 359, [85, 85, 85], 8); // centroid of R, G, B corners
    near(&r, 55, 302, WHITE, 0); // outside the triangle
    near(&r, 250, 342, [128, 128, 128], 10); // patch centre: corner average
}

#[test]
fn tiling_patterns_coloured_and_uncoloured() {
    let r = render();
    near(&r, 365, 387, [255, 0, 0], 0);
    near(&r, 375, 387, WHITE, 0);
    near(&r, 365, 377, WHITE, 0);
    near(&r, 485, 387, [0, 0, 255], 0); // uncoloured: colour from scn
    // Exact geometry: the cell square at page y 400..410 covers device rows 382..=391.
    near(&r, 365, 382, [255, 0, 0], 0);
    near(&r, 365, 391, [255, 0, 0], 0);
    near(&r, 365, 381, WHITE, 0);
}

#[test]
fn shading_pattern_fill() {
    let r = render();
    near(&r, 52, 542, [252, 0, 3], 4);
    near(&r, 248, 542, [3, 0, 252], 4);
}

#[test]
fn luminosity_soft_mask() {
    let r = render();
    near(&r, 350, 542, [0, 0, 255], 0);
    near(&r, 450, 542, WHITE, 0);
}

#[test]
fn transparency_group_alpha_applies_to_the_whole_group() {
    let r = render();
    near(&r, 525, 542, [255, 128, 128], 2);
    near(&r, 550, 542, [255, 128, 128], 2); // overlap: not darker
}

#[test]
fn annotation_appearances_and_hidden_flag() {
    let r = render();
    near(&r, 150, 692, [0, 255, 0], 0);
    near(&r, 350, 692, WHITE, 0);
    let doc = samples::shading().unwrap();
    let p = doc.pages().unwrap()[0];
    let off = render_page(
        &doc,
        p,
        &Options {
            annotations: false,
            ..Options::default()
        },
    )
    .unwrap();
    near(&off, 150, 692, WHITE, 0);
}

#[test]
fn shading_page_matches_poppler() {
    if std::process::Command::new("pdftoppm")
        .arg("-v")
        .output()
        .is_err()
    {
        eprintln!("pdftoppm not installed; oracle comparison skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("folio-shade-oracle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let doc = samples::shading().unwrap();
    let pdf = dir.join("shading.pdf");
    doc.save(&pdf).unwrap();
    let st = std::process::Command::new("pdftoppm")
        .args(["-r", "72", "-png", "-singlefile"])
        .arg(&pdf)
        .arg(dir.join("p"))
        .status()
        .unwrap();
    assert!(st.success());
    let ours = render();
    let theirs = tiny_skia::Pixmap::load_png(dir.join("p.png")).unwrap();
    let w = theirs.width() as usize;
    // Regions (device pixels): name, x0, y0, x1, y1, vertical offset of
    // Poppler's image. Poppler places tiling cells one device row lower than
    // their exact position (checked in tiling_patterns_coloured_and_uncoloured),
    // so those regions are compared one row apart.
    let regions = [
        ("axial", 50, 92, 250, 192, 0),
        ("radial", 300, 92, 400, 192, 0),
        ("function", 450, 92, 550, 192, 0),
        ("triangle", 50, 292, 150, 392, 0),
        ("coons", 200, 292, 300, 392, 0),
        ("tiling", 350, 292, 450, 391, 1),
        ("uncoloured tiling", 470, 292, 570, 391, 1),
        ("shading pattern", 50, 492, 250, 592, 0),
        ("soft mask", 300, 492, 500, 592, 0),
        ("group", 520, 492, 580, 592, 0),
        ("annotations", 100, 642, 400, 742, 0),
    ];
    let _ = std::fs::remove_dir_all(&dir);
    let mut bad = Vec::new();
    for (name, x0, y0, x1, y1, dy) in regions {
        let mut differ = 0usize;
        let mut total = 0usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let i = y * w + x;
                let (a, b) = (
                    ours.pixmap.pixels()[i].demultiply(),
                    theirs.pixels()[i + dy * w].demultiply(),
                );
                total += 1;
                if a.red().abs_diff(b.red()) > 40
                    || a.green().abs_diff(b.green()) > 40
                    || a.blue().abs_diff(b.blue()) > 40
                {
                    differ += 1;
                }
            }
        }
        // Edges, mesh seams and gradient steps may differ; areas must not.
        if differ * 100 > total * 3 {
            bad.push(format!("{name}: {differ}/{total}"));
        }
    }
    assert!(
        bad.is_empty(),
        "regions that differ from Poppler: {}",
        bad.join(", ")
    );
}
