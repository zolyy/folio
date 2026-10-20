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
pub mod export;
pub mod font;
pub mod function;
pub mod image;
mod interp;
pub mod oc;
pub mod samples;
pub mod shading;
pub mod text;

use folio_core::{Document, Error, ObjRef, Result};
pub use tiny_skia::Pixmap;

/// Largest image Folio will allocate for one page (about 16k × 16k).
pub const MAX_PIXELS: u64 = 268_435_456;

/// Acrobat's Page Display rendering choices.
#[derive(Clone, Debug)]
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
    /// Draw annotation appearances (comments, stamps, form fields).
    pub annotations: bool,
    /// Paint the page white first (off = transparent background).
    pub white_background: bool,
    /// Optional-content groups (layers) to hide, by object number; `None`
    /// shows the document's default layer state (Acrobat's Layers panel).
    pub hidden_layers: Option<std::sync::Arc<[u32]>>,
    /// Accessibility → Replace Document Colors.
    pub replace_colors: Option<ReplaceColors>,
    /// Record every glyph shown, with its box (for search and selection).
    pub collect_text: bool,
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
            annotations: true,
            white_background: true,
            hidden_layers: None,
            replace_colors: None,
            collect_text: false,
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
    /// Glyphs shown, in content order, when `Options::collect_text` is on.
    pub text: Vec<text::Glyph>,
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

/// The renderer without the last-resort guard, so tests see real panics.
#[doc(hidden)]
pub mod interp_for_tests {
    pub use crate::interp::render;
}

/// A page's visible size in points (width, height), its `/Rotate` applied:
/// the size `render_page` draws at 72 dpi, before rounding to pixels.
pub fn page_size_points(doc: &Document, page: ObjRef) -> (f64, f64) {
    let (b, rot) = interp::visible_box(doc, page);
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    if rot % 180 == 90 { (h, w) } else { (w, h) }
}

/// The pixel size a page renders to at `dpi`, rotation applied.
pub fn page_pixel_size(doc: &Document, page: ObjRef, dpi: f64) -> Result<(u32, u32)> {
    let geo = interp::PageGeometry::new(doc, page, dpi)?;
    Ok((geo.width, geo.height))
}

/// Render one page.
///
/// A last-resort guard (AGENTS.md §3) turns a panic escaping the renderer
/// or its rasteriser into an error for this page; the inputs that caused
/// such panics are rejected earlier and kept as regression tests.
pub fn render_page(doc: &Document, page: ObjRef, opts: &Options) -> Result<Rendered> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        interp::render(doc, page, opts)
    }))
    .unwrap_or_else(|_| {
        Err(Error::Invalid(
            "this page could not be drawn (the renderer hit an internal error)".into(),
        ))
    })
}

/// The page's text with every character's position, for search and
/// selection. Renders at a low resolution: only the glyph geometry is kept.
pub fn page_text(doc: &Document, page: ObjRef) -> Result<text::PageText> {
    let opts = Options {
        dpi: 9.0,
        smooth_line_art: false,
        smooth_images: false,
        smooth_text: false,
        annotations: false,
        collect_text: true,
        ..Options::default()
    };
    let r = render_page(doc, page, &opts)?;
    // Text placed outside the visible page (e.g. the other half of a spread
    // that was cropped away) is not part of the page the reader sees.
    let (w, h) = page_size_points(doc, page);
    let (w, h) = (w as f32, h as f32);
    let visible: Vec<text::Glyph> = r
        .text
        .into_iter()
        .filter(|g| {
            let cx = g.quad.iter().map(|p| p[0]).sum::<f32>() / 4.0;
            let cy = g.quad.iter().map(|p| p[1]).sum::<f32>() / 4.0;
            (-1.0..=w + 1.0).contains(&cx) && (-1.0..=h + 1.0).contains(&cy)
        })
        .collect();
    Ok(text::layout(&visible))
}

/// Acrobat's Accessibility preference "Replace Document Colors": the page
/// background and the text (and optionally line art) are drawn in chosen
/// colours; images are left alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReplaceColors {
    pub background: [u8; 3],
    pub text: [u8; 3],
    /// "Change the color of line art as well as text".
    pub line_art: bool,
    /// "Only change the color of black text or line art".
    pub only_black: bool,
}

impl ReplaceColors {
    /// Acrobat's high-contrast colour combinations.
    pub const HIGH_CONTRAST: [(&'static str, ReplaceColors); 4] = [
        (
            "Green text on black",
            ReplaceColors::pair([0, 0, 0], [0, 255, 0]),
        ),
        (
            "Yellow text on black",
            ReplaceColors::pair([0, 0, 0], [255, 255, 0]),
        ),
        (
            "White text on black",
            ReplaceColors::pair([0, 0, 0], [255, 255, 255]),
        ),
        (
            "Black text on white",
            ReplaceColors::pair([255, 255, 255], [0, 0, 0]),
        ),
    ];

    pub const fn pair(background: [u8; 3], text: [u8; 3]) -> Self {
        ReplaceColors {
            background,
            text,
            line_art: false,
            only_black: false,
        }
    }
}
