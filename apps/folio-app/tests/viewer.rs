//! The reading window's commands, without a screen: zoom, rotate view,
//! navigation, page labels, bookmarks and the background renderer.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use folio_app::cache::{Done, Key, Renderer, rotate};
use folio_app::viewer::{Layout, MAX_ZOOM, MIN_ZOOM, PX_PER_PT, Tab, Zoom};
use folio_core::Document;
use folio_ops::samples;

fn tab(doc: Document) -> Tab {
    Tab::new(doc, None, "test.pdf".into(), 1)
}

/// A one-page document of `w` × `h` points.
fn sized(w: f64, h: f64) -> Document {
    let mut doc = Document::new();
    doc.add_page([w, h], b"", folio_core::Dict::new()).unwrap();
    doc
}

const VP: [f32; 2] = [1000.0, 800.0];

#[test]
fn zoom_steps_follow_acrobats_menu() {
    let mut t = tab(sized(612.0, 792.0));
    t.set_zoom(Zoom::Percent(100.0));
    t.zoom_in(VP);
    assert_eq!(t.zoom, Zoom::Percent(125.0));
    t.zoom_out(VP);
    t.zoom_out(VP);
    assert_eq!(t.zoom, Zoom::Percent(75.0));
    t.set_zoom(Zoom::Percent(8.33));
    t.zoom_out(VP);
    assert_eq!(t.zoom, Zoom::Percent(MIN_ZOOM));
    t.set_zoom(Zoom::Percent(6400.0));
    t.zoom_in(VP);
    assert_eq!(t.zoom, Zoom::Percent(MAX_ZOOM));
    // From a fit mode, Zoom In goes to the next preset above the fitted size.
    t.set_zoom(Zoom::FitPage);
    let fitted = t.percent(VP);
    t.zoom_in(VP);
    let Zoom::Percent(p) = t.zoom else {
        panic!("zoom in should give a fixed magnification")
    };
    assert!(p > fitted, "{p} should be above {fitted}");
}

#[test]
fn typed_zoom_is_clamped_to_acrobats_range() {
    let mut t = tab(sized(612.0, 792.0));
    assert!(t.set_zoom_text("150%"));
    assert_eq!(t.zoom, Zoom::Percent(150.0));
    assert!(t.set_zoom_text(" 66,67 "));
    assert_eq!(t.zoom, Zoom::Percent(66.67));
    assert!(t.set_zoom_text("99999"));
    assert_eq!(t.zoom, Zoom::Percent(MAX_ZOOM));
    assert!(t.set_zoom_text("0.2"));
    assert_eq!(t.zoom, Zoom::Percent(MIN_ZOOM));
    assert!(!t.set_zoom_text("big"));
    assert!(!t.set_zoom_text("NaN"));
    assert_eq!(t.zoom, Zoom::Percent(MIN_ZOOM));
}

#[test]
fn hundred_percent_is_printed_size_and_fit_modes_fit() {
    let mut t = tab(sized(612.0, 792.0));
    t.set_zoom(Zoom::Percent(100.0));
    assert!((t.scale(VP) - PX_PER_PT).abs() < 1e-6);
    t.set_zoom(Zoom::FitWidth);
    let [w, _] = t.display_size(0, t.scale(VP));
    assert!((w - (VP[0] - 16.0)).abs() < 0.01, "fit width gave {w}");
    t.set_zoom(Zoom::FitPage);
    let [w, h] = t.display_size(0, t.scale(VP));
    assert!(
        (h - (VP[1] - 16.0)).abs() < 0.01 && w < VP[0],
        "fit page gave {w} x {h}"
    );
    // A degenerate window never yields a zero or infinite scale.
    for vp in [[0.0, 0.0], [-5.0, 1e9], [f32::MAX, f32::MAX]] {
        let s = t.scale(vp);
        assert!(s.is_finite() && s > 0.0, "{vp:?} gave {s}");
    }
}

#[test]
fn rotate_view_swaps_the_page_and_comes_back() {
    let mut t = tab(sized(612.0, 792.0));
    assert_eq!(t.display_size(0, 1.0), [612.0, 792.0]);
    t.rotate_clockwise();
    assert_eq!(t.rotation, 90);
    assert_eq!(t.display_size(0, 1.0), [792.0, 612.0]);
    for _ in 0..3 {
        t.rotate_clockwise();
    }
    assert_eq!(t.rotation, 0);
    t.rotate_counterclockwise();
    assert_eq!(t.rotation, 270);
    // Rotate View is not saved: the document is untouched.
    assert!(t.doc.page_attr(t.pages[0], b"Rotate").is_none());
}

#[test]
fn page_rotate_entry_is_applied_to_the_size() {
    let mut doc = sized(612.0, 792.0);
    let p = doc.pages().unwrap()[0];
    doc.get_mut(p)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Rotate", folio_core::Object::Int(-90));
    let t = tab(doc);
    assert_eq!(t.sizes[0], [792.0, 612.0]);
}

#[test]
fn navigation_stays_inside_the_document() {
    let mut t = tab(samples::numbered(5).unwrap());
    t.previous_page();
    assert_eq!(t.current, 0);
    t.last_page();
    assert_eq!(t.current, 4);
    t.next_page();
    assert_eq!(t.current, 4);
    t.go_to(usize::MAX);
    assert_eq!(t.current, 4);
    t.first_page();
    assert_eq!(t.current, 0);
    assert!(t.go_to_text("3"));
    assert_eq!(t.current, 2);
    for bad in ["0", "6", "-1", "", "  ", "three"] {
        assert!(!t.go_to_text(bad), "{bad:?}");
        assert_eq!(t.current, 2);
    }
    assert_eq!(t.page_name(2), "3");
}

#[test]
fn page_box_uses_page_labels_like_acrobat() {
    let mut t = tab(samples::rich().unwrap());
    assert_eq!(t.page_name(0), "i");
    assert_eq!(t.page_name(2), "A-1");
    assert!(t.go_to_text("ii"));
    assert_eq!(t.current, 1);
    assert!(t.go_to_text("A-1"));
    assert_eq!(t.current, 2);
    // Plain numbers still work when they are not someone's label.
    assert!(t.go_to_text("1"));
    assert_eq!(t.current, 0);
}

#[test]
fn bookmark_tree_resolves_explicit_and_named_destinations() {
    let t = tab(samples::rich().unwrap());
    let titles: Vec<&str> = t.outline.iter().map(|n| n.title.as_str()).collect();
    assert_eq!(titles, ["Chapter 1", "Capitolul 2 — ș"]);
    let pages: Vec<Option<usize>> = t
        .outline
        .iter()
        .map(|n| n.page.and_then(|p| t.index_of(p)))
        .collect();
    assert!(pages.iter().all(Option::is_some), "{pages:?}");
}

#[test]
fn bookmark_cycles_and_depth_are_cut_off() {
    use folio_core::{Dict, Object};
    let mut doc = samples::numbered(1).unwrap();
    let root = doc.add(Object::Dict(Dict::new()));
    // A chain 500 deep whose last item points back to the first.
    let mut items = Vec::new();
    for i in 0..500 {
        let mut d = Dict::new();
        d.set("Title", Object::Str(format!("B{i}").into_bytes()));
        items.push(doc.add(Object::Dict(d)));
    }
    for w in items.windows(2) {
        doc.get_mut(w[0])
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("First", w[1].into());
    }
    let last = *items.last().unwrap();
    doc.get_mut(last)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("First", items[0].into());
    let mut r = Dict::new();
    r.set("First", items[0].into());
    *doc.get_mut(root).unwrap() = Object::Dict(r);
    doc.catalog_mut().unwrap().set("Outlines", root.into());
    let t = tab(doc);
    let mut depth = 0;
    let mut level = &t.outline;
    while let Some(n) = level.first() {
        depth += 1;
        level = &n.children;
    }
    assert!((1..=64).contains(&depth), "depth {depth}");
}

#[test]
fn scrolling_layout_stacks_pages_and_finds_the_current_one() {
    let t = tab(samples::numbered(4).unwrap());
    let (tops, total) = t.page_tops(1.0);
    assert_eq!(tops.len(), 4);
    assert!(tops.windows(2).all(|w| w[0] < w[1]));
    assert!(total > tops[3]);
    assert_eq!(t.page_at(&tops, 0.0), 0);
    assert_eq!(t.page_at(&tops, tops[2] + 1.0), 2);
    assert_eq!(t.page_at(&tops, 1e12), 3);
}

#[test]
fn layout_switch_keeps_the_current_page() {
    let mut t = tab(samples::numbered(4).unwrap());
    t.go_to(2);
    t.scroll_to = None;
    t.set_layout(Layout::SinglePage);
    assert_eq!(t.current, 2);
    assert_eq!(t.scroll_to, Some(2));
}

#[test]
fn window_size_matches_the_renderer() {
    let doc = samples::rich().unwrap();
    for p in doc.pages().unwrap() {
        let (w, h) = folio_render::page_size_points(&doc, p);
        let (pw, ph) = folio_render::page_pixel_size(&doc, p, 72.0).unwrap();
        assert_eq!((w.ceil() as u32, h.ceil() as u32), (pw, ph));
    }
}

#[test]
fn rotate_moves_pixels_clockwise() {
    // 2 × 1: red, green.
    let src = [255, 0, 0, 255, 0, 255, 0, 255];
    let (out, w, h) = rotate(&src, 2, 1, 90);
    assert_eq!((w, h), (1, 2));
    assert_eq!(out, [255, 0, 0, 255, 0, 255, 0, 255]);
    let (out, w, h) = rotate(&src, 2, 1, 180);
    assert_eq!((w, h), (2, 1));
    assert_eq!(out, [0, 255, 0, 255, 255, 0, 0, 255]);
    let (out, _, _) = rotate(&src, 2, 1, 270);
    assert_eq!(out, [0, 255, 0, 255, 255, 0, 0, 255]);
    // Bad sizes give an empty image, never a panic.
    assert_eq!(rotate(&src, 3, 3, 90).1, 0);
}

fn key(page: usize, width: u32, rotation: u16) -> Key {
    Key {
        doc: 7,
        page,
        width,
        rotation,
        view: 0,
    }
}

#[test]
fn renderer_draws_in_the_background_at_the_asked_width() {
    let doc = Arc::new(folio_render::samples::vector().unwrap());
    let pages = doc.pages().unwrap();
    let mut r: Renderer<(u32, u32)> = Renderer::new(2, Arc::new(|| {}));
    r.request(key(0, 150, 0), &doc, pages[0]);
    r.request(key(0, 150, 90), &doc, pages[0]);
    assert!(r.wait_idle(Duration::from_secs(60)));
    let got = r.collect(|_, d| match d {
        Done::Image { width, height, .. } => (width, height),
        Done::Failed(m) => panic!("{m}"),
    });
    assert_eq!(got, 2);
    let (_, (w, h)) = r.get(key(0, 150, 0)).unwrap();
    assert_eq!(w, 150);
    let (_, (rw, rh)) = r.get(key(0, 150, 90)).unwrap();
    assert_eq!(rw, 150, "the asked width is after view rotation");
    assert!(h > 0 && rh > 0);
    // While a sharper image is drawn, the nearest one stands in.
    let (k, _) = r.get(key(0, 300, 0)).unwrap();
    assert_eq!(k.width, 150);
    assert!(r.cached_bytes() > 0);
    r.forget(7);
    assert!(r.get(key(0, 150, 0)).is_none());
    assert_eq!(r.cached_bytes(), 0);
}

#[test]
fn renderer_drops_requests_scrolled_away() {
    let doc = Arc::new(samples::numbered(200).unwrap());
    let pages = doc.pages().unwrap();
    // One worker, so most requests are still queued when we change our mind.
    let mut r: Renderer<()> = Renderer::new(1, Arc::new(|| {}));
    for (i, &p) in pages.iter().enumerate() {
        r.request(key(i, 600, 0), &doc, p);
    }
    let keep: HashSet<Key> = [key(199, 600, 0)].into_iter().collect();
    r.retain(&keep);
    assert!(r.wait_idle(Duration::from_secs(60)));
    let n = r.collect(|_, _| ());
    assert!(n < 10, "{n} pages were drawn after being dropped");
    assert!(r.get(key(199, 600, 0)).is_some());
}

#[test]
fn renderer_reports_pages_it_cannot_draw() {
    let mut doc = sized(612.0, 792.0);
    let p = doc.pages().unwrap()[0];
    // Contents pointing at a missing object, then garbage operators.
    doc.get_mut(p).unwrap().as_dict_mut().unwrap().set(
        "Contents",
        folio_core::ObjRef {
            num: 999,
            generation: 0,
        }
        .into(),
    );
    let doc = Arc::new(doc);
    let mut r: Renderer<bool> = Renderer::new(1, Arc::new(|| {}));
    r.request(key(0, 100, 0), &doc, p);
    assert!(r.wait_idle(Duration::from_secs(60)));
    r.collect(|_, d| matches!(d, Done::Image { .. }));
    // Either a blank page or a reported failure; never a hang or a panic.
    assert!(r.get(key(0, 100, 0)).is_some());
}

/// Found by the stage-4a gate: a zoom of NaN percent made the scale NaN,
/// and every page size and position with it.
#[test]
fn zoom_that_is_not_a_number_falls_back_to_actual_size() {
    let mut t = tab(sized(612.0, 792.0));
    t.set_zoom(Zoom::Percent(f32::NAN));
    assert_eq!(t.zoom, Zoom::Percent(100.0));
    for vp in [[0.0, 0.0], [f32::NAN, f32::NAN], [f32::INFINITY, 1.0]] {
        for z in [Zoom::FitPage, Zoom::FitWidth, Zoom::Percent(f32::INFINITY)] {
            t.set_zoom(z);
            let s = t.scale(vp);
            assert!(s.is_finite() && s > 0.0, "{z:?} in {vp:?} gave {s}");
        }
    }
    // Directly assigned (not through set_zoom), the scale still stays sane.
    t.zoom = Zoom::Percent(f32::NAN);
    assert!(t.scale([800.0, 600.0]).is_finite());
}
