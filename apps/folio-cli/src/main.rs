//! `folio` — the PDF workbench, headless. Every command can answer in JSON
//! (`--json`) so agents and scripts can drive it. Options follow Acrobat
//! Pro's dialogs (docs/acrobat/organize.md). Existing files are never
//! overwritten without `--force`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use folio_core::Document;
use folio_core::crypt::{Changes, Compat, Need, Printing, Scope, Settings};
use folio_ops::json;
use folio_ops::labels::{self, LabelChange, Style};
use folio_ops::merge::{self, Input};
use folio_ops::pages::{self, Margins, Orientation, PageBox, Subset};
use folio_ops::split::{self, SplitBy};
use folio_ops::{compress, info, meta, ranges, security, text};

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
    /// Password for protected PDFs (open or permissions password).
    /// Prefer the FOLIO_PASSWORD environment variable: it stays out of shell history.
    #[arg(long, global = true, env = "FOLIO_PASSWORD", hide_env_values = true)]
    password: Option<String>,
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

#[derive(Clone, Copy, ValueEnum)]
enum CompatArg {
    /// 40-bit RC4.
    Acrobat3,
    /// 128-bit RC4.
    Acrobat5,
    /// 128-bit RC4, allows clear metadata.
    Acrobat6,
    /// 128-bit AES, allows encrypting only attachments.
    Acrobat7,
    /// 256-bit AES (default).
    Acrobatx,
}

#[derive(Clone, Copy, ValueEnum)]
enum ScopeArg {
    All,
    ExceptMetadata,
    Attachments,
}

#[derive(Clone, Copy, ValueEnum)]
enum PrintingArg {
    None,
    Low,
    High,
}

#[derive(Clone, Copy, ValueEnum)]
enum ChangesArg {
    None,
    /// Inserting, deleting, and rotating pages.
    Pages,
    /// Filling in form fields and signing existing signature fields.
    Forms,
    /// Commenting, filling in form fields, and signing.
    Comments,
    /// Any except extracting pages.
    Any,
}

#[derive(Clone, Copy, ValueEnum)]
enum FormatArg {
    Png,
    Jpeg,
    Tiff,
}

#[derive(Clone, Copy, ValueEnum)]
enum ColorspaceArg {
    Auto,
    Rgb,
    Gray,
    Cmyk,
    Mono,
}

#[derive(Clone, Copy, ValueEnum)]
enum QualityArg {
    Maximum,
    High,
    Medium,
    Low,
    Minimum,
}

#[derive(Clone, Copy, ValueEnum)]
enum JpegModeArg {
    Baseline,
    Optimized,
    Progressive,
}

#[derive(Clone, Copy, ValueEnum)]
enum PngFilterArg {
    None,
    Sub,
    Up,
    Average,
    Paeth,
    Adaptive,
}

#[derive(Clone, Copy, ValueEnum)]
enum TiffArg {
    None,
    Lzw,
    Zip,
    G4,
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
        /// Remove white margins: crop each page to its visible content.
        #[arg(long, group = "how")]
        remove_white_margins: bool,
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
    /// Find text (Acrobat's Find): every match with its page and position.
    Find {
        file: PathBuf,
        /// The words or phrase to find.
        query: String,
        /// Whole words only.
        #[arg(long)]
        whole_words: bool,
        /// Case-sensitive.
        #[arg(long)]
        case_sensitive: bool,
        /// Page range to search (default: all).
        #[arg(long, default_value = "")]
        pages: String,
    },
    /// Export pages as images (Export PDF → Image): PNG, JPEG or TIFF.
    /// Files are `<name>_Page_<n>.<ext>`.
    Render {
        file: PathBuf,
        /// Page range (default: all).
        #[arg(long, default_value = "")]
        pages: String,
        /// Resolution in pixels per inch (Acrobat offers 72–2400).
        #[arg(long, default_value_t = 150.0)]
        dpi: f64,
        /// Output file for a single page (otherwise files go to --out-dir).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Folder for the images (default: next to the file).
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Turn off "Smooth line art".
        #[arg(long)]
        no_smooth_line_art: bool,
        /// Turn off "Enhance thin lines".
        #[arg(long)]
        no_enhance_thin_lines: bool,
        /// Turn off "Smooth images".
        #[arg(long)]
        no_smooth_images: bool,
        /// Turn off "Smooth text".
        #[arg(long)]
        no_smooth_text: bool,
        /// Do not draw missing fonts with installed substitutes.
        #[arg(long)]
        no_system_fonts: bool,
        /// Do not draw comments, stamps and form fields.
        #[arg(long)]
        no_annotations: bool,
        /// Hide these layers (names, comma-separated), on top of the default state.
        #[arg(long, default_value = "")]
        hide_layers: String,
        /// Show these layers (names, comma-separated), on top of the default state.
        #[arg(long, default_value = "")]
        show_layers: String,
        /// Transparent instead of white page background (PNG only).
        #[arg(long)]
        transparent: bool,
        #[arg(long, value_enum, default_value = "png")]
        format: FormatArg,
        /// Colour space ("Determine automatically" picks grayscale for pages without colour).
        #[arg(long, value_enum, default_value = "auto")]
        colorspace: ColorspaceArg,
        /// JPEG quality.
        #[arg(long, value_enum, default_value = "high")]
        quality: QualityArg,
        /// JPEG format.
        #[arg(long, value_enum, default_value = "baseline")]
        jpeg_mode: JpegModeArg,
        /// Number of scans for progressive JPEG (3, 4 or 5).
        #[arg(long, default_value_t = 3)]
        scans: u8,
        #[arg(long, value_enum, default_value = "adaptive")]
        png_filter: PngFilterArg,
        #[arg(long, value_enum, default_value = "lzw")]
        tiff_compression: TiffArg,
        #[arg(long)]
        force: bool,
    },
    /// List the document's layers (optional content), as Acrobat's Layers panel.
    Layers { file: PathBuf },
    /// List and save file attachments (Acrobat's Attachments panel).
    Attachments {
        file: PathBuf,
        /// Save the attachment with this number (from the list) or name.
        #[arg(long)]
        save: Option<String>,
        /// Where to save it (default: its own file name, next to the PDF).
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    /// List the fonts used, as the Fonts tab of Document Properties.
    Fonts { file: PathBuf },
    /// Show the security settings and the Document Restrictions Summary.
    Security { file: PathBuf },
    /// Encrypt with Password (Acrobat's Password Security - Settings).
    Encrypt {
        file: PathBuf,
        #[arg(long, value_enum, default_value = "acrobatx")]
        compat: CompatArg,
        /// Which components to encrypt.
        #[arg(long = "encrypt", value_enum, default_value = "all")]
        scope: ScopeArg,
        /// Require a password to open the document.
        #[arg(long, env = "FOLIO_OPEN_PASSWORD", hide_env_values = true)]
        open_password: Option<String>,
        /// Permissions password (required for any restriction).
        #[arg(long, env = "FOLIO_PERMISSIONS_PASSWORD", hide_env_values = true)]
        permissions_password: Option<String>,
        #[arg(long, value_enum, default_value = "high")]
        printing: PrintingArg,
        #[arg(long, value_enum, default_value = "any")]
        changes: ChangesArg,
        /// Disable copying of text, images, and other content.
        #[arg(long)]
        no_copy: bool,
        /// Disable text access for screen reader devices.
        #[arg(long)]
        no_screen_reader: bool,
        #[command(flatten)]
        out: Out,
    },
    /// Remove security (needs the permissions password if one is set).
    Decrypt {
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

static PASSWORD: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn main() -> ExitCode {
    let cli = Cli::parse();
    let _ = PASSWORD.set(cli.password.clone().unwrap_or_default());
    match run(&cli) {
        Ok(code) => code,
        Err(f) => {
            if cli.json {
                println!(
                    "{}",
                    json::obj(&[
                        ("ok", "false".into()),
                        ("error", json::s(&f.msg)),
                        ("kind", json::s(f.kind())),
                    ])
                );
            } else {
                eprintln!("error: {}", f.msg);
            }
            ExitCode::from(f.code)
        }
    }
}

/// A failure and its exit code: 2 = usage or file problem, 3 = password or
/// permission (the document's security said no).
struct Fail {
    msg: String,
    code: u8,
}

impl Fail {
    fn kind(&self) -> &'static str {
        if self.code == 3 { "security" } else { "error" }
    }
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}

impl From<String> for Fail {
    fn from(msg: String) -> Self {
        Fail { msg, code: 2 }
    }
}

impl From<&str> for Fail {
    fn from(msg: &str) -> Self {
        Fail {
            msg: msg.into(),
            code: 2,
        }
    }
}

fn from_core(e: folio_core::Error, context: Option<&Path>) -> Fail {
    let code = match e {
        folio_core::Error::Password(_) | folio_core::Error::Permission(_) => 3,
        _ => 2,
    };
    let msg = match context {
        Some(p) => format!("{}: {e}", p.display()),
        None => e.to_string(),
    };
    Fail { msg, code }
}

type Res<T> = Result<T, Fail>;

fn load(p: &Path) -> Res<Document> {
    let pw = PASSWORD.get().map(String::as_str).unwrap_or("");
    Document::load_with_password(p, pw).map_err(|e| from_core(e, Some(p)))
}

/// Refuse what the document's security does not allow.
fn need(doc: &Document, n: Need, p: &Path) -> Res<()> {
    doc.require(n).map_err(|e| from_core(e, Some(p)))
}

fn check_free(p: &Path, force: bool) -> Res<()> {
    if p.exists() && !force {
        return Err(format!("{} already exists; pass --force to replace it", p.display()).into());
    }
    Ok(())
}

fn save(doc: &Document, p: &Path, force: bool) -> Res<()> {
    check_free(p, force)?;
    doc.save(p).map_err(|e| from_core(e, Some(p)))
}

fn e<T>(r: folio_core::Result<T>) -> Res<T> {
    r.map_err(|e| from_core(e, None))
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
                need(&doc, Need::Extract, &path)?;
                let pages = match range {
                    Some(r) => {
                        let n = e(doc.pages())?.len();
                        Some(
                            e(ranges::parse(&r, n))
                                .map_err(|m| Fail::from(format!("{}: {m}", path.display())))?,
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
            need(&doc, Need::Extract, file)?;
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
            need(&doc, Need::Extract, file)?;
            if *delete_after {
                need(&doc, Need::Assemble, file)?;
            }
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
            need(&doc, Need::Assemble, file)?;
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
            need(&doc, Need::Assemble, file)?;
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
            need(&doc, Need::Assemble, file)?;
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
                need(&other, Need::Extract, src)?;
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
            need(&doc, Need::Assemble, file)?;
            let n = e(doc.pages())?.len();
            let idx = e(ranges::parse(spec, n))?;
            let other = load(from)?;
            need(&other, Need::Extract, from)?;
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
            need(&doc, Need::Assemble, file)?;
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
            remove_white_margins,
            which,
            unit,
            pages: spec,
            subset: s,
            out,
        } => {
            let mut doc = load(file)?;
            need(&doc, Need::Assemble, file)?;
            let n = e(doc.pages())?.len();
            let idx = e(ranges::parse(spec, n))?;
            let idx = e(pages::filter(&doc, &idx, subset(*s), Orientation::Any))?;
            let b = match which {
                BoxArg::Crop => PageBox::Crop,
                BoxArg::Art => PageBox::Art,
                BoxArg::Trim => PageBox::Trim,
                BoxArg::Bleed => PageBox::Bleed,
            };
            let changed = if *remove_white_margins {
                let list = e(doc.pages())?;
                let opts = folio_render::Options {
                    dpi: 72.0,
                    annotations: false,
                    ..folio_render::Options::default()
                };
                let mut n = 0;
                for &i in &idx {
                    let Some(&page) = list.get(i) else { continue };
                    let r = e(folio_render::render_page(&doc, page, &opts))?;
                    if let Some(m) = white_margins(&r) {
                        n += e(pages::crop(&mut doc, &[i], b, m))?;
                    }
                }
                n
            } else if *reset {
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
            need(&doc, Need::Assemble, file)?;
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
            need(&doc, Need::Modify, file)?;
            e(meta::update(&mut doc, &u, std::time::SystemTime::now()))?;
            save(&doc, out, *force)?;
            Ok(done(cli, "updated properties", out, &doc, &[]))
        }

        Cmd::Compress { file, out } => {
            let before = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
            let mut doc = load(file)?;
            need(&doc, Need::Modify, file)?;
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

        Cmd::Find {
            file,
            query,
            whole_words,
            case_sensitive,
            pages: spec,
        } => {
            let doc = load(file)?;
            // Finding reads the text, like copying it (Folio choice).
            need(&doc, Need::Copy, file)?;
            let list = e(doc.pages())?;
            let idx = e(ranges::parse(spec, list.len()))?;
            let opts = folio_render::text::FindOptions {
                whole_words: *whole_words,
                case_sensitive: *case_sensitive,
            };
            let mut hits = Vec::new();
            for i in idx {
                let Some(&page) = list.get(i) else { continue };
                let t = e(folio_render::page_text(&doc, page))?;
                for r in folio_render::text::find(&t, query, opts) {
                    let found = folio_render::text::slice(&t, r.clone());
                    // A little context on each side, on one line.
                    let before = folio_render::text::slice(&t, r.start.saturating_sub(30)..r.start);
                    let after = folio_render::text::slice(&t, r.end..r.end + 30);
                    let quads = folio_render::text::quads(&t, r);
                    hits.push((i + 1, found, before, after, quads));
                }
            }
            if cli.json {
                let items: Vec<String> = hits
                    .iter()
                    .map(|(p, f, b, a, q)| {
                        let rects: Vec<String> = q
                            .iter()
                            .map(|q| {
                                let xs = q.iter().map(|v| v[0]);
                                let ys = q.iter().map(|v| v[1]);
                                json::arr(&[
                                    json::n(f64::from(xs.clone().fold(f32::MAX, f32::min))),
                                    json::n(f64::from(ys.clone().fold(f32::MAX, f32::min))),
                                    json::n(f64::from(xs.fold(f32::MIN, f32::max))),
                                    json::n(f64::from(ys.fold(f32::MIN, f32::max))),
                                ])
                            })
                            .collect();
                        json::obj(&[
                            ("page", p.to_string()),
                            ("text", json::s(f)),
                            ("before", json::s(b)),
                            ("after", json::s(a)),
                            ("rects", json::arr(&rects)),
                        ])
                    })
                    .collect();
                println!(
                    "{}",
                    json::obj(&[
                        ("ok", "true".into()),
                        ("matches", hits.len().to_string()),
                        ("results", json::arr(&items)),
                    ])
                );
            } else {
                let flat = |s: &str| s.replace(['\n', '\r'], " ");
                for (p, f, b, a, _) in &hits {
                    println!("page {p}: …{}[{}]{}…", flat(b), flat(f), flat(a));
                }
                println!(
                    "{} match{}",
                    hits.len(),
                    if hits.len() == 1 { "" } else { "es" }
                );
            }
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Render {
            file,
            pages: spec,
            dpi,
            output,
            out_dir,
            no_smooth_line_art,
            no_enhance_thin_lines,
            no_smooth_images,
            no_smooth_text,
            no_system_fonts,
            no_annotations,
            hide_layers,
            show_layers,
            transparent,
            format,
            colorspace,
            quality,
            jpeg_mode,
            scans,
            png_filter,
            tiff_compression,
            force,
        } => {
            use folio_render::export;
            if *transparent && !matches!(format, FormatArg::Png) {
                return Err("--transparent works with PNG only".into());
            }
            let fmt = match format {
                FormatArg::Png => export::Format::Png(match png_filter {
                    PngFilterArg::None => export::PngFilter::None,
                    PngFilterArg::Sub => export::PngFilter::Sub,
                    PngFilterArg::Up => export::PngFilter::Up,
                    PngFilterArg::Average => export::PngFilter::Average,
                    PngFilterArg::Paeth => export::PngFilter::Paeth,
                    PngFilterArg::Adaptive => export::PngFilter::Adaptive,
                }),
                FormatArg::Jpeg => export::Format::Jpeg(
                    match quality {
                        QualityArg::Maximum => export::Quality::Maximum,
                        QualityArg::High => export::Quality::High,
                        QualityArg::Medium => export::Quality::Medium,
                        QualityArg::Low => export::Quality::Low,
                        QualityArg::Minimum => export::Quality::Minimum,
                    },
                    match jpeg_mode {
                        JpegModeArg::Baseline => export::JpegMode::Baseline,
                        JpegModeArg::Optimized => export::JpegMode::Optimized,
                        JpegModeArg::Progressive => export::JpegMode::Progressive(*scans),
                    },
                ),
                FormatArg::Tiff => export::Format::Tiff(match tiff_compression {
                    TiffArg::None => export::TiffCompression::None,
                    TiffArg::Lzw => export::TiffCompression::Lzw,
                    TiffArg::Zip => export::TiffCompression::Zip,
                    TiffArg::G4 => export::TiffCompression::CcittG4,
                }),
            };
            let color = match colorspace {
                ColorspaceArg::Auto => export::ColorMode::Auto,
                ColorspaceArg::Rgb => export::ColorMode::Rgb,
                ColorspaceArg::Gray => export::ColorMode::Gray,
                ColorspaceArg::Cmyk => export::ColorMode::Cmyk,
                ColorspaceArg::Mono => export::ColorMode::Mono,
            };
            let ext = fmt.extension();
            let doc = load(file)?;
            // Exporting page images extracts content, like copying (Folio choice).
            need(&doc, Need::Copy, file)?;
            let list = e(doc.pages())?;
            let idx = e(ranges::parse(spec, list.len()))?;
            if output.is_some() && idx.len() != 1 {
                return Err(
                    "-o/--output takes a single page; use --out-dir for several pages".into(),
                );
            }
            let opts = folio_render::Options {
                dpi: *dpi,
                smooth_line_art: !no_smooth_line_art,
                enhance_thin_lines: !no_enhance_thin_lines,
                smooth_images: !no_smooth_images,
                smooth_text: !no_smooth_text,
                system_fonts: !no_system_fonts,
                annotations: !no_annotations,
                white_background: !transparent,
                collect_text: false,
                hidden_layers: layer_state(&doc, hide_layers, show_layers)?,
                replace_colors: None,
            };
            let dir = dir_of(file, out_dir);
            let base = stem(file);
            let targets: Vec<(usize, PathBuf)> = idx
                .iter()
                .map(|&i| {
                    let t = output
                        .clone()
                        .unwrap_or_else(|| dir.join(format!("{base}_Page_{}.{ext}", i + 1)));
                    (i, t)
                })
                .collect();
            for (_, t) in &targets {
                check_free(t, *force)?;
            }
            let mut report = Vec::new();
            for (i, t) in &targets {
                let Some(&page) = list.get(*i) else { continue };
                let r = e(folio_render::render_page(&doc, page, &opts))?;
                let png = if *transparent {
                    e(r.png())?
                } else {
                    e(export::encode(
                        &r.pixmap,
                        &export::Settings {
                            format: fmt,
                            color,
                            dpi: *dpi,
                        },
                    ))?
                };
                std::fs::write(t, png)
                    .map_err(|err| Fail::from(format!("{}: {err}", t.display())))?;
                report.push((
                    i + 1,
                    t.clone(),
                    r.pixmap.width(),
                    r.pixmap.height(),
                    r.skipped,
                ));
            }
            if cli.json {
                let files: Vec<String> = report
                    .iter()
                    .map(|(n, t, w, h, sk)| {
                        let skipped: Vec<(&str, String)> =
                            sk.iter().map(|(k, c)| (*k, c.to_string())).collect();
                        json::obj(&[
                            ("page", n.to_string()),
                            ("file", json::s(&t.display().to_string())),
                            ("width", w.to_string()),
                            ("height", h.to_string()),
                            ("not_drawn_yet", json::obj(&skipped)),
                        ])
                    })
                    .collect();
                println!(
                    "{}",
                    json::obj(&[("ok", "true".into()), ("images", json::arr(&files))])
                );
            } else {
                for (n, t, w, h, sk) in &report {
                    let note = if sk.is_empty() {
                        String::new()
                    } else {
                        let parts: Vec<String> =
                            sk.iter().map(|(k, c)| format!("{c} {k}")).collect();
                        format!("  (not drawn yet: {})", parts.join(", "))
                    };
                    println!("page {n}: {} ({w} x {h}){note}", t.display());
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Layers { file } => {
            let doc = load(file)?;
            let list = folio_render::oc::layers(&doc);
            if cli.json {
                let rows: Vec<String> = list
                    .iter()
                    .map(|l| {
                        json::obj(&[
                            ("name", json::s(&l.name)),
                            ("title_only", l.ocg.is_none().to_string()),
                            ("depth", l.depth.to_string()),
                            ("visible_by_default", l.on_by_default.to_string()),
                            ("locked", l.locked.to_string()),
                        ])
                    })
                    .collect();
                println!(
                    "{}",
                    json::obj(&[("ok", "true".into()), ("layers", json::arr(&rows))])
                );
            } else if list.is_empty() {
                println!("{} has no layers.", file.display());
            } else {
                for l in &list {
                    let indent = "  ".repeat(l.depth);
                    match l.ocg {
                        None => println!("{indent}{}", l.name),
                        Some(_) => println!(
                            "{indent}[{}] {}{}",
                            if l.on_by_default { "x" } else { " " },
                            l.name,
                            if l.locked { " (locked)" } else { "" }
                        ),
                    }
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Attachments {
            file,
            save,
            output,
            force,
        } => {
            let doc = load(file)?;
            let list = folio_ops::attachments::list(&doc);
            let Some(which) = save else {
                if cli.json {
                    let rows: Vec<String> = list
                        .iter()
                        .enumerate()
                        .map(|(i, a)| {
                            json::obj(&[
                                ("number", (i + 1).to_string()),
                                ("name", json::s(&a.name)),
                                ("description", json::opt(a.description.as_deref())),
                                ("size", a.size.map_or("null".into(), |s| s.to_string())),
                                ("modified", json::opt(a.modified.as_deref())),
                                (
                                    "page",
                                    a.page.map_or("null".into(), |p| (p + 1).to_string()),
                                ),
                            ])
                        })
                        .collect();
                    println!(
                        "{}",
                        json::obj(&[("ok", "true".into()), ("attachments", json::arr(&rows))])
                    );
                } else if list.is_empty() {
                    println!("{} has no attachments.", file.display());
                } else {
                    for (i, a) in list.iter().enumerate() {
                        let size = a.size.map(info::human_size).unwrap_or_else(|| "?".into());
                        let page = a
                            .page
                            .map(|p| format!(", on page {}", p + 1))
                            .unwrap_or_default();
                        println!("{:>3}. {} ({size}{page})", i + 1, a.name);
                        if let Some(d) = &a.description {
                            println!("     {d}");
                        }
                    }
                }
                return Ok(ExitCode::SUCCESS);
            };
            // Saving an attachment hands its content out, like copying.
            need(&doc, Need::Copy, file)?;
            let idx = match which.trim().parse::<usize>() {
                Ok(n) if n >= 1 && n <= list.len() => n - 1,
                _ => list
                    .iter()
                    .position(|a| a.name == *which)
                    .ok_or_else(|| {
                        Fail::from(format!(
                            "no attachment numbered or named '{which}'; run 'folio attachments {}' to list them",
                            file.display()
                        ))
                    })?,
            };
            let Some(att) = list.get(idx) else {
                return Err("no such attachment".into());
            };
            let bytes = e(folio_ops::attachments::contents(&doc, att))?;
            let target = output.clone().unwrap_or_else(|| {
                dir_of(file, &None).join(folio_ops::split::safe_file_name(&att.name))
            });
            check_free(&target, *force)?;
            std::fs::write(&target, &bytes)
                .map_err(|err| Fail::from(format!("{}: {err}", target.display())))?;
            if cli.json {
                println!(
                    "{}",
                    json::obj(&[
                        ("ok", "true".into()),
                        ("output", json::s(&target.display().to_string())),
                        ("bytes", bytes.len().to_string()),
                    ])
                );
            } else {
                println!("saved {} ({} bytes)", target.display(), bytes.len());
            }
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Fonts { file } => {
            let doc = load(file)?;
            let list = folio_ops::fonts::list(&doc);
            if cli.json {
                let rows: Vec<String> = list
                    .iter()
                    .map(|f| {
                        json::obj(&[
                            ("name", json::s(&f.name)),
                            ("type", json::s(&f.kind)),
                            ("encoding", json::opt(f.encoding.as_deref())),
                            ("embedded", f.embedded.to_string()),
                            ("subset", f.subset.to_string()),
                        ])
                    })
                    .collect();
                println!(
                    "{}",
                    json::obj(&[("ok", "true".into()), ("fonts", json::arr(&rows))])
                );
            } else if list.is_empty() {
                println!("{} uses no fonts.", file.display());
            } else {
                for f in &list {
                    let how = match (f.embedded, f.subset) {
                        (true, true) => "Embedded Subset",
                        (true, false) => "Embedded",
                        _ => "Not embedded",
                    };
                    println!("{}", f.name);
                    println!("    Type: {}", f.kind);
                    if let Some(enc) = &f.encoding {
                        println!("    Encoding: {enc}");
                    }
                    println!("    {how}");
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Security { file } => {
            let doc = load(file)?;
            let rows = security::summary(&doc);
            if cli.json {
                let body = security::to_json(&rows);
                println!("{{\"ok\":true,{}", body.trim_start_matches('{'));
            } else {
                print!("{}", security::to_text(&rows));
            }
            Ok(ExitCode::SUCCESS)
        }

        Cmd::Encrypt {
            file,
            compat,
            scope,
            open_password,
            permissions_password,
            printing,
            changes,
            no_copy,
            no_screen_reader,
            out,
        } => {
            let mut doc = load(file)?;
            check_free(&out.output, out.force)?;
            let settings = Settings {
                compat: match compat {
                    CompatArg::Acrobat3 => Compat::Acrobat3,
                    CompatArg::Acrobat5 => Compat::Acrobat5,
                    CompatArg::Acrobat6 => Compat::Acrobat6,
                    CompatArg::Acrobat7 => Compat::Acrobat7,
                    CompatArg::Acrobatx => Compat::AcrobatX,
                },
                scope: match scope {
                    ScopeArg::All => Scope::All,
                    ScopeArg::ExceptMetadata => Scope::ExceptMetadata,
                    ScopeArg::Attachments => Scope::AttachmentsOnly,
                },
                open_password: open_password.clone(),
                permissions_password: permissions_password.clone(),
                printing: match printing {
                    PrintingArg::None => Printing::None,
                    PrintingArg::Low => Printing::Low,
                    PrintingArg::High => Printing::High,
                },
                changes: match changes {
                    ChangesArg::None => Changes::None,
                    ChangesArg::Pages => Changes::Pages,
                    ChangesArg::Forms => Changes::Forms,
                    ChangesArg::Comments => Changes::Comments,
                    ChangesArg::Any => Changes::Any,
                },
                copy: !no_copy,
                screen_reader: !no_screen_reader,
            };
            doc.encrypt(&settings)
                .map_err(|e| from_core(e, Some(file)))?;
            save(&doc, &out.output, out.force)?;
            let method = doc
                .security
                .as_ref()
                .map(|s| s.method())
                .unwrap_or_default();
            Ok(done(
                cli,
                &format!("encrypted ({method})"),
                &out.output,
                &doc,
                &[("encryption", json::s(&method))],
            ))
        }

        Cmd::Decrypt { file, out } => {
            let mut doc = load(file)?;
            if doc.security.is_none() {
                return Err(format!("{} is not protected", file.display()).into());
            }
            doc.remove_security()
                .map_err(|e| from_core(e, Some(file)))?;
            save(&doc, &out.output, out.force)?;
            Ok(done(cli, "security removed", &out.output, &doc, &[]))
        }

        Cmd::Text {
            file,
            pages: spec,
            output,
            encoding,
            force,
        } => {
            let doc = load(file)?;
            need(&doc, Need::Copy, file)?;
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
                    std::fs::write(out, bytes)
                        .map_err(|e| Fail::from(format!("{}: {e}", out.display())))?;
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

/// Margins (top, bottom, left, right, in points at 72 dpi) around the
/// non-white content of a rendered page, or `None` for a blank page.
fn white_margins(r: &folio_render::Rendered) -> Option<Margins> {
    let [t, b, l, rr] = folio_render::export::content_margins(&r.pixmap)?;
    Some(Margins {
        top: f64::from(t),
        bottom: f64::from(b),
        left: f64::from(l),
        right: f64::from(rr),
    })
}

/// The layers to hide: the document's default, then `--hide-layers` and
/// `--show-layers` by name. Unknown names are an error that lists the real ones.
fn layer_state(doc: &Document, hide: &str, show: &str) -> Res<Option<std::sync::Arc<[u32]>>> {
    let names = |s: &str| -> Vec<String> {
        s.split(',')
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .collect()
    };
    let (hide, show) = (names(hide), names(show));
    if hide.is_empty() && show.is_empty() {
        return Ok(None);
    }
    let layers = folio_render::oc::layers(doc);
    let mut hidden = folio_render::oc::default_hidden(doc);
    for (list, off) in [(&hide, true), (&show, false)] {
        for n in list {
            let found: Vec<u32> = layers
                .iter()
                .filter(|l| l.name == *n)
                .filter_map(|l| l.ocg.map(|r| r.num))
                .collect();
            if found.is_empty() {
                let known: Vec<&str> = layers
                    .iter()
                    .filter(|l| l.ocg.is_some())
                    .map(|l| l.name.as_str())
                    .collect();
                return Err(format!(
                    "no layer named '{n}'; layers: {}",
                    if known.is_empty() {
                        "(none)".to_string()
                    } else {
                        known.join(", ")
                    }
                )
                .into());
            }
            for num in found {
                if off {
                    hidden.insert(num);
                } else {
                    hidden.remove(&num);
                }
            }
        }
    }
    let v: Vec<u32> = hidden.into_iter().collect();
    Ok(Some(v.into()))
}
