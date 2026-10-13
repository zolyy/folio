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

/// Stage 2: protected files at every compatibility level.
fn encrypted_samples() -> Vec<Vec<u8>> {
    use folio_core::Document;
    use folio_core::crypt::{Changes, Compat, Printing, Settings};
    let mut out = Vec::new();
    for compat in [
        Compat::Acrobat3,
        Compat::Acrobat5,
        Compat::Acrobat6,
        Compat::Acrobat7,
        Compat::AcrobatX,
    ] {
        let mut d = Document::new();
        d.add_text_page("Protected (text)").unwrap();
        d.add_text_page("Two").unwrap();
        d.encrypt(&Settings {
            compat,
            open_password: Some("user".into()),
            permissions_password: Some("owner".into()),
            printing: Printing::Low,
            changes: Changes::Pages,
            ..Settings::default()
        })
        .unwrap();
        out.push(d.to_bytes().unwrap());
    }
    out
}

/// Hand-written hostile encryption dictionaries.
fn nasty_encryption() -> Vec<Vec<u8>> {
    let base = |enc: &str| {
        format!(
            "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [] /Count 0 >> endobj\n3 0 obj {enc} endobj\ntrailer << /Root 1 0 R /Encrypt 3 0 R /ID [<00> <00>] >>\n"
        )
        .into_bytes()
    };
    vec![
        base("<< /Filter /Standard /V 5 /R 6 /O <00> /U <00> /P -1 >>"),
        base("<< /Filter /Standard /V 4 /R 4 /Length 128 /O <00> /U <00> /P -4 /StmF /Missing >>"),
        base("<< /Filter /Standard /V 2 /R 3 /Length 99999 /O () /U () /P 99999999999 >>"),
        base("<< /Filter /Standard /V 1 /R 2 >>"),
        base("<< /Filter /Standard /V 7 /R 9 >>"),
        base("<< /Filter /PubSec /V 4 /R 4 >>"),
        base("[1 2 3]"),
        base(
            "<< /Filter /Standard /V 4 /R 4 /CF << /StdCF << /CFM /AESV2 >> >> /StmF /StdCF /StrF /StdCF /O <0000> /U <0000> /P 0 >>",
        ),
    ]
}

/// Everything stage 2 adds, with each password.
fn exercise_security(bytes: &[u8]) {
    use folio_core::Document;
    use folio_core::crypt::{Need, Settings};
    for pw in ["", "user", "owner", "wrong"] {
        let Ok(mut d) = Document::from_bytes_with_password(bytes, pw) else {
            continue;
        };
        for n in [
            Need::Assemble,
            Need::Extract,
            Need::Modify,
            Need::Copy,
            Need::Owner,
        ] {
            let _ = d.require(n);
        }
        exercise_core(&d.to_bytes().unwrap_or_default());
        if let Ok(out) = d.to_bytes() {
            let _ = Document::from_bytes_with_password(&out, pw);
        }
        let mut e = d.clone();
        let _ = e.encrypt(&Settings {
            open_password: Some("again".into()),
            ..Settings::default()
        });
        let _ = e.to_bytes();
        if d.remove_security().is_ok() {
            let _ = d.to_bytes();
        }
    }
}

#[test]
fn stage2_encrypted_mutants_do_not_crash() {
    for (i, input) in nasty_encryption().iter().enumerate() {
        let t = Instant::now();
        exercise_security(input);
        assert!(t.elapsed() < SLOW, "nasty encryption {i} too slow");
    }
    let samples = encrypted_samples();
    let n = iterations() / 4;
    let mut rng = Rng::new(0x5EED_F011_0000_0003);
    let mut worst = Duration::ZERO;
    for i in 0..n {
        let input = mutate(&samples[i % samples.len()], &mut rng);
        let t = Instant::now();
        let outcome = std::panic::catch_unwind(|| exercise_security(&input));
        let took = t.elapsed();
        worst = worst.max(took);
        if outcome.is_err() || took > SLOW {
            let path = std::env::temp_dir().join(format!("folio-crash-sec-{i}.pdf"));
            let _ = std::fs::write(&path, &input);
            panic!(
                "encrypted mutant {i} {} (saved to {}); add it as a regression test",
                if outcome.is_err() {
                    "panicked"
                } else {
                    "was too slow"
                },
                path.display()
            );
        }
    }
    eprintln!("stage 2 anti-crash: {n} encrypted mutants x 4 passwords, slowest {worst:?}");
}

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
