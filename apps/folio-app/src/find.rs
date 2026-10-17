//! Find (Ctrl+F) and text selection: the page texts read in the background,
//! the search state, the selection across pages, and the mapping between page
//! points and the rotated view. No egui here, so all of it is unit-tested.

use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use folio_core::{Document, ObjRef};
use folio_render::text::{self, FindOptions, PageText, Pt};

/// Every page's text with positions, read on a background thread so a long
/// document never freezes the window.
pub struct TextIndex {
    pages: Arc<Mutex<Vec<Option<Arc<PageText>>>>>,
    ready: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    started: bool,
    total: usize,
}

impl TextIndex {
    pub fn new(total: usize) -> Self {
        TextIndex {
            pages: Arc::new(Mutex::new(vec![None; total])),
            ready: Arc::new(AtomicUsize::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
            started: false,
            total,
        }
    }

    /// Start reading (once). `notify` is called as pages become ready.
    pub fn start(
        &mut self,
        doc: Arc<Document>,
        pages: Vec<ObjRef>,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) {
        if self.started {
            return;
        }
        self.started = true;
        let (slots, ready, stop) = (self.pages.clone(), self.ready.clone(), self.stop.clone());
        let spawned = std::thread::Builder::new()
            .name("folio-text".into())
            .spawn(move || {
                for (i, p) in pages.into_iter().enumerate() {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    // A page that cannot be read counts as a page without text.
                    let t = catch_unwind(AssertUnwindSafe(|| folio_render::page_text(&doc, p)))
                        .ok()
                        .and_then(Result::ok)
                        .unwrap_or_default();
                    if let Ok(mut s) = slots.lock()
                        && let Some(slot) = s.get_mut(i)
                    {
                        *slot = Some(Arc::new(t));
                    }
                    ready.fetch_add(1, Ordering::Relaxed);
                    notify();
                }
            });
        if spawned.is_err() {
            // No thread: the index stays empty and Find says so.
            self.started = false;
        }
    }

    /// Read every page now, on this thread (tests and very small documents).
    pub fn fill_now(&mut self, doc: &Document, pages: &[ObjRef]) {
        let texts: Vec<Option<Arc<PageText>>> = pages
            .iter()
            .map(|&p| {
                Some(Arc::new(
                    folio_render::page_text(doc, p).unwrap_or_default(),
                ))
            })
            .collect();
        if let Ok(mut s) = self.pages.lock() {
            *s = texts;
        }
        self.ready.store(pages.len(), Ordering::Relaxed);
        self.started = true;
    }

    pub fn get(&self, page: usize) -> Option<Arc<PageText>> {
        self.pages.lock().ok()?.get(page).cloned().flatten()
    }

    pub fn ready(&self) -> usize {
        self.ready.load(Ordering::Relaxed).min(self.total)
    }

    pub fn total(&self) -> usize {
        self.total
    }

    pub fn complete(&self) -> bool {
        self.ready() >= self.total
    }
}

impl std::fmt::Debug for TextIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TextIndex({} of {} pages)", self.ready(), self.total)
    }
}

impl Drop for TextIndex {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// One match: a page and a character range in that page's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub page: usize,
    pub range: Range<usize>,
}

/// The Find bar.
#[derive(Default)]
pub struct FindState {
    pub open: bool,
    pub query: String,
    pub whole_words: bool,
    pub case_sensitive: bool,
    pub results: Vec<Hit>,
    pub current: Option<usize>,
    /// Ask the text box for keyboard focus on the next frame.
    pub focus: bool,
    /// Go to the first match as soon as there is one (open parameters).
    pub pending: bool,
    /// What the results were computed for: query, options, document, pages ready.
    computed: Option<(String, bool, bool, u64, usize)>,
}

impl FindState {
    pub fn options(&self) -> FindOptions {
        FindOptions {
            whole_words: self.whole_words,
            case_sensitive: self.case_sensitive,
        }
    }

    /// Recompute the matches when the query, the options, the document or
    /// the number of pages read has changed. Returns true when it did.
    pub fn refresh(&mut self, doc_id: u64, index: &TextIndex) -> bool {
        let key = (
            self.query.clone(),
            self.whole_words,
            self.case_sensitive,
            doc_id,
            index.ready(),
        );
        if self.computed.as_ref() == Some(&key) {
            return false;
        }
        let same_query = self.computed.as_ref().is_some_and(|(q, w, c, d, _)| {
            *q == key.0 && *w == key.1 && *c == key.2 && *d == key.3
        });
        let opts = self.options();
        self.results.clear();
        if !self.query.trim().is_empty() {
            for page in 0..index.total() {
                if let Some(t) = index.get(page) {
                    for range in text::find(&t, &self.query, opts) {
                        self.results.push(Hit { page, range });
                    }
                }
            }
        }
        // More pages read: keep the user's place. New query: start over.
        self.current = if self.results.is_empty() {
            None
        } else if same_query {
            // Keep the place (or the lack of one) while more pages are read.
            self.current.map(|c| c.min(self.results.len() - 1))
        } else {
            None
        };
        self.computed = Some(key);
        true
    }

    /// Move to the next match at or after `from_page` (or wrap around).
    pub fn next(&mut self, from_page: usize) -> Option<&Hit> {
        let n = self.results.len();
        if n == 0 {
            return None;
        }
        let i = match self.current {
            Some(c) => (c + 1) % n,
            None => self
                .results
                .iter()
                .position(|h| h.page >= from_page)
                .unwrap_or(0),
        };
        self.current = Some(i);
        self.results.get(i)
    }

    pub fn previous(&mut self, from_page: usize) -> Option<&Hit> {
        let n = self.results.len();
        if n == 0 {
            return None;
        }
        let i = match self.current {
            Some(c) => (c + n - 1) % n,
            None => self
                .results
                .iter()
                .rposition(|h| h.page <= from_page)
                .unwrap_or(n - 1),
        };
        self.current = Some(i);
        self.results.get(i)
    }

    /// Whether a match at or after `page` is known, or none can still come.
    pub fn settled_from(&self, page: usize, index: &TextIndex) -> bool {
        index.complete() || self.results.iter().any(|h| h.page >= page)
    }

    /// "3 of 17", "No matches", or how far the reading has come.
    pub fn status(&self, index: &TextIndex) -> String {
        let reading = !index.complete();
        let base = match (self.results.len(), self.current) {
            (0, _) if self.query.trim().is_empty() => String::new(),
            (0, _) if reading => String::new(),
            (0, _) => "No matches".into(),
            (n, Some(c)) => format!("{} of {n}", c + 1),
            (n, None) => format!("{n} match{}", if n == 1 { "" } else { "es" }),
        };
        if reading {
            let r = format!("reading {} of {} pages", index.ready(), index.total());
            if base.is_empty() {
                r
            } else {
                format!("{base} ({r})")
            }
        } else {
            base
        }
    }
}

/// A text selection: from where the drag started to where it is now, each
/// a page and a character index. It may span pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: (usize, usize),
    pub head: (usize, usize),
}

impl Selection {
    pub fn ordered(&self) -> ((usize, usize), (usize, usize)) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// The selected characters of `page`, whose text has `len` characters.
    pub fn range_on(&self, page: usize, len: usize) -> Option<Range<usize>> {
        let ((p0, c0), (p1, c1)) = self.ordered();
        if page < p0 || page > p1 {
            return None;
        }
        let start = if page == p0 { c0.min(len) } else { 0 };
        let end = if page == p1 { (c1 + 1).min(len) } else { len };
        (start < end).then_some(start..end)
    }

    /// The selected text, pages joined by line breaks. Pages not read yet
    /// are counted in the second value.
    pub fn text(&self, index: &TextIndex) -> (String, usize) {
        let ((p0, _), (p1, _)) = self.ordered();
        let mut out = String::new();
        let mut missing = 0;
        for page in p0..=p1.min(index.total().saturating_sub(1)) {
            match index.get(page) {
                Some(t) => {
                    let len = t.text.chars().count();
                    if let Some(r) = self.range_on(page, len) {
                        if !out.is_empty() && !out.ends_with('\n') {
                            out.push('\n');
                        }
                        out.push_str(&text::slice(&t, r));
                    }
                }
                None => missing += 1,
            }
        }
        (out, missing)
    }
}

/// A point on the page (points, page's own rotation applied) to the view,
/// which Rotate View turns by `rotation` degrees clockwise. `size` is the
/// page's size before the view rotation.
pub fn to_view(p: Pt, size: [f32; 2], rotation: u16) -> Pt {
    let [w, h] = size;
    match rotation % 360 {
        90 => [h - p[1], p[0]],
        180 => [w - p[0], h - p[1]],
        270 => [p[1], w - p[0]],
        _ => p,
    }
}

/// The inverse of [`to_view`].
pub fn from_view(v: Pt, size: [f32; 2], rotation: u16) -> Pt {
    let [w, h] = size;
    match rotation % 360 {
        90 => [v[1], h - v[0]],
        180 => [w - v[0], h - v[1]],
        270 => [w - v[1], v[0]],
        _ => v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_rotation_round_trips() {
        let size = [200.0, 100.0];
        for r in [0, 90, 180, 270] {
            let p = [30.0, 70.0];
            let v = to_view(p, size, r);
            assert_eq!(from_view(v, size, r), p, "rotation {r}");
        }
        // Clockwise: the top-left corner goes to the top-right.
        assert_eq!(to_view([0.0, 0.0], size, 90), [100.0, 0.0]);
    }

    #[test]
    fn selection_ranges_across_pages() {
        let s = Selection {
            anchor: (2, 5),
            head: (0, 3),
        };
        assert_eq!(s.range_on(0, 10), Some(3..10));
        assert_eq!(s.range_on(1, 7), Some(0..7));
        assert_eq!(s.range_on(2, 10), Some(0..6));
        assert_eq!(s.range_on(3, 10), None);
    }

    #[test]
    fn find_state_navigation_wraps() {
        let mut f = FindState {
            results: vec![
                Hit {
                    page: 0,
                    range: 0..1,
                },
                Hit {
                    page: 3,
                    range: 0..1,
                },
                Hit {
                    page: 5,
                    range: 0..1,
                },
            ],
            ..FindState::default()
        };
        assert_eq!(f.next(2).map(|h| h.page), Some(3));
        assert_eq!(f.next(2).map(|h| h.page), Some(5));
        assert_eq!(f.next(2).map(|h| h.page), Some(0));
        assert_eq!(f.previous(0).map(|h| h.page), Some(5));
    }
}
