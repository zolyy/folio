//! Anti-crash gate for stage 0 (PLAN.md §4). Run in full with
//! `cargo xtask crash`; `FOLIO_CRASH_ITERS` sets the number of mutants.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use folio_core::crashgen::{Rng, core_samples, exercise_core, mutate, nasty_inputs};

fn iterations() -> usize {
    std::env::var("FOLIO_CRASH_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2_000)
}

/// One input may take this long at most (in a release build it is
/// milliseconds; the margin covers slow CI machines and debug builds).
const SLOW: Duration = Duration::from_secs(10);

#[test]
fn nasty_inputs_do_not_crash() {
    for (i, input) in nasty_inputs().iter().enumerate() {
        let t = Instant::now();
        exercise_core(input);
        assert!(t.elapsed() < SLOW, "nasty input {i} took {:?}", t.elapsed());
    }
}

#[test]
fn mutated_documents_do_not_crash() {
    let samples = core_samples();
    let n = iterations();
    let mut rng = Rng::new(0x5EED_F011_0000_0001);
    let mut worst = Duration::ZERO;
    for i in 0..n {
        let base = &samples[i % samples.len()];
        let input = mutate(base, &mut rng);
        let t = Instant::now();
        let outcome = std::panic::catch_unwind(|| exercise_core(&input));
        let took = t.elapsed();
        worst = worst.max(took);
        if outcome.is_err() || took > SLOW {
            let path = std::env::temp_dir().join(format!("folio-crash-core-{i}.pdf"));
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
    eprintln!("stage 0 anti-crash: {n} mutants, slowest {worst:?}");
}
