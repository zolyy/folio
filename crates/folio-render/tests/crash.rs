//! Anti-crash gate for stage 3a (PLAN.md §4): rendering mutated documents
//! must never panic, hang or allocate without bound.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::time::{Duration, Instant};

use folio_core::crashgen::{Rng, core_samples, mutate, nasty_inputs};
use folio_core::{Dict, Document};
use folio_render::{Options, render_page, samples};

/// Debug builds and slow CI runners get more time; release gates stay strict.
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

/// Small resolution keeps the gate fast while exercising every code path.
fn exercise_render(bytes: &[u8]) {
    let Ok(doc) = Document::from_bytes(bytes) else {
        return;
    };
    let Ok(pages) = doc.pages() else { return };
    for &p in pages.iter().take(8) {
        for dpi in [9.0, 18.0] {
            let opts = Options {
                dpi,
                smooth_line_art: dpi > 10.0,
                ..Options::default()
            };
            if let Ok(r) = render_page(&doc, p, &opts) {
                let _ = r.rgba(0, 0);
            }
        }
    }
}

/// Hand-written hostile content streams.
fn nasty_content() -> Vec<Vec<u8>> {
    let mut list: Vec<Vec<u8>> = vec![
        b"1e30 w 0 0 m 1e30 1e30 l S".to_vec(),
        b"NaN 0 0 1 0 0 cm 0 0 10 10 re f".to_vec(),
        b"[0 0 0] 0 d 0 0 m 100 100 l S".to_vec(),
        b"[0.0001] 0 d 1 w 0 0 m 600 800 l S".to_vec(),
        b"/CS0 cs 1 2 3 4 5 6 7 8 scn 0 0 50 50 re f".to_vec(),
        b"0 0 0 0 0 0 cm 0 0 100 100 re f".to_vec(),
        b"1e-30 0 0 1e-30 0 0 cm 0 0 1e40 1e40 re f".to_vec(),
        b"W n W* n h h h S f f* B b* n".to_vec(),
        b"/Missing Do /Missing gs /X cs 1 scn sh".to_vec(),
        b"BI /W 99999 /H 99999 ID \x00\x01 EI 0 0 1 1 re f".to_vec(),
    ];
    list.push(b"q ".repeat(50_000));
    list.push(b"Q ".repeat(50_000));
    let mut many = Vec::new();
    for i in 0..20_000 {
        many.extend_from_slice(format!("q {i} 0 0 1 1 re W n Q ").as_bytes());
    }
    list.push(many);
    list.push(
        b"0 0 m "
            .repeat(10_000)
            .into_iter()
            .chain(b"1 1 l S".iter().copied())
            .collect(),
    );
    list
}

#[test]
fn stage3a_hostile_content_does_not_crash() {
    for (i, content) in nasty_content().iter().enumerate() {
        let mut doc = Document::new();
        // A self-referencing form XObject to test recursion guards.
        let form = doc.add(folio_core::Object::Null);
        let mut fd = Dict::new();
        fd.set("Subtype", folio_core::Object::name("Form"));
        let mut xo = Dict::new();
        xo.set("F", form.into());
        let mut fres = Dict::new();
        fres.set("XObject", xo.clone().into());
        fd.set("Resources", fres.into());
        doc.objects.insert(
            form.num,
            folio_core::Object::Stream(folio_core::Stream {
                dict: fd,
                data: b"/F Do 0 0 1 1 re f".to_vec(),
            }),
        );
        let mut res = Dict::new();
        res.set("XObject", xo.into());
        let mut c = content.clone();
        c.extend_from_slice(b" /F Do");
        doc.add_page([612.0, 792.0], &c, res).unwrap();
        let t = Instant::now();
        exercise_render(&doc.to_bytes().unwrap());
        assert!(
            t.elapsed() < SLOW,
            "hostile content {i} took {:?}",
            t.elapsed()
        );
    }
    for input in nasty_inputs() {
        exercise_render(&input);
    }
}

#[test]
fn stage3a_mutated_documents_do_not_crash() {
    let mut bases = core_samples();
    bases.push(samples::vector().unwrap().to_bytes().unwrap());
    bases.push(samples::vector_uncompressed().unwrap().to_bytes().unwrap());
    bases.push(common::image_sample().to_bytes().unwrap());
    let n = iterations();
    let mut rng = Rng::new(0x5EED_F011_0000_003A);
    let mut worst = Duration::ZERO;
    for i in 0..n {
        let input = mutate(&bases[i % bases.len()], &mut rng);
        let t = Instant::now();
        let outcome = std::panic::catch_unwind(|| exercise_render(&input));
        let took = t.elapsed();
        worst = worst.max(took);
        if outcome.is_err() || took > SLOW {
            let path = std::env::temp_dir().join(format!("folio-crash-render-{i}.pdf"));
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
    eprintln!(
        "stage 3 anti-crash (vector + images): {n} mutants rendered at 2 resolutions, slowest {worst:?}"
    );
}

/// Hostile image dictionaries and data for every decoder.
#[test]
fn stage3b_hostile_images_do_not_crash() {
    use common::{image_stream, int, name};
    use folio_core::Object;
    let mut rng = Rng::new(0x5EED_F011_0000_003B);
    let mut junk = |n: usize| -> Vec<u8> { (0..n).map(|_| rng.next_u64() as u8).collect() };
    type Case<'a> = (Vec<(&'a str, Object)>, Vec<u8>);
    let cases: Vec<Case> = vec![
        (
            vec![
                ("Width", int(100_000)),
                ("Height", int(100_000)),
                ("BitsPerComponent", int(8)),
            ],
            vec![0; 16],
        ),
        (vec![("Width", int(-3)), ("Height", int(2))], vec![]),
        (
            vec![
                ("Width", int(4)),
                ("Height", int(4)),
                ("BitsPerComponent", int(3)),
            ],
            vec![0; 16],
        ),
        (
            vec![
                ("Width", int(4)),
                ("Height", int(4)),
                ("BitsPerComponent", int(8)),
                ("Decode", Object::Array(vec![Object::Real(f64::NAN)])),
            ],
            vec![],
        ),
        (
            vec![
                ("Width", int(64)),
                ("Height", int(64)),
                ("Filter", name("DCTDecode")),
            ],
            junk(500),
        ),
        (
            vec![
                ("Width", int(64)),
                ("Height", int(64)),
                ("Filter", name("JPXDecode")),
            ],
            junk(500),
        ),
        (
            vec![
                ("Width", int(64)),
                ("Height", int(64)),
                ("Filter", name("JBIG2Decode")),
            ],
            junk(500),
        ),
        (
            vec![
                ("Width", int(64)),
                ("Height", int(64)),
                ("Filter", name("CCITTFaxDecode")),
            ],
            junk(500),
        ),
        (
            vec![
                ("Width", int(64)),
                ("Height", int(64)),
                ("ImageMask", Object::Bool(true)),
                ("Filter", name("CCITTFaxDecode")),
            ],
            junk(64),
        ),
        (
            vec![
                ("Width", int(2)),
                ("Height", int(2)),
                (
                    "ColorSpace",
                    Object::Array(vec![
                        name("Indexed"),
                        name("DeviceRGB"),
                        int(255),
                        Object::Str(vec![1]),
                    ]),
                ),
            ],
            vec![255; 4],
        ),
        (
            vec![
                ("Width", int(2)),
                ("Height", int(2)),
                ("Mask", Object::Array(vec![int(-1)])),
            ],
            vec![0; 4],
        ),
    ];
    for (i, (dict, data)) in cases.into_iter().enumerate() {
        let mut doc = Document::new();
        let x = doc.add(image_stream(&dict, data));
        let mut xo = Dict::new();
        xo.set("X", x.into());
        let mut res = Dict::new();
        res.set("XObject", xo.into());
        doc.add_page(
            [200.0, 200.0],
            b"q 100 0 0 100 0 0 cm /X Do Q BI /W 99999 /H 99999 /F /DCT ID xx EI",
            res,
        )
        .unwrap();
        let t = Instant::now();
        exercise_render(&doc.to_bytes().unwrap());
        assert!(
            t.elapsed() < SLOW,
            "hostile image {i} took {:?}",
            t.elapsed()
        );
    }
}
