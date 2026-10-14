//! Write the stage-3a test document: `cargo run -p folio-render --example vector_sample -- out.pdf`

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "vector-sample.pdf".into());
    folio_render::samples::vector()?.save(&out)?;
    println!("{out}");
    Ok(())
}
