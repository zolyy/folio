//! One open document as the reading window sees it: pages, the current
//! page, zoom, view rotation and page layout (Acrobat's View menu), with no
//! user interface attached, so every command is testable without a screen.

use std::path::PathBuf;
use std::sync::Arc;

use folio_core::{Document, ObjRef};
use folio_ops::outline::{self, OutlineNode};

/// Screen pixels per PDF point at 100% (Acrobat: Page Display → Resolution
/// "Use system setting": 100% shows the page at its printed size on a
/// 96-pixel-per-inch screen; egui points are such pixels).
pub const PX_PER_PT: f32 = 96.0 / 72.0;

/// Acrobat's zoom range, in percent.
pub const MIN_ZOOM: f32 = 1.0;
pub const MAX_ZOOM: f32 = 6400.0;

/// The magnifications of Acrobat's zoom menu, which Zoom In / Zoom Out step through.
pub const ZOOM_STEPS: [f32; 20] = [
    8.33, 12.5, 25.0, 33.33, 50.0, 66.67, 75.0, 100.0, 125.0, 150.0, 200.0, 300.0, 400.0, 600.0,
    800.0, 1200.0, 1600.0, 2400.0, 3200.0, 6400.0,
];

/// Gap between pages in scrolling layouts, in screen points (Folio choice).
pub const PAGE_GAP: f32 = 8.0;

/// Pages bigger than this (Acrobat's limit is 200 × 200 in) or degenerate
/// ones are shown at a placeholder size instead.
const MAX_PAGE_PT: f64 = 14_400.0;
const FALLBACK_PAGE: [f32; 2] = [612.0, 792.0];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    /// A fixed magnification in percent.
    Percent(f32),
    /// View → Zoom → Zoom to Page Level (Fit Page).
    FitPage,
    /// View → Zoom → Fit Width.
    FitWidth,
}

/// View → Page Display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Single Page View: one page at a time.
    SinglePage,
    /// Enable Scrolling: every page in one column.
    Continuous,
}

#[derive(Debug)]
pub struct Tab {
    /// Where the document came from, if from a file.
    pub path: Option<PathBuf>,
    pub title: String,
    pub doc: Arc<Document>,
    pub pages: Vec<ObjRef>,
    /// Each page's size in points with its own `/Rotate` applied.
    pub sizes: Vec<[f32; 2]>,
    /// Page labels (§12.4.2), when the document defines them.
    pub labels: Option<Vec<String>>,
    pub outline: Vec<OutlineNode>,
    /// What the loader had to repair, for the status bar.
    pub repairs: Vec<String>,
    /// The page shown (single page) or the page in the middle of the window.
    pub current: usize,
    pub zoom: Zoom,
    /// The page Fit Page / Fit Width are measured on: the page navigated to
    /// last, not the one scrolled past, so mixed page sizes do not make the
    /// magnification jump while scrolling.
    pub fit_ref: usize,
    /// Rotate View: 0, 90, 180 or 270 degrees clockwise. Not saved to the file.
    pub rotation: u16,
    pub layout: Layout,
    /// A page the view must scroll to on the next frame.
    pub scroll_to: Option<usize>,
    /// Identifies this document in the render cache.
    pub id: u64,
}

impl Tab {
    pub fn new(doc: Document, path: Option<PathBuf>, title: String, id: u64) -> Self {
        let pages = doc.pages().unwrap_or_default();
        let sizes = pages
            .iter()
            .map(|&p| page_size(&doc, p))
            .collect::<Vec<_>>();
        let labels = folio_ops::labels::per_page(&doc, pages.len())
            .map(|l| l.iter().map(|x| x.text()).collect());
        Tab {
            path,
            title,
            outline: outline::tree(&doc),
            repairs: doc.repairs.clone(),
            doc: Arc::new(doc),
            pages,
            sizes,
            labels,
            current: 0,
            zoom: Zoom::FitPage,
            fit_ref: 0,
            rotation: 0,
            layout: Layout::Continuous,
            scroll_to: Some(0),
            id,
        }
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// A page's size on screen at `scale` (screen points per PDF point),
    /// view rotation applied.
    pub fn display_size(&self, page: usize, scale: f32) -> [f32; 2] {
        let [w, h] = self.sizes.get(page).copied().unwrap_or(FALLBACK_PAGE);
        let [w, h] = if self.rotation % 180 == 90 {
            [h, w]
        } else {
            [w, h]
        };
        [w * scale, h * scale]
    }

    /// Screen points per PDF point for the current zoom in a window of
    /// `viewport` screen points. Fit modes fit the current page, as Acrobat does.
    pub fn scale(&self, viewport: [f32; 2]) -> f32 {
        let [w, h] = self.display_size(self.fit_ref, 1.0);
        let avail_w = (viewport[0] - 2.0 * PAGE_GAP).max(1.0);
        let avail_h = (viewport[1] - 2.0 * PAGE_GAP).max(1.0);
        let s = match self.zoom {
            Zoom::Percent(p) => p / 100.0 * PX_PER_PT,
            Zoom::FitWidth => avail_w / w,
            Zoom::FitPage => (avail_w / w).min(avail_h / h),
        };
        let s = if s.is_finite() { s } else { PX_PER_PT };
        s.clamp(MIN_ZOOM / 100.0 * PX_PER_PT, MAX_ZOOM / 100.0 * PX_PER_PT)
    }

    /// The magnification in percent, as the zoom box shows it.
    pub fn percent(&self, viewport: [f32; 2]) -> f32 {
        self.scale(viewport) / PX_PER_PT * 100.0
    }

    /// Change the zoom (menu, toolbar, keyboard).
    /// A magnification outside Acrobat's range is clamped; one that is not
    /// a number means 100%.
    pub fn set_zoom(&mut self, zoom: Zoom) {
        self.zoom = match zoom {
            Zoom::Percent(p) if !p.is_finite() => Zoom::Percent(100.0),
            Zoom::Percent(p) => Zoom::Percent(p.clamp(MIN_ZOOM, MAX_ZOOM)),
            z => z,
        };
        self.fit_ref = self.current;
        self.scroll_to = Some(self.current);
    }

    /// Zoom In: the next preset above the current magnification.
    pub fn zoom_in(&mut self, viewport: [f32; 2]) {
        let now = self.percent(viewport);
        let next = ZOOM_STEPS
            .iter()
            .copied()
            .find(|&z| z > now + 0.01)
            .unwrap_or(MAX_ZOOM);
        self.set_zoom(Zoom::Percent(next));
    }

    /// Zoom Out: the next preset below the current magnification.
    pub fn zoom_out(&mut self, viewport: [f32; 2]) {
        let now = self.percent(viewport);
        let next = ZOOM_STEPS
            .iter()
            .rev()
            .copied()
            .find(|&z| z < now - 0.01)
            .unwrap_or(MIN_ZOOM);
        self.set_zoom(Zoom::Percent(next));
    }

    /// A magnification typed into the zoom box ("150", "150%"), clamped to
    /// Acrobat's range. Returns false if the text is not a number.
    pub fn set_zoom_text(&mut self, text: &str) -> bool {
        let t = text.trim().trim_end_matches('%').trim();
        match t.replace(',', ".").parse::<f32>() {
            Ok(v) if v.is_finite() => {
                self.set_zoom(Zoom::Percent(v.clamp(MIN_ZOOM, MAX_ZOOM)));
                true
            }
            _ => false,
        }
    }

    pub fn rotate_clockwise(&mut self) {
        self.rotation = (self.rotation + 90) % 360;
        self.fit_ref = self.current;
        self.scroll_to = Some(self.current);
    }

    pub fn rotate_counterclockwise(&mut self) {
        self.rotation = (self.rotation + 270) % 360;
        self.fit_ref = self.current;
        self.scroll_to = Some(self.current);
    }

    pub fn go_to(&mut self, page: usize) {
        if self.pages.is_empty() {
            return;
        }
        self.current = page.min(self.pages.len() - 1);
        self.fit_ref = self.current;
        self.scroll_to = Some(self.current);
    }

    pub fn next_page(&mut self) {
        self.go_to(self.current.saturating_add(1));
    }

    pub fn previous_page(&mut self) {
        self.go_to(self.current.saturating_sub(1));
    }

    pub fn first_page(&mut self) {
        self.go_to(0);
    }

    pub fn last_page(&mut self) {
        self.go_to(usize::MAX);
    }

    /// What the page box shows for a page: its label when the document has
    /// labels (Acrobat's "Use logical page numbers"), else its number.
    pub fn page_name(&self, page: usize) -> String {
        match self.labels.as_ref().and_then(|l| l.get(page)) {
            Some(l) if !l.is_empty() => l.clone(),
            _ => (page + 1).to_string(),
        }
    }

    /// Go to what was typed in the page box: a page label first, then a page
    /// number. Returns false (and stays put) if neither matches.
    pub fn go_to_text(&mut self, text: &str) -> bool {
        let t = text.trim();
        if t.is_empty() {
            return false;
        }
        if let Some(i) = self
            .labels
            .as_ref()
            .and_then(|l| l.iter().position(|x| x == t))
        {
            self.go_to(i);
            return true;
        }
        match t.parse::<usize>() {
            Ok(n) if n >= 1 && n <= self.pages.len() => {
                self.go_to(n - 1);
                true
            }
            _ => false,
        }
    }

    /// The index of a page object, for bookmarks.
    pub fn index_of(&self, page: ObjRef) -> Option<usize> {
        self.pages.iter().position(|&p| p == page)
    }

    /// Top edge of each page in the scrolling layout at `scale`, plus the
    /// total height, in screen points.
    pub fn page_tops(&self, scale: f32) -> (Vec<f32>, f32) {
        let mut tops = Vec::with_capacity(self.pages.len());
        let mut y = PAGE_GAP;
        for i in 0..self.pages.len() {
            tops.push(y);
            y += self.display_size(i, scale)[1] + PAGE_GAP;
        }
        (tops, y)
    }

    /// The page under the vertical position `y` (screen points from the top).
    pub fn page_at(&self, tops: &[f32], y: f32) -> usize {
        tops.partition_point(|&t| t <= y).saturating_sub(1)
    }

    /// Single Page View or Enable Scrolling.
    pub fn set_layout(&mut self, layout: Layout) {
        self.layout = layout;
        self.scroll_to = Some(self.current);
    }
}

/// A page's size in points, its `/Rotate` applied, sane even for broken boxes.
fn page_size(doc: &Document, page: ObjRef) -> [f32; 2] {
    let (w, h) = folio_render::page_size_points(doc, page);
    let ok = |v: f64| v.is_finite() && (1.0..=MAX_PAGE_PT).contains(&v);
    if ok(w) && ok(h) {
        [w as f32, h as f32]
    } else {
        FALLBACK_PAGE
    }
}
