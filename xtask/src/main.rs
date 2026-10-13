//! The verification agent. Contributors run exactly what CI runs:
//!
//! ```text
//! cargo xtask ci              every gate below, in order
//! cargo xtask crash [--iters N] [--log STAGE]   the anti-crash gate (release build)
//! cargo xtask parity          parity/acrobat-features.toml is honest
//! cargo xtask assets          every asset file is attributed
//! cargo xtask lints           every crate inherits the never-crash lints
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = workspace_root();
    std::env::set_current_dir(&root).expect("cannot enter the workspace root");
    let result = match args.first().map(String::as_str) {
        Some("ci") => ci(),
        Some("crash") => crash(&args[1..]),
        Some("parity") => parity(),
        Some("assets") => assets(),
        Some("lints") => lints(),
        _ => {
            eprintln!("usage: cargo xtask <ci|crash|parity|assets|lints>");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("\nxtask: FAILED: {e}");
            ExitCode::FAILURE
        }
    }
}

type Res = Result<(), String>;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives inside the workspace")
        .to_path_buf()
}

fn step(name: &str) {
    eprintln!("\n==> {name}");
}

fn cargo(args: &[&str], env: &[(&str, String)]) -> Res {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.args(args);
    for (k, v) in env {
        c.env(k, v);
    }
    let status = c
        .status()
        .map_err(|e| format!("could not run cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo {}` failed", args.join(" ")))
    }
}

fn ci() -> Res {
    step("1/7 formatting (cargo fmt --check)");
    cargo(&["fmt", "--all", "--check"], &[])?;
    step("2/7 clippy, warnings are errors, never-crash lints on");
    cargo(
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        &[],
    )?;
    step("3/7 every crate inherits the workspace lints");
    lints()?;
    step("4/7 tests");
    cargo(&["test", "--workspace"], &[])?;
    step("5/7 parity claims are backed by tests");
    parity()?;
    step("6/7 assets are attributed");
    assets()?;
    step("7/7 anti-crash gate (short run)");
    crash(&["--iters".into(), "3000".into()])?;
    eprintln!("\nxtask: all gates passed");
    Ok(())
}

/// The anti-crash gate: every `tests/crash.rs` in the workspace, in release
/// mode, with `FOLIO_CRASH_ITERS` mutants each.
fn crash(args: &[String]) -> Res {
    let mut iters = "20000".to_string();
    let mut log: Option<String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--iters" => iters = it.next().cloned().ok_or("--iters needs a number")?,
            "--log" => log = Some(it.next().cloned().ok_or("--log needs a stage name")?),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    iters
        .parse::<u64>()
        .map_err(|_| format!("--iters: `{iters}` is not a number"))?;
    eprintln!("anti-crash gate: {iters} mutants per crash test");
    cargo(
        &[
            "test",
            "--release",
            "--workspace",
            "--test",
            "crash",
            "--",
            "--nocapture",
        ],
        &[("FOLIO_CRASH_ITERS", iters.clone())],
    )?;
    if let Some(stage) = log {
        let path = Path::new("docs/crash-log.md");
        let mut text = std::fs::read_to_string(path).unwrap_or_default();
        text.push_str(&format!(
            "| {} | {stage} | {iters} | passed |\n",
            today_utc()
        ));
        std::fs::write(path, text).map_err(|e| e.to_string())?;
        eprintln!("recorded in docs/crash-log.md");
    }
    Ok(())
}

fn today_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Civil-from-days (Howard Hinnant).
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `parity/acrobat-features.toml` must list every feature of PLAN.md, use
/// only known statuses, and back `partial`/`shipped` with tests that exist.
fn parity() -> Res {
    let text = std::fs::read_to_string("parity/acrobat-features.toml")
        .map_err(|e| format!("parity/acrobat-features.toml: {e}"))?;
    let test_fns = all_fn_names();
    let mut errors = Vec::new();
    let mut ids = BTreeSet::new();
    let (mut shipped, mut partial, mut planned) = (0, 0, 0);
    for block in text.split("\n[[feature]]").skip(1) {
        let id = field(block, "id").unwrap_or_default();
        let status = field(block, "status").unwrap_or_default();
        if id.is_empty() {
            errors.push("a [[feature]] has no id".to_string());
            continue;
        }
        if !ids.insert(id.clone()) {
            errors.push(format!("{id}: listed twice"));
        }
        let evidence = list_field(block, "evidence");
        match status.as_str() {
            "planned" => planned += 1,
            "partial" | "shipped" => {
                if status == "shipped" {
                    shipped += 1;
                } else {
                    partial += 1;
                }
                if evidence.is_empty() {
                    errors.push(format!(
                        "{id}: `{status}` needs at least one test in `evidence`"
                    ));
                }
                for t in &evidence {
                    if !test_fns.contains(t) {
                        errors.push(format!("{id}: evidence test `{t}` does not exist"));
                    }
                }
                if field(block, "notes").is_none_or(|n| n.trim().is_empty()) {
                    errors.push(format!(
                        "{id}: `{status}` needs `notes` saying what is still missing"
                    ));
                }
            }
            "out-of-scope" => {}
            other => errors.push(format!(
                "{id}: unknown status `{other}` (planned, partial, shipped, out-of-scope)"
            )),
        }
    }
    // Every feature id in the plan has an entry.
    let plan = std::fs::read_to_string("PLAN.md").unwrap_or_default();
    for line in plan.lines() {
        if let Some(rest) = line.strip_prefix("| ") {
            let id = rest.split(" |").next().unwrap_or("").trim();
            if id.contains('.') && !id.contains(' ') && !ids.contains(id) {
                errors.push(format!(
                    "PLAN.md feature `{id}` is missing from the parity file"
                ));
            }
        }
    }
    if errors.is_empty() {
        eprintln!(
            "parity: {} features — {shipped} shipped, {partial} partial, {planned} planned",
            ids.len()
        );
        Ok(())
    } else {
        Err(format!("parity:\n  {}", errors.join("\n  ")))
    }
}

fn field(block: &str, key: &str) -> Option<String> {
    block.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

fn list_field(block: &str, key: &str) -> Vec<String> {
    field(block, key)
        .map(|v| {
            v.trim_matches(|c| c == '[' || c == ']')
                .split(',')
                .map(|s| s.trim().trim_matches('"').to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn all_fn_names() -> BTreeSet<String> {
    let mut files = Vec::new();
    rust_files(Path::new("crates"), &mut files);
    rust_files(Path::new("apps"), &mut files);
    let mut names = BTreeSet::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap_or_default();
        for part in text.split("fn ").skip(1) {
            let name: String = part
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                names.insert(name);
            }
        }
    }
    names
}

const ASSET_EXT: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "svg", "ico", "webp", "bmp", "tif", "tiff", "ttf", "otf", "woff",
    "woff2", "pdf", "icc", "icm", "wav", "mp3", "mp4",
];

/// Every tracked asset file needs an `ATTRIBUTION.toml` entry.
fn assets() -> Res {
    let out = Command::new("git")
        .args(["ls-files", "-z"])
        .output()
        .map_err(|e| format!("git is needed for the asset check: {e}"))?;
    let attribution = std::fs::read_to_string("ATTRIBUTION.toml").unwrap_or_default();
    let mut missing = Vec::new();
    let mut count = 0;
    for f in String::from_utf8_lossy(&out.stdout).split('\0') {
        let ext = Path::new(f)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if ASSET_EXT.contains(&ext.as_str()) {
            count += 1;
            if !attribution.contains(&format!("path = \"{f}\"")) {
                missing.push(f.to_string());
            }
        }
    }
    if missing.is_empty() {
        eprintln!("assets: {count} asset files, all attributed");
        Ok(())
    } else {
        Err(format!(
            "these assets have no ATTRIBUTION.toml entry (AGENTS.md §1):\n  {}",
            missing.join("\n  ")
        ))
    }
}

/// Every crate under crates/ and apps/ must use `[lints] workspace = true`
/// and must not switch the never-crash lints off at crate level.
fn lints() -> Res {
    let mut errors = Vec::new();
    for top in ["crates", "apps"] {
        let Ok(rd) = std::fs::read_dir(top) else {
            continue;
        };
        for e in rd.flatten() {
            let manifest = e.path().join("Cargo.toml");
            let Ok(text) = std::fs::read_to_string(&manifest) else {
                continue;
            };
            let compact: String = text.split_whitespace().collect();
            if !compact.contains("[lints]workspace=true") {
                errors.push(format!(
                    "{}: add `[lints] workspace = true`",
                    manifest.display()
                ));
            }
            for src in ["src/lib.rs", "src/main.rs"] {
                let p = e.path().join(src);
                let body = std::fs::read_to_string(&p).unwrap_or_default();
                for banned in [
                    "allow(clippy::unwrap_used",
                    "allow(clippy::expect_used",
                    "allow(clippy::panic",
                    "allow(unsafe_code",
                ] {
                    if body.contains(&format!("#![{banned}")) {
                        errors.push(format!(
                            "{}: crate-wide `{banned})` disables a never-crash rule",
                            p.display()
                        ));
                    }
                }
            }
        }
    }
    if errors.is_empty() {
        eprintln!("lints: every crate inherits the never-crash rules");
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}
