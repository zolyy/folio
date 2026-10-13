# Contributing to Folio

Thank you for helping build an open PDF workbench. Every contribution — from a person or
an AI agent — goes through the same verification before it reaches `main`.

## 1. Before you start

- Read [`AGENTS.md`](AGENTS.md) (rules) and [`PLAN.md`](PLAN.md) (what is being built, stage by stage).
- Pick a feature id from `PLAN.md` §1 or an open issue. Say in the issue that you are on it.

## 2. While you work

```bash
cargo build
cargo test -p folio-core          # the crate you changed
cargo run -p folio-cli -- check some.pdf
```

- No `unwrap`/`expect`/`panic!` outside tests, no `unsafe`, no indexing with numbers that
  came from a file. Clippy enforces most of it.
- New operation → add it to the crate's `tests/crash.rs` so the anti-crash gate covers it.
- New feature → update `parity/acrobat-features.toml` (`partial` or `shipped`, with test
  names in `evidence` and honest `notes`).
- Assets → `ATTRIBUTION.toml` entry, licence on the allow-list (`AGENTS.md` §1).

## 3. Verify locally — the same gates CI runs

```bash
cargo xtask ci
```

| Gate | What it checks |
|---|---|
| fmt | `cargo fmt --check` |
| clippy | warnings are errors; never-crash lints on |
| lints | every crate inherits the workspace lints |
| tests | the whole workspace, on Linux, Windows and macOS in CI |
| parity | every `partial`/`shipped` claim names tests that exist |
| assets | every image/font/PDF in git has an attribution entry |
| anti-crash | mutated documents never panic or hang |

## 4. Commit and open a pull request

- Sign off every commit: `git commit -s` (Developer Certificate of Origin, https://developercertificate.org/).
- Base on `main`, keep the PR focused, fill in the template. Unchecked boxes fail the
  `pr-checklist` check — delete lines that truly do not apply.

## 5. What happens next

1. **CI** runs `cargo xtask ci` and the tests on three operating systems.
2. **DCO** checks the sign-offs.
3. **The review agent** (Claude) reads the diff against `AGENTS.md` and comments. For PRs
   from forks a maintainer starts it by commenting `/agent-review`.
4. A maintainer reviews and merges once every check is green.

## Licence

By contributing you agree your work is licensed under `MIT OR Apache-2.0`.
