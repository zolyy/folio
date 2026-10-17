//! Print a PDF's page text as Folio's Find sees it: `cargo run -p folio-render --example dump_text -- file.pdf`

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("give a PDF path")?;
    let doc = folio_core::Document::load(&path)?;
    for (i, p) in doc.pages()?.into_iter().enumerate() {
        let t = folio_render::page_text(&doc, p)?;
        println!("--- page {} ---\n{}", i + 1, t.text);
    }
    Ok(())
}
