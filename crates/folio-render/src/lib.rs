//! # folio-render
//!
//! Draws PDF pages into pixels. The content-stream interpreter, graphics
//! state, colour spaces and functions are Folio's own; rasterisation (paths,
//! strokes, masks, blending) is done by tiny-skia.
//!
//! Stage 3a covers vector graphics; images (3b), text (3c) and shadings /
//! soft masks (3d) arrive in the following sub-stages. Content that cannot
//! be drawn yet is skipped and counted in [`Rendered::skipped`].

pub mod color;
pub mod font;
pub mod function;
pub mod image;
mod interp;
pub mod samples;

use folio_core::{Document, Error, ObjRef, Result};
pub use tiny_skia::Pixmap;

/// Largest image Folio will allocate for one page (about 16k × 16k).
pub const MAX_PIXELS: u64 = 268_435_456;

/// Acrobat's Page Display rendering choices.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Resolution in pixels per inch (Acrobat: 72–2400 when exporting).
    pub dpi: f64,
    /// "Smooth line art".
    pub smooth_line_art: bool,
    /// "Enhance thin lines": strokes thinner than a pixel are drawn one pixel wide.
    pub enhance_thin_lines: bool,
    /// "Smooth images".
    pub smooth_images: bool,
    /// "Smooth text".
    pub smooth_text: bool,
    /// Draw fonts the PDF does not embed with a metric-compatible font
    /// installed on this computer (never copied or shipped).
    pub system_fonts: bool,
    /// Paint the page white first (off = transparent background).
    pub white_background: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            dpi: 72.0,
            smooth_line_art: true,
            enhance_thin_lines: true,
            smooth_images: true,
            smooth_text: true,
            system_fonts: true,
            white_background: true,
        }
    }
}

/// A rendered page and what could not be drawn yet.
#[derive(Debug)]
pub struct Rendered {
    pub pixmap: Pixmap,
    /// Operations skipped because their kind is not implemented yet
    /// (e.g. `"text"`, `"image"`, `"shading"`), with counts.
    pub skipped: Vec<(&'static str, usize)>,
}

impl Rendered {
    pub fn png(&self) -> Result<Vec<u8>> {
        self.pixmap
            .encode_png()
            .map_err(|e| Error::Invalid(format!("could not encode PNG: {e}")))
    }

    /// Unpremultiplied RGBA of one pixel, if inside the image.
    pub fn rgba(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let p = self.pixmap.pixel(x, y)?.demultiply();
        Some([p.red(), p.green(), p.blue(), p.alpha()])
    }
}

/// The pixel size a page renders to at `dpi`, rotation applied.
pub fn page_pixel_size(doc: &Document, page: ObjRef, dpi: f64) -> Result<(u32, u32)> {
    let geo = interp::PageGeometry::new(doc, page, dpi)?;
    Ok((geo.width, geo.height))
}

/// Render one page.
pub fn render_page(doc: &Document, page: ObjRef, opts: &Options) -> Result<Rendered> {
    interp::render(doc, page, opts)
}
