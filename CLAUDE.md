# CLAUDE.md — working notes for agents

Read `AGENTS.md` first; it wins over this file. The plan and stages are in `PLAN.md`.

## Layout

- `crates/folio-core` — object model (`object.rs`), lexer, parser, xref loading and
  repair (`xref.rs`), filters, writer, `Document` (page tree, import, prune, atomic save),
  `crashgen` (samples + mutator for the anti-crash gate).
- `apps/folio-cli` — the `folio` binary. Every command supports `--json`.
- `xtask` — the verification agent (`cargo xtask ci|crash|parity|assets|lints`).
- `parity/acrobat-features.toml` — the honest feature ledger.

## Before you finish any change

```bash
cargo xtask ci
```

That is: fmt, clippy `-D warnings` with the never-crash lints, lint inheritance, tests,
parity, assets, and a short anti-crash run. At the end of a stage run the full gate and
record it: `cargo xtask crash --iters 20000 --log "stage N"`.

## Conventions

- Errors: `folio_core::Error`; messages say what was wrong and what is allowed.
- Lenient parsing records what it fixed in `Document::repairs`, in plain words.
- Tests may `unwrap`. Integration test files start with
  `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`.
- New operation? Add it to the crate's `tests/crash.rs` exerciser too.
- Rust edition 2024 (the newest stable one).
