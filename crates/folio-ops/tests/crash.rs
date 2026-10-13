//! Anti-crash gate for stage 1 (PLAN.md §4): every organize/properties/
//! text operation runs on every mutant. `FOLIO_CRASH_ITERS` sets the count.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use folio_core::Document;
use folio_core::crashgen::{Rng, core_samples, mutate, nasty_inputs};
use folio_ops::labels::{self, LabelChange, Style};
use folio_ops::merge::{self, Input};
use folio_ops::pages::{self, Margins, Orientation, PageBox, Subset};
use folio_ops::split::{self, SplitBy};
use folio_ops::{compress, info, meta, outline, ranges, samples, text};

/// Unoptimised (debug) builds and slow CI runners get more time; release
/// builds, which the full gate uses, must stay fast.
const SLOW: Duration = if cfg!(debug_assertions) {
    Duration::from_secs(60)
} else {
    Duration::from_secs(10)
};

fn iterations() -> usize {
    std::env::var("FOLIO_CRASH_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000)
}

fn stage1_samples() -> Vec<Vec<u8>> {
    let mut v = core_samples();
    v.push(samples::rich().unwrap().to_bytes().unwrap());
    let mut uncompressed = samples::rich().unwrap();
    for o in uncompressed.objects.values_mut() {
        if let folio_core::Object::Stream(s) = o
            && let Ok(d) = folio_core::filters::decode_stream(s)
        {
            s.data = d;
            s.dict.remove(b"Filter");
        }
    }
    // Uncompressed streams let the mutator reach content-stream and CMap syntax.
    v.push(uncompressed.to_bytes().unwrap());
    v
}

/// Every stage-1 operation. Errors are fine; panics and hangs are not.
fn exercise_ops(bytes: &[u8], rng: &mut Rng) {
    let Ok(doc) = Document::from_bytes(bytes) else {
        return;
    };
    let n = doc.pages().map(|p| p.len()).unwrap_or(0);
    let _ = info::info(&doc, Some(bytes.len() as u64)).map(|i| (i.to_json(), i.to_text()));
    let _ = outline::top_level(&doc);
    let _ = labels::per_page(&doc, n.min(10_000));
    if let Ok(p) = doc.pages() {
        for &pg in p.iter().take(16) {
            let _ = text::page_text(&doc, pg);
        }
    }
    let spec = ["1-3,last", "odd", "even", "2-", "-2", "3-1", "all"][rng.below(7)];
    let idx = ranges::parse(spec, n).unwrap_or_default();

    let mut d = doc.clone();
    let _ = pages::filter(&d, &idx, Subset::Odd, Orientation::Landscape);
    let _ = pages::rotate(&mut d, &idx, 90);
    let _ = pages::crop(
        &mut d,
        &idx,
        PageBox::Crop,
        Margins {
            top: 10.0,
            bottom: 5.0,
            left: 3.0,
            right: 1.0,
        },
    );
    let _ = pages::reset_box(&mut d, &idx, PageBox::Crop);
    let _ = d.to_bytes();

    let _ = pages::delete(&mut doc.clone(), &idx);
    let _ = pages::select(&mut doc.clone(), &idx);
    let _ = pages::reverse(&mut doc.clone());
    let _ = pages::insert_blank(&mut doc.clone(), n / 2, 1);
    let _ = pages::insert(&mut doc.clone(), doc.clone(), Some(&idx), n);
    let _ = pages::replace(&mut doc.clone(), &idx, doc.clone(), 0);
    let _ = labels::apply(
        &mut doc.clone(),
        0,
        n.saturating_sub(1),
        &LabelChange::NewSection {
            style: Style::RomanUpper,
            prefix: "P-".into(),
            start: 2,
        },
    );
    let _ = labels::apply(&mut doc.clone(), 0, 0, &LabelChange::Extend);
    let _ = meta::update(
        &mut doc.clone(),
        &meta::Update {
            title: Some("T".into()),
            ..meta::Update::default()
        },
        std::time::SystemTime::now(),
    );
    let mut c = doc.clone();
    let _ = compress::compress(&mut c);
    let _ = c.to_bytes();

    if n <= 64 {
        let _ = split::split(&doc, SplitBy::Pages(2));
        let _ = split::split(&doc, SplitBy::Bookmarks);
        let _ = split::split(&doc, SplitBy::MaxBytes(4096));
        let merged = merge::merge(
            vec![
                Input {
                    doc: doc.clone(),
                    name: "a".into(),
                    pages: None,
                },
                Input {
                    doc: doc.clone(),
                    name: "b".into(),
                    pages: Some(idx.clone()),
                },
            ],
            &merge::Options::default(),
        );
        if let Ok(m) = merged
            && let Ok(b) = m.to_bytes()
        {
            // What we write must read back.
            assert!(Document::from_bytes(&b).is_ok(), "merged output unreadable");
        }
    }
}

#[test]
fn stage1_nasty_inputs_do_not_crash() {
    let mut rng = Rng::new(7);
    for (i, input) in nasty_inputs().iter().enumerate() {
        let t = Instant::now();
        exercise_ops(input, &mut rng);
        assert!(t.elapsed() < SLOW, "nasty input {i} took {:?}", t.elapsed());
    }
}

#[test]
fn stage1_mutated_documents_do_not_crash() {
    let samples = stage1_samples();
    let n = iterations();
    let mut rng = Rng::new(0x5EED_F011_0000_0002);
    let mut worst = Duration::ZERO;
    for i in 0..n {
        let base = &samples[i % samples.len()];
        let input = mutate(base, &mut rng);
        let seed = i as u64 + 1;
        let t = Instant::now();
        let outcome = std::panic::catch_unwind(|| exercise_ops(&input, &mut Rng::new(seed)));
        let took = t.elapsed();
        worst = worst.max(took);
        if outcome.is_err() || took > SLOW {
            let path = std::env::temp_dir().join(format!("folio-crash-ops-{i}.pdf"));
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
    eprintln!("stage 1 anti-crash: {n} mutants x all operations, slowest {worst:?}");
}
