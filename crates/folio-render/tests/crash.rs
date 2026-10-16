//! Anti-crash gate for stage 3a (PLAN.md §4): rendering mutated documents
//! must never panic, hang or allocate without bound.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::time::{Duration, Instant};

use folio_core::crashgen::{Rng, core_samples, mutate, nasty_inputs};
use folio_core::{Dict, Document};
use folio_render::{Options, samples};

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
            if let Ok(r) = folio_render::interp_for_tests::render(&doc, p, &opts) {
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
    bases.push(samples::text().unwrap().to_bytes().unwrap());
    let mut plain = samples::text().unwrap();
    for o in plain.objects.values_mut() {
        if let folio_core::Object::Stream(st) = o
            && let Ok(d) = folio_core::filters::decode_stream(st)
        {
            st.data = d;
            st.dict.remove(b"Filter");
        }
    }
    bases.push(plain.to_bytes().unwrap());
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

/// Stage 3c: font programs, CMaps and Type 3 fonts, damaged on purpose.
#[test]
fn stage3c_hostile_fonts_do_not_crash() {
    use folio_core::{Object, Stream};
    let mut rng = Rng::new(0x5EED_F011_0000_003C);
    let programs: Vec<(&str, Option<&str>, Vec<u8>)> = vec![
        ("FontFile2", None, samples::tiny_truetype()),
        ("FontFile3", Some("Type1C"), samples::tiny_cff()),
        ("FontFile3", Some("OpenType"), samples::tiny_truetype()),
        ("FontFile", None, samples::tiny_type1()),
    ];
    let n = (iterations() / 4).max(50);
    for i in 0..n {
        let (key, sub, base) = &programs[i % programs.len()];
        let data = mutate(base, &mut rng);
        let mut doc = Document::new();
        let mut sd = Dict::new();
        if let Some(s) = sub {
            sd.set("Subtype", Object::name(s));
        }
        let file = doc.add(Object::Stream(Stream { dict: sd, data }));
        let mut desc = Dict::new();
        desc.set("Flags", Object::Int((i % 64) as i64));
        desc.set(*key, file.into());
        let desc = doc.add(desc.into());
        let cmap = doc.add(Object::Stream(Stream {
            dict: Dict::new(),
            data: mutate(
                b"1 begincodespacerange <00> <FFFF> endcodespacerange 1 begincidrange <0000> <FFFF> 0 endcidrange",
                &mut rng,
            ),
        }));
        let mut cid = Dict::new();
        let cid_kind = if *key == "FontFile2" {
            "CIDFontType2"
        } else {
            "CIDFontType0"
        };
        cid.set("Subtype", Object::name(cid_kind));
        cid.set("FontDescriptor", desc.into());
        cid.set(
            "W",
            Object::Array(vec![Object::Int(0), Object::Int(65535), Object::Int(500)]),
        );
        let cid = doc.add(cid.into());
        let mut t0 = Dict::new();
        t0.set("Subtype", Object::name("Type0"));
        t0.set("Encoding", cmap.into());
        t0.set("DescendantFonts", Object::Array(vec![cid.into()]));
        let mut simple = Dict::new();
        let simple_kind = if *key == "FontFile2" {
            "TrueType"
        } else {
            "Type1"
        };
        simple.set("Subtype", Object::name(simple_kind));
        simple.set("FontDescriptor", desc.into());
        let mut enc = Dict::new();
        enc.set(
            "Differences",
            Object::Array(vec![
                Object::Int(-5),
                Object::name("A"),
                Object::Int(300),
                Object::name("uniZZZZ"),
            ]),
        );
        simple.set("Encoding", enc.into());
        // A Type 3 font whose glyph shows itself again.
        let mut t3 = Dict::new();
        t3.set("Subtype", Object::name("Type3"));
        t3.set(
            "FontMatrix",
            Object::Array([1, 0, 0, 1, 0, 0].iter().map(|&v| Object::Int(v)).collect()),
        );
        let glyph = doc.add(Object::Stream(Stream {
            dict: Dict::new(),
            data: b"BT /F3 1 Tf (A) Tj ET 0 0 1 1 re f".to_vec(),
        }));
        let mut procs = Dict::new();
        procs.set("A", glyph.into());
        t3.set("CharProcs", procs.into());
        let mut e3 = Dict::new();
        e3.set(
            "Differences",
            Object::Array(vec![Object::Int(65), Object::name("A")]),
        );
        t3.set("Encoding", e3.into());
        let t3r = doc.add(Object::Null);
        let mut fonts = Dict::new();
        let f0 = doc.add(t0.into());
        let f1 = doc.add(simple.into());
        fonts.set("F0", f0.into());
        fonts.set("F1", f1.into());
        fonts.set("F3", t3r.into());
        let mut r3 = Dict::new();
        r3.set("Font", fonts.clone().into());
        t3.set("Resources", r3.into());
        doc.objects.insert(t3r.num, t3.into());
        let mut res = Dict::new();
        res.set("Font", fonts.into());
        let content = b"BT /F0 40 Tf 10 10 Td <00410042FFFF> Tj /F1 40 Tf (ABC\\377) Tj 7 Tr 5 Tr /F3 40 Tf (AAA) Tj ET";
        doc.add_page([200.0, 200.0], content, res).unwrap();
        let bytes = doc.to_bytes().unwrap();
        let t = Instant::now();
        let outcome = std::panic::catch_unwind(|| exercise_render(&bytes));
        assert!(outcome.is_ok(), "hostile font {i} panicked");
        assert!(
            t.elapsed() < SLOW,
            "hostile font {i} took {:?}",
            t.elapsed()
        );
    }
    eprintln!("stage 3c anti-crash: {n} hostile font programs");
}

/// Stage 3d: shadings, patterns, soft masks, groups and annotations, damaged
/// on purpose, plus mutations of the stage-3d sample.
#[test]
fn stage3d_hostile_shading_and_transparency_do_not_crash() {
    use folio_core::{Object, Stream};
    let num = |v: f64| Object::Real(v);
    let arr = |v: &[f64]| Object::Array(v.iter().map(|&x| Object::Real(x)).collect());
    let mut doc = Document::new();

    // A tiling pattern whose cell paints with itself, and a soft mask whose
    // group sets the same soft mask again.
    let pat = doc.add(Object::Null);
    let mut pres = Dict::new();
    let mut pdict = Dict::new();
    pdict.set("P", pat.into());
    pres.set("Pattern", pdict.clone().into());
    let mut pd = Dict::new();
    pd.set("PatternType", Object::Int(1));
    pd.set("PaintType", Object::Int(1));
    pd.set("BBox", arr(&[0.0, 0.0, 0.001, 1e9]));
    pd.set("XStep", num(0.0001));
    pd.set("YStep", num(1e12));
    pd.set("Resources", pres.into());
    doc.objects.insert(
        pat.num,
        Object::Stream(Stream {
            dict: pd,
            data: b"/Pattern cs /P scn 0 0 1 1 re f".to_vec(),
        }),
    );
    let grp = doc.add(Object::Null);
    let mut sm = Dict::new();
    sm.set("S", Object::name("Luminosity"));
    sm.set("G", grp.into());
    sm.set("BC", arr(&[f64::NAN]));
    sm.set("TR", Object::Int(7));
    let mut gsd = Dict::new();
    gsd.set("SMask", sm.into());
    let mut ext = Dict::new();
    ext.set("S", gsd.into());
    let mut gres = Dict::new();
    gres.set("ExtGState", ext.clone().into());
    let mut gd = Dict::new();
    gd.set("Subtype", Object::name("Form"));
    gd.set("BBox", arr(&[0.0, 0.0, 100.0, 100.0]));
    gd.set("Resources", gres.into());
    let mut g = Dict::new();
    g.set("S", Object::name("Transparency"));
    gd.set("Group", g.into());
    doc.objects.insert(
        grp.num,
        Object::Stream(Stream {
            dict: gd,
            data: b"/S gs 1 g 0 0 50 50 re f /G Do".to_vec(),
        }),
    );

    let mut shadings = Dict::new();
    let mut bad_radial = Dict::new();
    bad_radial.set("ShadingType", Object::Int(3));
    bad_radial.set("ColorSpace", Object::name("DeviceRGB"));
    bad_radial.set("Coords", arr(&[0.0, 0.0, -5.0, f64::NAN, 1e300, -1e300]));
    let mut f = Dict::new();
    f.set("FunctionType", Object::Int(2));
    f.set("C0", arr(&[0.0]));
    f.set("C1", arr(&[1.0, 1.0, 1.0, 1.0]));
    f.set("N", num(-3.5));
    bad_radial.set("Function", f.into());
    shadings.set("R", bad_radial.into());
    // A mesh claiming 32-bit fields over random data.
    let mut rng = Rng::new(0x5EED_F011_0000_003D);
    for (name, ty) in [("M4", 4), ("M5", 5), ("M6", 6), ("M7", 7)] {
        let mut d = Dict::new();
        d.set("ShadingType", Object::Int(ty));
        d.set("ColorSpace", Object::name("DeviceCMYK"));
        d.set("BitsPerCoordinate", Object::Int(32));
        d.set("BitsPerComponent", Object::Int(16));
        d.set("BitsPerFlag", Object::Int(8));
        d.set("VerticesPerRow", Object::Int(2));
        d.set(
            "Decode",
            arr(&[-1e9, 1e9, -1e9, 1e9, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0]),
        );
        let data: Vec<u8> = (0..200_000).map(|_| rng.next_u64() as u8).collect();
        let r = doc.add(Object::Stream(Stream { dict: d, data }));
        shadings.set(name, r.into());
    }

    let mut xo = Dict::new();
    xo.set("G", grp.into());
    let mut res = Dict::new();
    res.set("Pattern", pdict.into());
    res.set("Shading", shadings.into());
    res.set("ExtGState", ext.into());
    res.set("XObject", xo.into());
    let page = doc
        .add_page(
            [300.0, 300.0],
            b"/R sh /M4 sh /M5 sh /M6 sh /M7 sh /Pattern cs /P scn 0 0 300 300 re f /S gs /G Do 0 0 100 100 re f",
            res,
        )
        .unwrap();
    // Annotations with absurd rectangles and appearance cycles.
    let ap = doc.add(Object::Null);
    let mut apd = Dict::new();
    apd.set("Subtype", Object::name("Form"));
    apd.set("BBox", arr(&[0.0, 0.0, 0.0, 0.0]));
    apd.set("Matrix", arr(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]));
    doc.objects.insert(
        ap.num,
        Object::Stream(Stream {
            dict: apd,
            data: b"0 0 10 10 re f".to_vec(),
        }),
    );
    let mut annots = Vec::new();
    for r in [
        [0.0, 0.0, 1e30, 1e30],
        [f64::NAN, 0.0, 1.0, 1.0],
        [5.0, 5.0, 5.0, 5.0],
    ] {
        let mut n = Dict::new();
        n.set("N", ap.into());
        let mut a = Dict::new();
        a.set("Subtype", Object::name("Stamp"));
        a.set("Rect", arr(&r));
        a.set("AP", n.into());
        annots.push(doc.add(a.into()).into());
    }
    if let Some(d) = doc.get_mut(page).and_then(Object::as_dict_mut) {
        d.set("Annots", Object::Array(annots));
    }
    let bytes = doc.to_bytes().unwrap();
    let t = Instant::now();
    let outcome = std::panic::catch_unwind(|| exercise_render(&bytes));
    assert!(outcome.is_ok(), "hostile stage-3d page panicked");
    assert!(
        t.elapsed() < SLOW,
        "hostile stage-3d page took {:?}",
        t.elapsed()
    );

    // Mutations of the stage-3d sample.
    let base = samples::shading().unwrap().to_bytes().unwrap();
    let mut plain = samples::shading().unwrap();
    for o in plain.objects.values_mut() {
        if let Object::Stream(st) = o
            && let Ok(d) = folio_core::filters::decode_stream(st)
        {
            st.data = d;
            st.dict.remove(b"Filter");
        }
    }
    let plain = plain.to_bytes().unwrap();
    let n = (iterations() / 4).max(50);
    let mut worst = Duration::ZERO;
    for i in 0..n {
        let input = mutate(if i % 2 == 0 { &base } else { &plain }, &mut rng);
        let t = Instant::now();
        let outcome = std::panic::catch_unwind(|| exercise_render(&input));
        let took = t.elapsed();
        worst = worst.max(took);
        if outcome.is_err() || took > SLOW {
            let path = std::env::temp_dir().join(format!("folio-crash-3d-{i}.pdf"));
            let _ = std::fs::write(&path, &input);
            panic!(
                "stage-3d mutant {i} {} (saved to {})",
                if outcome.is_err() {
                    "panicked"
                } else {
                    "was too slow"
                },
                path.display()
            );
        }
    }
    eprintln!("stage 3d anti-crash: hostile page + {n} mutants, slowest {worst:?}");
}
