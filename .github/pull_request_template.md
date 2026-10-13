<!--
Thanks for contributing to Folio. Base this on `main`.
Every box must be checked before merge (the `pr-checklist` check enforces it).
Delete a line only if it truly does not apply to this change.
-->

## What this changes

<!-- What is different afterwards and why. `Fixes #123` if it closes an issue.
     Which PLAN.md feature id(s) does it advance? -->

## How you verified it

- [ ] `cargo xtask ci` passes locally
- [ ] New behaviour has tests; a bug fix has a test I saw fail before the fix

## Never-crash (AGENTS.md §3)

- [ ] No new `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!` outside tests, and no `unsafe`
- [ ] No indexing or slicing with input-derived numbers; arithmetic checked; allocations capped; recursion and loops bounded
- [ ] New operations are added to the crate's `tests/crash.rs`

## Provenance (AGENTS.md §1–2)

- [ ] No asset from an Adobe product; no Adobe icon recreated or imitated
- [ ] No GPL/AGPL code copied; behaviour comes from specifications or black-box observation
- [ ] Every new asset has an `ATTRIBUTION.toml` entry
- [ ] Every commit is signed off (`git commit -s`)

## Parity and reach

- [ ] `parity/acrobat-features.toml` updated, with honest `notes`
- [ ] Reachable from the `folio` command line with `--json`, where it applies

## Anything a reviewer should look at first

<!-- The judgement call you are least sure about. -->
