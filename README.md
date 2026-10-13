<h1 align="center">Folio</h1>

<p align="center">
  <b>The open PDF workbench — a clean-room alternative to Adobe Acrobat Pro, written from scratch in pure Rust.</b><br>
  Read, repair, organize, comment, fill, sign and protect PDFs. No account, no telemetry.<br>
  Windows · macOS · Linux
</p>

<p align="center">
  <img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-2b6cb0">
  <img alt="Written in Rust" src="https://img.shields.io/badge/written%20in-Rust-b7410e">
  <img alt="No unsafe" src="https://img.shields.io/badge/unsafe-forbidden-2f855a">
  <a href="https://github.com/zolyy/folio/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/zolyy/folio/actions/workflows/ci.yml/badge.svg"></a>
</p>

> Folio is an independent project and is not affiliated with Adobe. "Acrobat" is used
> only as the reference for which features to build.

## Status

Folio is being built in public, stage by stage — see **[PLAN.md](PLAN.md)** for the full
list of Acrobat Pro features and the order they arrive in, and
**[parity/acrobat-features.toml](parity/acrobat-features.toml)** for exactly what works today.

| Stage | Scope | State |
|---|---|---|
| 0 | Foundation: PDF engine (lenient parser, repair, filters, writer), verification agent | ✅ |
| 1 | Organize pages + command line (merge, split, rotate, extract, text, metadata…) | next |
| 2 | Security (decrypt / encrypt) | planned |
| 3 | Rendering engine | planned |
| 4 | Desktop app | planned |
| 5–16 | Comments, forms, editing, convert, signatures, redaction, OCR, optimize, PDF/A, tools, print, installers | planned |

## Try it

```bash
git clone https://github.com/zolyy/folio
cd folio
cargo run --release -p folio-cli -- check your.pdf
cargo run --release -p folio-cli -- check broken.pdf --repair fixed.pdf
cargo run --release -p folio-cli -- --json check your.pdf
```

## How it's built

- **`folio-core`** — the PDF object model, a tolerant lexer/parser, cross-reference
  tables *and* streams, object streams, incremental updates, rebuild-by-scanning for
  damaged files, Flate/LZW/ASCII85/ASCIIHex/RunLength with PNG/TIFF predictors, and a
  writer that always emits a clean, repaired file atomically.
- **Never crash.** `unsafe` is forbidden; `unwrap`/`expect`/`panic!` are denied outside
  tests; every stage closes with an **anti-crash gate** that runs every operation on tens
  of thousands of corrupted documents.
- **Headless first.** Every feature is a `folio` command with `--json` output, so scripts
  and AI agents can drive it.

## Contributing

Contributions from people and AI agents are welcome. Everything passes the same
**verification agent** — `cargo xtask ci` locally and in CI, DCO sign-off, a PR checklist,
and an automated review against [AGENTS.md](AGENTS.md). Start with
[CONTRIBUTING.md](CONTRIBUTING.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
