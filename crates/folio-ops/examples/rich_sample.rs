//! Write the stage-1 rich sample: `cargo run -p folio-ops --example rich_sample -- out.pdf`

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args().nth(1).unwrap_or_else(|| "rich.pdf".into());
    folio_ops::samples::rich()?.save(&out)?;
    Ok(())
}
