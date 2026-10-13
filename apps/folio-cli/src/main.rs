//! `folio` — the PDF workbench, headless. Every command can answer in JSON
//! (`--json`) so agents and scripts can drive it. Options follow Acrobat
//! Pro's dialogs (docs/acrobat/organize.md). Existing files are never
//! overwritten without `--force`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use folio_core::Document;
use folio_ops::json;
use folio_ops::labels::{self, LabelChange, Style};
use folio_ops::merge::{self, Input};
use folio_ops::pages::{self, Margins, Orientation, PageBox, Subset};
use folio_ops::split::{self, SplitBy};
use folio_ops::{compress, info, meta, ranges, text};

#[derive(Parser)]
#[command(
    name = "folio",
    version,
    about = "Folio: an open-source PDF workbench in pure Rust"
)]
struct Cli {
    /// Print machine-readable JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args)]
struct Out {
    /// Output file.
    #[arg(short, long)]
    output: PathBuf,
    /// Replace the output file if it exists.
    #[arg(long)]
    force: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Direction {
    /// 90° clockwise.
    Cw,
    /// 90° counterclockwise.
    Ccw,
    #[value(name = "180")]
    Half,
}

#[derive(Clone, Copy, ValueEnum)]
enum SubsetArg {
    All,
    Even,
    Odd,
}

#[derive(Clone, Copy, ValueEnum)]
enum OrientationArg {
    Any,
    Landscape,
    Portrait,
}

#[derive(Clone, Copy, ValueEnum)]
enum BoxArg {
    Crop,
    Art,
    Trim,
    Bleed,
}

#[derive(Clone, Copy, ValueEnum)]
enum Unit {
    Pt,
    In,
    Mm,
    Cm,
}

#[derive(Clone, Copy, ValueEnum)]
enum StyleArg {
    None,
    Decimal,
    /// i, ii, iii
    Roman,
    /// I, II, III
    #[value(name = "ROMAN")]
    RomanUpper,
    /// a, b, c
    Alpha,
    /// A, B, C
    #[value(name = "ALPHA")]
    AlphaUpper,
}

#[derive(Clone, Copy, ValueEnum)]
enum Encoding {
    Utf8,
    Utf16,
}

#[derive(Subcommand)]
enum Cmd {
    /// Open a PDF, report how healthy it is and what was repaired.
    Check {
        file: PathBuf,
        /// Also write the repaired file here.
        #[arg(long)]
        repair: Option<PathBuf>,
        /// Allow --repair to replace an existing file.
        #[arg(long)]
        force: bool,
    },
    /// Document Properties: description, advanced, pages, bookmarks, labels.
    Info { file: PathBuf },
    /// Combine Files. Inputs may carry a page range: `report.pdf:1-3,7`.
    Merge {
        #[command(flatten)]
        out: Out,
        /// Do not add a bookmark for each file.
        #[arg(long)]
        no_bookmarks: bool,
        #[arg(required = true)]
        inputs: Vec<String>,
    },
    /// Split Document by number of pages, file size, or top-level bookmarks.
    Split {
        file: PathBuf,
        /// Maximum pages per file.
        #[arg(long, group = "method")]
        pages_per_file: Option<usize>,
        /// Maximum file size in MB.
        #[arg(long, group = "method")]
        max_size: Option<f64>,
        /// One file per top-level bookmark.
        #[arg(long, group = "method")]
        bookmarks: bool,
        /// Folder for the parts (default: next to the file).
        #[arg(long)]
        out_dir: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    /// Extract Pages.
    Extract {
        file: PathBuf,
        #[arg(long)]
        pages: String,
        /// Output file (not used with --separate).
        #[arg(short, long, required_unless_present = "separate")]
        output: Option<PathBuf>,
        /// Extract pages as separate files: `<name> <page>.pdf`.
        #[arg(long)]
        separate: bool,
        /// Folder for --separate (default: next to the file).
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Delete pages after extracting (rewrites the source; needs --force).
        #[arg(long)]
        delete_after: bool,
        #[arg(long)]
        force: bool,
    },
    /// Delete Pages.
    Delete {
        file: PathBuf,
        #[arg(long)]
        pages: String,
        #[command(flatten)]
        out: Out,
    },
    /// Rotate Pages.
    Rotate {
        file: PathBuf,
        #[arg(long, value_enum, default_value = "cw")]
        direction: Direction,
        /// Page range (default: all).
        #[arg(long, default_value = "")]
        pages: String,
        #[arg(long, value_enum, default_value = "all")]
        subset: SubsetArg,
        #[arg(long, value_enum, default_value = "any")]
        orientation: OrientationArg,
        #[command(flatten)]
        out: Out,
    },
    /// Insert Pages from a file or blank pages.
    Insert {
        file: PathBuf,
        /// The file to insert from.
        #[arg(long, group = "source")]
        from: Option<PathBuf>,
        /// Pages of --from to insert (default: all).
        #[arg(long)]
        from_pages: Option<String>,
        /// Insert blank pages (same size as the neighbouring page).
        #[arg(long, group = "source")]
        blank: bool,
        /// How many blank pages.
        #[arg(long, default_value_t = 1)]
        count: usize,
        /// Insert before this page (number, `first` or `last`).
        #[arg(long, group = "place")]
        before: Option<String>,
        /// Insert after this page (number, `first` or `last`; default `last`).
        #[arg(long, group = "place")]
        after: Option<String>,
        #[command(flatten)]
        out: Out,
    },
    /// Replace Pages with pages of another file.
    Replace {
        file: PathBuf,
        /// Original pages to replace.
        #[arg(long)]
        pages: String,
        #[arg(long)]
        from: PathBuf,
        /// First replacement page in --from.
        #[arg(long, default_value_t = 1)]
        from_start: usize,
        #[command(flatten)]
        out: Out,
    },
    /// Reverse pages or give a new order.
    Reorder {
        file: PathBuf,
        #[arg(long, group = "how")]
        reverse: bool,
        /// Every page exactly once, e.g. `3,1,2`.
        #[arg(long, group = "how")]
        order: Option<String>,
        #[command(flatten)]
        out: Out,
    },
    /// Crop Pages (set page boxes).
    Crop {
        file: PathBuf,
        /// Top,Bottom,Left,Right — or one value for all four.
        #[arg(long, group = "how")]
        margins: Option<String>,
        /// Set to zero: remove the box.
        #[arg(long, group = "how")]
        reset: bool,
        #[arg(long = "box", value_enum, default_value = "crop")]
        which: BoxArg,
        #[arg(long, value_enum, default_value = "pt")]
        unit: Unit,
        #[arg(long, default_value = "")]
        pages: String,
        #[arg(long, value_enum, default_value = "all")]
        subset: SubsetArg,
        #[command(flatten)]
        out: Out,
    },
    /// Page Labels for a range of pages.
    Labels {
        file: PathBuf,
        /// A contiguous range, e.g. `1-4`.
        #[arg(long)]
        pages: String,
        #[arg(long, value_enum, group = "how")]
        style: Option<StyleArg>,
        #[arg(long, default_value = "")]
        prefix: String,
        #[arg(long, default_value_t = 1)]
        start: i64,
        /// Extend numbering used in the preceding section.
        #[arg(long, group = "how")]
        extend: bool,
        #[command(flatten)]
        out: Out,
    },
    /// Show or edit Title, Author, Subject, Keywords (Info and XMP).
    Meta {
        file: PathBuf,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        author: Option<String>,
        #[arg(long)]
        subject: Option<String>,
        #[arg(long)]
        keywords: Option<String>,
        /// Output file (required when editing).
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    /// Reduce File Size (streams and unused objects).
    Compress {
        file: PathBuf,
        #[command(flatten)]
        out: Out,
    },
    /// Save As Text (Plain). Pages are separated by a form feed.
    Text {
        file: PathBuf,
        #[arg(long, default_value = "")]
        pages: String,
        /// Output file (default: print).
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "utf8")]
        encoding: Encoding,
        #[arg(long)]
        force: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(e) => {
            if cli.json {
                println!(
                    "{}",
                    json::obj(&[("ok", "false".into()), ("error", json::s(&e))])
                );
            } else {
                eprintln!("error: {e}");
            }
            ExitCode::from(2)
        }
    }
}

type Res<T> = Result<T, String>;

fn load(p: &Path) -> Res<Document> {
    Document::load(p).map_err(|e| format!("{}: {e}", p.display()))
}

fn check_free(p: &Path, force: bool) -> Res<()> {
    if p.exists() && !force {
        return Err(format!(
            "{} already exists; pass --force to replace it",
            p.display()
        ));
    }
    Ok(())
}

fn save(doc: &Document, p: &Path, force: bool) -> Res<()> {
    check_free(p, force)?;
    doc.save(p).map_err(|e| format!("{}: {e}", p.display()))
}

fn e<T>(r: folio_core::Result<T>) -> Res<T> {
    r.map_err(|e| e.to_string())
}

fn subset(s: SubsetArg) -> Subset {
    match s {
        SubsetArg::All => Subset::All,
        SubsetArg::Even => Subset::Even,
        SubsetArg::Odd => Subset::Odd,
    }
}

fn stem(p: &Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".into())
}

fn dir_of(p: &Path, explicit: &Option<PathBuf>) -> PathBuf {
    explicit.clone().unwrap_or_else(|| {
        p.parent()
            .filter(|d| !d.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    })
}

/// Report a written document.
fn done(cli: &Cli, what: &str, out: &Path, doc: &Document, extra: &[(&str, String)]) -> ExitCode {
    let pages = doc.pages().map(|p| p.len()).unwrap_or(0);
    if cli.json {
        let mut f = vec![
            ("ok", "true".to_string()),
            ("output", json::s(&out.display().to_string())),
            ("pages", pages.to_string()),
        ];
        f.extend(extra.iter().map(|(k, v)| (*k, v.clone())));
        println!("{}", json::obj(&f));
    } else {
        println!("{what}: {} ({pages} pages)", out.display());
    }
    ExitCode::SUCCESS
}

/// `first`, `last` or a 1-based number → 0-based page index.
fn page_word(w: &str, total: usize) -> Res<usize> {
    match w.trim() {
        "first" => Ok(0),
        "last" => Ok(total.saturating_sub(1)),
        other => Ok(e(ranges::parse(other, total))?
            .first()
            .copied()
            .unwrap_or(0)),
    }
}

/// `path` or `path:range`, Windows drive letters included.
fn split_input(spec: &str) -> (PathBuf, Option<String>) {
    if Path::new(spec).exists() {
        return (PathBuf::from(spec), None);
    }
    match spec.rsplit_once(':') {
        Some((p, r)) if !p.is_empty() && !r.contains(['/', '\\']) => {
            (PathBuf::from(p), Some(r.to_string()))
        }
        _ => (PathBuf::from(spec), None),
    }
}

fn run(cli: &Cli) -> Res<ExitCode> {
    match &cli.cmd {
        Cmd::Check {
            file,
            repair,
            force,
        } => {
            let doc = load(file)?;
            let pages = e(doc.pages())?;
            if let Some(out) = repair {
                save(&doc, out, *force)?;
            }
            if cli.json {
                println!(
                    "{}",
                    json::obj(&[
                        ("ok", "true".into()),
                        ("version", json::s(&doc.version)),
                        ("pages", pages.len().to_string()),
                        ("objects", doc.objects.len().to_string()),
                        (
                            "repairs",
                            json::arr(&doc.repairs.iter().map(|r| json::s(r)).collect::<Vec<_>>()),
                        ),
                    ])
                );
            } else {
                println!("{}: PDF {}", file.display(), doc.version);
                println!("  pages:   {}", pages.len());
                println!("  objects: {}", doc.objects.len());
                if doc.repairs.is_empty() {
                    println!("  status:  healthy");
                } else {
                    println!("  status:  repaired");
                    for r in &doc.repairs {
                        println!("    - {r}");
                    }
                }
                if let Some(out) = repair {
                    println!("  written: {}", out.display());
                }
            }
            Ok(if doc.repairs.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }

        Cmd::Info { file } => {
            let doc = load(file)?;
            let size = std::fs::metadata(file).ok().map(|m| m.len());
            let i = e(info::info(&doc, size))?;
            if cli.json {
                println!("{}", i.to_json());
            } else {
                print!("{}", i.to_text());
            }
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Merge {
            out,
            no_bookmarks,
            inputs,
        } => {
            check_free(&out.output, out.force)?;
            let mut list = Vec::new();
            for spec in inputs {
                let (path, range) = split_input(spec);
                let doc = load(&path)?;
                let pages = match range {
                    Some(r) => {
                        let n = e(doc.pages())?.len();
                        Some(
                            e(ranges::parse(&r, n))
                                .map_err(|m| format!("{}: {m}", path.display()))?,
                        )
                    }
                    None => None,
                };
                list.push(Input {
                    doc,
                    name: stem(&path),
                    pages,
                });
            }
            let n = list.len();
            let doc = e(merge::merge(
                list,
                &merge::Options {
                    bookmarks: !no_bookmarks,
                },
            ))?;
            save(&doc, &out.output, out.force)?;
            Ok(done(
                cli,
                &format!("combined {n} files"),
                &out.output,
                &doc,
                &[],
            ))
        }

        Cmd::Split {
            file,
            pages_per_file,
            max_size,
            bookmarks,
            out_dir,
            force,
        } => {
            let doc = load(file)?;
            let by = match (pages_per_file, max_size, bookmarks) {
                (Some(n), _, _) => SplitBy::Pages(*n),
                (_, Some(mb), _) => {
                    if !mb.is_finite() || *mb <= 0.0 {
                        return Err("--max-size must be a positive number of MB".into());
                    }
                    SplitBy::MaxBytes((mb * 1024.0 * 1024.0) as u64)
                }
                (_, _, true) => SplitBy::Bookmarks,
                _ => {
                    return Err(
                        "choose how to split: --pages-per-file N, --max-size MB or --bookmarks"
                            .into(),
                    );
                }
            };
            let parts = e(split::split(&doc, by))?;
            let dir = dir_of(file, out_dir);
            let base = stem(file);
            let mut names = Vec::new();
            let mut used = std::collections::HashSet::new();
            for (k, p) in parts.iter().enumerate() {
                let mut name = match &p.title {
                    Some(t) => split::safe_file_name(t),
                    None => format!("{base}_Part{}", k + 1),
                };
                if !used.insert(name.clone()) {
                    name = format!("{name} ({})", k + 1);
                }
                names.push(dir.join(format!("{name}.pdf")));
            }
            for n in &names {
                check_free(n, *force)?;
            }
            for (p, n) in parts.iter().zip(&names) {
                save(&p.doc, n, *force)?;
            }
            if cli.json {
                let files: Vec<String> = names
                    .iter()
                    .zip(&parts)
                    .map(|(n, p)| {
                        json::obj(&[
                            ("file", json::s(&n.display().to_string())),
                            ("pages", p.pages.len().to_string()),
                        ])
                    })
                    .collect();
                println!(
                    "{}",
                    json::obj(&[("ok", "true".into()), ("files", json::arr(&files))])
                );
            } else {
                for (n, p) in names.iter().zip(&parts) {
                    println!("{} ({} pages)", n.display(), p.pages.len());
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Extract {
            file,
            pages: spec,
            output,
            separate,
            out_dir,
            delete_after,
            force,
        } => {
            let doc = load(file)?;
            let n = e(doc.pages())?.len();
            let idx = e(ranges::parse(spec, n))?;
            if *delete_after && !force {
                return Err(
                    "--delete-after rewrites the source file; pass --force to allow it".into(),
                );
            }
            let mut written = Vec::new();
            if *separate {
                let dir = dir_of(file, out_dir);
                let base = stem(file);
                let targets: Vec<(usize, PathBuf)> = idx
                    .iter()
                    .map(|&i| (i, dir.join(format!("{base} {}.pdf", i + 1))))
                    .collect();
                for (_, t) in &targets {
                    check_free(t, *force)?;
                }
                for (i, t) in targets {
                    let mut one = doc.clone();
                    e(pages::select(&mut one, &[i]))?;
                    save(&one, &t, *force)?;
                    written.push(t);
                }
            } else if let Some(out) = output {
                let mut part = doc.clone();
                e(pages::select(&mut part, &idx))?;
                save(&part, out, *force)?;
                written.push(out.clone());
            }
            if *delete_after {
                let mut rest = doc;
                e(pages::delete(&mut rest, &idx))?;
                save(&rest, file, true)?;
            }
            if cli.json {
                let files: Vec<String> = written
                    .iter()
                    .map(|w| json::s(&w.display().to_string()))
                    .collect();
                println!(
                    "{}",
                    json::obj(&[
                        ("ok", "true".into()),
                        ("files", json::arr(&files)),
                        ("deleted_from_source", delete_after.to_string()),
                    ])
                );
            } else {
                for w in &written {
                    println!("extracted: {}", w.display());
                }
                if *delete_after {
                    println!("deleted those pages from {}", file.display());
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Delete {
            file,
            pages: spec,
            out,
        } => {
            let mut doc = load(file)?;
            let n = e(doc.pages())?.len();
            let idx = e(ranges::parse(spec, n))?;
            e(pages::delete(&mut doc, &idx))?;
            save(&doc, &out.output, out.force)?;
            Ok(done(cli, "deleted pages", &out.output, &doc, &[]))
        }

        Cmd::Rotate {
            file,
            direction,
            pages: spec,
            subset: s,
            orientation,
            out,
        } => {
            let mut doc = load(file)?;
            let n = e(doc.pages())?.len();
            let idx = e(ranges::parse(spec, n))?;
            let o = match orientation {
                OrientationArg::Any => Orientation::Any,
                OrientationArg::Landscape => Orientation::Landscape,
                OrientationArg::Portrait => Orientation::Portrait,
            };
            let idx = e(pages::filter(&doc, &idx, subset(*s), o))?;
            let deg = match direction {
                Direction::Cw => 90,
                Direction::Ccw => -90,
                Direction::Half => 180,
            };
            let rotated = e(pages::rotate(&mut doc, &idx, deg))?;
            save(&doc, &out.output, out.force)?;
            Ok(done(
                cli,
                &format!("rotated {rotated} pages"),
                &out.output,
                &doc,
                &[("rotated", rotated.to_string())],
            ))
        }

        Cmd::Insert {
            file,
            from,
            from_pages,
            blank,
            count,
            before,
            after,
            out,
        } => {
            let mut doc = load(file)?;
            let n = e(doc.pages())?.len();
            let at = match (before, after) {
                (Some(b), _) => page_word(b, n)?,
                (_, Some(a)) => page_word(a, n)? + 1,
                _ => n,
            };
            if *blank {
                e(pages::insert_blank(&mut doc, at, *count))?;
            } else if let Some(src) = from {
                let other = load(src)?;
                let idx = match from_pages {
                    Some(r) => Some(e(ranges::parse(r, e(other.pages())?.len()))?),
                    None => None,
                };
                e(pages::insert(&mut doc, other, idx.as_deref(), at))?;
            } else {
                return Err("choose what to insert: --from FILE or --blank".into());
            }
            save(&doc, &out.output, out.force)?;
            Ok(done(cli, "inserted pages", &out.output, &doc, &[]))
        }

        Cmd::Replace {
            file,
            pages: spec,
            from,
            from_start,
            out,
        } => {
            let mut doc = load(file)?;
            let n = e(doc.pages())?.len();
            let idx = e(ranges::parse(spec, n))?;
            let other = load(from)?;
            if *from_start == 0 {
                return Err("--from-start counts from 1".into());
            }
            e(pages::replace(&mut doc, &idx, other, from_start - 1))?;
            save(&doc, &out.output, out.force)?;
            Ok(done(cli, "replaced pages", &out.output, &doc, &[]))
        }

        Cmd::Reorder {
            file,
            reverse,
            order,
            out,
        } => {
            let mut doc = load(file)?;
            let n = e(doc.pages())?.len();
            if *reverse {
                e(pages::reverse(&mut doc))?;
            } else if let Some(o) = order {
                e(pages::reorder(&mut doc, &e(ranges::parse(o, n))?))?;
            } else {
                return Err("choose --reverse or --order".into());
            }
            save(&doc, &out.output, out.force)?;
            Ok(done(cli, "reordered", &out.output, &doc, &[]))
        }

        Cmd::Crop {
            file,
            margins,
            reset,
            which,
            unit,
            pages: spec,
            subset: s,
            out,
        } => {
            let mut doc = load(file)?;
            let n = e(doc.pages())?.len();
            let idx = e(ranges::parse(spec, n))?;
            let idx = e(pages::filter(&doc, &idx, subset(*s), Orientation::Any))?;
            let b = match which {
                BoxArg::Crop => PageBox::Crop,
                BoxArg::Art => PageBox::Art,
                BoxArg::Trim => PageBox::Trim,
                BoxArg::Bleed => PageBox::Bleed,
            };
            let changed = if *reset {
                e(pages::reset_box(&mut doc, &idx, b))?
            } else if let Some(m) = margins {
                let k = match unit {
                    Unit::Pt => 1.0,
                    Unit::In => 72.0,
                    Unit::Mm => 72.0 / 25.4,
                    Unit::Cm => 72.0 / 2.54,
                };
                let v: Vec<f64> = m
                    .split(',')
                    .map(|x| x.trim().parse::<f64>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| "--margins takes numbers: T,B,L,R or one value".to_string())?;
                let m = match v.as_slice() {
                    [a] => Margins {
                        top: a * k,
                        bottom: a * k,
                        left: a * k,
                        right: a * k,
                    },
                    [t, b, l, r] => Margins {
                        top: t * k,
                        bottom: b * k,
                        left: l * k,
                        right: r * k,
                    },
                    _ => return Err("--margins takes 1 or 4 numbers: T,B,L,R".into()),
                };
                e(pages::crop(&mut doc, &idx, b, m))?
            } else {
                return Err("choose --margins T,B,L,R or --reset".into());
            };
            save(&doc, &out.output, out.force)?;
            Ok(done(
                cli,
                &format!("set {} on {changed} pages", b.key()),
                &out.output,
                &doc,
                &[("changed", changed.to_string())],
            ))
        }

        Cmd::Labels {
            file,
            pages: spec,
            style,
            prefix,
            start,
            extend,
            out,
        } => {
            let mut doc = load(file)?;
            let n = e(doc.pages())?.len();
            let set = e(ranges::parse_set(spec, n))?;
            let (Some(&first), Some(&last)) = (set.first(), set.last()) else {
                return Err("--pages selects no pages".into());
            };
            if last - first + 1 != set.len() {
                return Err("--pages must be one contiguous range, like 3-7".into());
            }
            let change = if *extend {
                LabelChange::Extend
            } else {
                let style = match style.unwrap_or(StyleArg::Decimal) {
                    StyleArg::None => Style::None,
                    StyleArg::Decimal => Style::Decimal,
                    StyleArg::Roman => Style::RomanLower,
                    StyleArg::RomanUpper => Style::RomanUpper,
                    StyleArg::Alpha => Style::AlphaLower,
                    StyleArg::AlphaUpper => Style::AlphaUpper,
                };
                LabelChange::NewSection {
                    style,
                    prefix: prefix.clone(),
                    start: *start,
                }
            };
            e(labels::apply(&mut doc, first, last, &change))?;
            save(&doc, &out.output, out.force)?;
            Ok(done(cli, "labelled pages", &out.output, &doc, &[]))
        }

        Cmd::Meta {
            file,
            title,
            author,
            subject,
            keywords,
            output,
            force,
        } => {
            let mut doc = load(file)?;
            let u = meta::Update {
                title: title.clone(),
                author: author.clone(),
                subject: subject.clone(),
                keywords: keywords.clone(),
            };
            if u.is_empty() {
                let m = meta::read(&doc);
                if cli.json {
                    println!(
                        "{}",
                        json::obj(&[
                            ("ok", "true".into()),
                            ("title", json::opt(m.title.as_deref())),
                            ("author", json::opt(m.author.as_deref())),
                            ("subject", json::opt(m.subject.as_deref())),
                            ("keywords", json::opt(m.keywords.as_deref())),
                            ("application", json::opt(m.creator.as_deref())),
                            ("producer", json::opt(m.producer.as_deref())),
                            ("created", json::opt(m.created.as_deref())),
                            ("modified", json::opt(m.modified.as_deref())),
                        ])
                    );
                } else {
                    let d = |v: &Option<String>| v.clone().unwrap_or_else(|| "—".into());
                    println!("Title:    {}", d(&m.title));
                    println!("Author:   {}", d(&m.author));
                    println!("Subject:  {}", d(&m.subject));
                    println!("Keywords: {}", d(&m.keywords));
                }
                return Ok(ExitCode::SUCCESS);
            }
            let Some(out) = output else {
                return Err("editing metadata needs -o/--output".into());
            };
            e(meta::update(&mut doc, &u, std::time::SystemTime::now()))?;
            save(&doc, out, *force)?;
            Ok(done(cli, "updated properties", out, &doc, &[]))
        }

        Cmd::Compress { file, out } => {
            let before = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
            let mut doc = load(file)?;
            let r = compress::compress(&mut doc);
            save(&doc, &out.output, out.force)?;
            let after = std::fs::metadata(&out.output).map(|m| m.len()).unwrap_or(0);
            if cli.json {
                println!(
                    "{}",
                    json::obj(&[
                        ("ok", "true".into()),
                        ("output", json::s(&out.output.display().to_string())),
                        ("bytes_before", before.to_string()),
                        ("bytes_after", after.to_string()),
                        ("streams_compressed", r.streams_compressed.to_string()),
                        ("objects_removed", r.objects_removed.to_string()),
                    ])
                );
            } else {
                println!(
                    "{}: {} → {} ({} streams compressed, {} unused objects removed)",
                    out.output.display(),
                    info::human_size(before),
                    info::human_size(after),
                    r.streams_compressed,
                    r.objects_removed
                );
            }
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Text {
            file,
            pages: spec,
            output,
            encoding,
            force,
        } => {
            let doc = load(file)?;
            let list = e(doc.pages())?;
            let idx = e(ranges::parse(spec, list.len()))?;
            let mut parts = Vec::new();
            for i in idx {
                if let Some(&p) = list.get(i) {
                    parts.push(e(text::page_text(&doc, p))?);
                }
            }
            let all = parts.join("\n\u{c}\n");
            match output {
                Some(out) => {
                    check_free(out, *force)?;
                    let bytes = match encoding {
                        Encoding::Utf8 => all.clone().into_bytes(),
                        Encoding::Utf16 => {
                            let mut b = vec![0xFF, 0xFE];
                            for u in all.encode_utf16() {
                                b.extend_from_slice(&u.to_le_bytes());
                            }
                            b
                        }
                    };
                    std::fs::write(out, bytes).map_err(|e| format!("{}: {e}", out.display()))?;
                    if cli.json {
                        println!(
                            "{}",
                            json::obj(&[
                                ("ok", "true".into()),
                                ("output", json::s(&out.display().to_string())),
                                ("pages", parts.len().to_string()),
                            ])
                        );
                    } else {
                        println!("text of {} pages: {}", parts.len(), out.display());
                    }
                }
                None => {
                    if cli.json {
                        let pages: Vec<String> = parts.iter().map(|p| json::s(p)).collect();
                        println!(
                            "{}",
                            json::obj(&[("ok", "true".into()), ("pages", json::arr(&pages))])
                        );
                    } else {
                        println!("{all}");
                    }
                }
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}
