//! `folio` — the PDF workbench, headless. Every command can answer in JSON
//! (`--json`) so agents and scripts can drive it.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use folio_core::Document;

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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(e) => {
            if cli.json {
                println!("{{\"ok\":false,\"error\":{}}}", json_str(&e));
            } else {
                eprintln!("error: {e}");
            }
            ExitCode::from(2)
        }
    }
}

fn run(cli: &Cli) -> Result<ExitCode, String> {
    match &cli.cmd {
        Cmd::Check {
            file,
            repair,
            force,
        } => {
            let doc = Document::load(file).map_err(|e| format!("{}: {e}", file.display()))?;
            let pages = doc.pages().map_err(|e| e.to_string())?;
            if let Some(out) = repair {
                if out.exists() && !force {
                    return Err(format!(
                        "{} already exists; pass --force to replace it",
                        out.display()
                    ));
                }
                doc.save(out)
                    .map_err(|e| format!("{}: {e}", out.display()))?;
            }
            if cli.json {
                let repairs: Vec<String> = doc.repairs.iter().map(|r| json_str(r)).collect();
                println!(
                    "{{\"ok\":true,\"version\":{},\"pages\":{},\"objects\":{},\"repairs\":[{}]}}",
                    json_str(&doc.version),
                    pages.len(),
                    doc.objects.len(),
                    repairs.join(",")
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
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
