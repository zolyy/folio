//! Anti-crash gate for stage 4a (PLAN.md §4): opening mutated documents in
//! the reading window and hammering it with random view commands, window
//! sizes, typed text and key presses must never panic, hang or grow without
//! bound.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use folio_app::app::{App, Config, Pane};
use folio_app::cache::{Key as PageKey, Renderer};
use folio_app::viewer::{Layout, Tab, Zoom};
use folio_core::Document;
use folio_core::crashgen::{Rng, core_samples, mutate};
use folio_ops::samples;

const SLOW: Duration = if cfg!(debug_assertions) {
    Duration::from_secs(60)
} else {
    Duration::from_secs(10)
};

fn iterations() -> usize {
    std::env::var("FOLIO_CRASH_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600)
}

fn bases() -> Vec<Vec<u8>> {
    let mut b = core_samples();
    b.push(samples::rich().unwrap().to_bytes().unwrap());
    b.push(samples::numbered(6).unwrap().to_bytes().unwrap());
    b.push(folio_render::samples::vector().unwrap().to_bytes().unwrap());
    b.push(folio_render::samples::text().unwrap().to_bytes().unwrap());
    b
}

const VIEWPORTS: [[f32; 2]; 7] = [
    [1000.0, 800.0],
    [0.0, 0.0],
    [-10.0, 300.0],
    [1.0, 1e9],
    [f32::MAX, f32::MAX],
    [f32::NAN, 500.0],
    [f32::INFINITY, f32::NEG_INFINITY],
];

const TYPED: [&str; 12] = [
    "",
    "0",
    "1",
    "-1",
    "99999999999999999999",
    "nan",
    "inf",
    "1e40",
    "%%",
    "ii",
    "A-1",
    "3,5",
];

fn random_text(rng: &mut Rng) -> String {
    if rng.below(2) == 0 {
        return TYPED[rng.below(TYPED.len())].to_owned();
    }
    let n = rng.below(12);
    let bytes: Vec<u8> = (0..n).map(|_| rng.below(256) as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Every view command, in random order, with hostile arguments.
fn hammer(t: &mut Tab, rng: &mut Rng) {
    for _ in 0..60 {
        let vp = VIEWPORTS[rng.below(VIEWPORTS.len())];
        match rng.below(20) {
            0 => t.zoom_in(vp),
            1 => t.zoom_out(vp),
            2 => {
                let _ = t.set_zoom_text(&random_text(rng));
            }
            3 => t.set_zoom([Zoom::FitPage, Zoom::FitWidth, Zoom::Percent(f32::NAN)][rng.below(3)]),
            4 => t.rotate_clockwise(),
            5 => t.rotate_counterclockwise(),
            6 => t.go_to(rng.next_u64() as usize),
            7 => {
                let _ = t.go_to_text(&random_text(rng));
            }
            8 => t.next_page(),
            9 => t.previous_page(),
            10 => t.last_page(),
            11 => t.set_layout(
                [
                    Layout::SinglePage,
                    Layout::Continuous,
                    Layout::TwoPage,
                    Layout::TwoPageScrolling,
                ][rng.below(4)],
            ),
            // Stage 4c: cover page, spreads, placements, layers.
            16 => t.set_cover_page(rng.below(2) == 0),
            17 => {
                let s = t.scale(vp);
                let (placed, total, width) = t.placements(s);
                assert!(total.is_finite() && width.is_finite());
                assert!(placed.iter().all(|p| p.page < t.page_count()));
            }
            18 => {
                if let Some(l) = t
                    .layers
                    .get(rng.below(t.layers.len() + 1))
                    .and_then(|l| l.ocg)
                {
                    t.set_layer_visible(l.num, rng.below(2) == 0);
                }
            }
            19 => t.reset_layers(),
            12 => {
                let s = t.scale(vp);
                assert!(
                    s.is_finite() && s > 0.0,
                    "scale {s} for {vp:?} at {:?}",
                    t.zoom
                );
                let (tops, _) = t.page_tops(s);
                let _ = t.page_at(&tops, [0.0, -1.0, f32::NAN, 1e30][rng.below(4)]);
            }
            13 => {
                let _ = t.percent(vp);
                let _ = t.display_size(rng.below(t.page_count() + 2), 1.0);
            }
            14 => {
                let _ = t.page_name(rng.below(t.page_count() + 2));
            }
            _ => t.first_page(),
        }
        assert!(t.page_count() == 0 || t.current < t.page_count());
        assert!(t.rotation.is_multiple_of(90) && t.rotation < 360);
    }
}

#[test]
fn stage4a_view_commands_on_mutated_documents() {
    let bases = bases();
    let n = iterations();
    let mut rng = Rng::new(0x5EED_F011_0000_004A);
    let mut renderer: Renderer<()> = Renderer::new(2, Arc::new(|| {}));
    let mut worst = Duration::ZERO;
    for i in 0..n {
        let input = mutate(&bases[i % bases.len()], &mut rng);
        let t0 = Instant::now();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let Ok(doc) = Document::from_bytes(&input) else {
                return;
            };
            let mut tab = Tab::new(doc, None, format!("mutant-{i}.pdf"), i as u64);
            hammer(&mut tab, &mut rng);
            // The background renderer at small sizes, in every view rotation.
            let mut wanted = HashSet::new();
            for page in 0..tab.page_count().min(3) {
                // Stage 4c: hidden layers and replaced colours as well.
                let view = folio_app::cache::ViewOptions {
                    hidden_layers: (rng.below(2) == 0).then(|| {
                        let all = folio_render::oc::all_groups(&tab.doc);
                        all.iter().map(|r| r.num).take(rng.below(4)).collect()
                    }),
                    replace_colors: (rng.below(2) == 0).then(|| folio_render::ReplaceColors {
                        line_art: rng.below(2) == 0,
                        only_black: rng.below(2) == 0,
                        ..folio_render::ReplaceColors::HIGH_CONTRAST[rng.below(4)].1
                    }),
                };
                let key = PageKey {
                    doc: i as u64,
                    page,
                    width: 24 + rng.below(40) as u32,
                    rotation: [0, 90, 180, 270][rng.below(4)],
                    view: view.key(),
                };
                wanted.insert(key);
                renderer.request_with(key, &tab.doc, tab.pages[page], &view);
            }
            renderer.retain(&wanted);
            assert!(renderer.wait_idle(SLOW), "renderer did not finish");
            renderer.collect(|_, _| ());
            renderer.forget(i as u64);
        }));
        let took = t0.elapsed();
        worst = worst.max(took);
        if outcome.is_err() || took > SLOW {
            let path = std::env::temp_dir().join(format!("folio-crash-view-{i}.pdf"));
            let _ = std::fs::write(&path, &input);
            panic!(
                "mutant {i} {} (saved to {}); add it as a regression test",
                if outcome.is_err() {
                    "panicked"
                } else {
                    "was too slow"
                },
                path.display()
            );
        }
    }
    assert_eq!(
        renderer.cached_bytes(),
        0,
        "forgotten documents left images behind"
    );
    eprintln!("stage 4 anti-crash (view model + renderer): {n} mutants, slowest {worst:?}");
}

const BUTTONS: [&str; 12] = [
    "Next Page",
    "Previous Page",
    "First Page",
    "Last Page",
    "Zoom In",
    "Zoom Out",
    "Fit Page",
    "Fit Width",
    "Rotate Clockwise",
    "Rotate Counterclockwise",
    "Single Page",
    "Scrolling",
];

const KEYS: [Key; 9] = [
    Key::ArrowRight,
    Key::ArrowLeft,
    Key::ArrowDown,
    Key::PageDown,
    Key::Home,
    Key::End,
    Key::F4,
    Key::Minus,
    Key::Equals,
];

/// The whole window: fewer mutants (each one draws real frames).
#[test]
fn stage4a_window_on_mutated_documents() {
    let bases = bases();
    let n = (iterations() / 20).clamp(10, 400);
    let mut rng = Rng::new(0x5EED_F011_0000_04A1);
    let mut worst = Duration::ZERO;
    for i in 0..n {
        let input = mutate(&bases[i % bases.len()], &mut rng);
        let t0 = Instant::now();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let mut app = App::new(Config {
                recent_file: None,
                threads: 1,
            });
            app.open_bytes(input.clone(), None, format!("mutant-{i}.pdf"));
            app.open_bytes(input.clone(), None, format!("again-{i}.pdf"));
            app.pane = [
                Pane::Thumbnails,
                Pane::Bookmarks,
                Pane::Layers,
                Pane::Attachments,
                Pane::Hidden,
            ][rng.below(5)];
            app.properties_open = rng.below(3) == 0;
            app.colors_open = rng.below(4) == 0;
            app.colors.enabled = rng.below(2) == 0;
            app.read_mode = rng.below(4) == 0;
            app.apply_colors();
            let size = [Vec2::new(900.0, 640.0), Vec2::new(200.0, 120.0)][rng.below(2)];
            let mut h = Harness::builder()
                .with_size(size)
                .with_max_steps(6)
                .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
            for _ in 0..8 {
                match rng.below(4) {
                    0 => {
                        let name = BUTTONS[rng.below(BUTTONS.len())];
                        if let Some(b) = h.query_by_label(name) {
                            b.click();
                        }
                    }
                    1 => h.key_press(KEYS[rng.below(KEYS.len())]),
                    2 => h.key_press_modifiers(
                        Modifiers::COMMAND,
                        [
                            Key::Num0,
                            Key::Num1,
                            Key::Num2,
                            Key::Tab,
                            Key::W,
                            Key::L,
                            Key::H,
                            Key::D,
                        ][rng.below(8)],
                    ),
                    _ => {
                        if let Some(t) = h.state_mut().tab_mut() {
                            let _ = t.set_zoom_text(&random_text(&mut rng));
                        }
                    }
                }
                h.run_ok();
                assert!(
                    h.state().renderer.wait_idle(SLOW),
                    "renderer did not finish"
                );
            }
            h.run_ok();
        }));
        let took = t0.elapsed();
        worst = worst.max(took);
        if outcome.is_err() || took > SLOW * 2 {
            let path = std::env::temp_dir().join(format!("folio-crash-window-{i}.pdf"));
            let _ = std::fs::write(&path, &input);
            panic!(
                "mutant {i} {} in the window (saved to {}); add it as a regression test",
                if outcome.is_err() {
                    "panicked"
                } else {
                    "was too slow"
                },
                path.display()
            );
        }
    }
    eprintln!("stage 4 anti-crash (whole window): {n} mutants, slowest {worst:?}");
}

/// Stage 4b: read the text, search with random queries, step through the
/// matches and select across pages, on mutated documents.
#[test]
fn stage4b_find_and_selection_on_mutated_documents() {
    use folio_app::find::{FindState, Selection, from_view, to_view};
    let bases = bases();
    let n = (iterations() / 10).max(50);
    let mut rng = Rng::new(0x5EED_F011_0000_004B);
    for i in 0..n {
        let input = mutate(&bases[i % bases.len()], &mut rng);
        let Ok(doc) = folio_core::Document::from_bytes(&input) else {
            continue;
        };
        let mut t = Tab::new(doc, None, "m.pdf".into(), 1);
        let (d, p) = (t.doc.clone(), t.pages.clone());
        t.index.fill_now(&d, &p[..p.len().min(6)]);
        let mut f = FindState::default();
        for _ in 0..6 {
            f.query = random_text(&mut rng);
            f.whole_words = rng.below(2) == 0;
            f.case_sensitive = rng.below(2) == 0;
            f.refresh(t.id, &t.index);
            let _ = f.next(rng.below(t.page_count() + 3));
            let _ = f.previous(rng.below(t.page_count() + 3));
            let _ = f.status(&t.index);
        }
        let pick = |rng: &mut Rng| (rng.below(t.page_count() + 3), rng.next_u64() as usize);
        let s = Selection {
            anchor: pick(&mut rng),
            head: pick(&mut rng),
        };
        let _ = s.text(&t.index);
        for r in [0u16, 90, 180, 270, 45, u16::MAX] {
            let v = to_view([f32::NAN, 1e30], [0.0, -5.0], r);
            let _ = from_view(v, [1e-30, 1e30], r);
        }
    }
}
