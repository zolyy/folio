//! Background page rendering. The window asks for the pages it can see each
//! frame; worker threads draw them with `folio-render` and the results are
//! kept, up to a memory budget, so scrolling back is instant. Requests the
//! window no longer needs are dropped before they start.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use folio_core::{Document, ObjRef};

/// Longest side of one rendered page image, in pixels. Larger views are
/// drawn at this size and stretched (Folio choice until tiled rendering).
pub const MAX_SIDE: u32 = 8192;
/// Pixels in one rendered page image.
pub const MAX_IMAGE_PIXELS: u64 = 40_000_000;
/// Memory kept for finished images, in bytes (Folio choice).
pub const BUDGET_BYTES: usize = 768 * 1024 * 1024;

/// What to draw: a page of a document, at a width, in a view rotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub doc: u64,
    pub page: usize,
    /// Target width in pixels, rotation applied.
    pub width: u32,
    /// View rotation, 0/90/180/270.
    pub rotation: u16,
}

/// A finished page: premultiplied RGBA, or why it could not be drawn.
#[derive(Debug)]
pub enum Done {
    Image {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    Failed(String),
}

impl Done {
    fn bytes(&self) -> usize {
        match self {
            Done::Image { rgba, .. } => rgba.len(),
            Done::Failed(m) => m.len(),
        }
    }
}

struct Job {
    key: Key,
    doc: Arc<Document>,
    page: ObjRef,
    /// Longest side allowed for this image (the GPU's texture limit).
    max_side: u32,
}

#[derive(Default)]
struct Shared {
    queue: VecDeque<Job>,
    /// Keys being drawn right now.
    running: Vec<Key>,
    finished: Vec<(Key, Done)>,
    stop: bool,
}

/// The workers and the finished pages, kept as `T` (the window keeps
/// GPU textures, tests keep the pixels) with their size in bytes.
pub struct Renderer<T> {
    shared: Arc<(Mutex<Shared>, Condvar)>,
    ready: HashMap<Key, (T, usize)>,
    /// Use order for eviction: most recently used last.
    order: VecDeque<Key>,
    bytes: usize,
    pending: HashSet<Key>,
    max_side: u32,
    workers: Vec<thread::JoinHandle<()>>,
}

fn lock(m: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    // A worker never panics while holding the lock (the renderer's own
    // guard catches panics), but recover the data rather than fail if one did.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl<T: Clone> Renderer<T> {
    /// Start `threads` workers; `notify` is called after each finished page
    /// (the window uses it to repaint).
    pub fn new(threads: usize, notify: Arc<dyn Fn() + Send + Sync>) -> Self {
        let shared = Arc::new((Mutex::new(Shared::default()), Condvar::new()));
        let workers = (0..threads.clamp(1, 8))
            .filter_map(|i| {
                let shared = shared.clone();
                let notify = notify.clone();
                thread::Builder::new()
                    .name(format!("folio-render-{i}"))
                    .spawn(move || work(&shared, &*notify))
                    .ok()
            })
            .collect();
        Renderer {
            shared,
            ready: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            pending: HashSet::new(),
            max_side: MAX_SIDE,
            workers,
        }
    }

    /// The longest image side the display can take (its maximum texture
    /// size); never more than `MAX_SIDE`.
    pub fn set_max_side(&mut self, side: usize) {
        self.max_side = u32::try_from(side).unwrap_or(u32::MAX).clamp(64, MAX_SIDE);
    }

    pub fn max_side(&self) -> u32 {
        self.max_side
    }

    /// Workers for this computer: all cores but one, at most four.
    pub fn default_threads() -> usize {
        thread::available_parallelism()
            .map(|n| n.get().saturating_sub(1))
            .unwrap_or(1)
            .clamp(1, 4)
    }

    /// Move finished pages from the workers into the cache, converting each
    /// with `convert`. Returns how many arrived.
    pub fn collect(&mut self, mut convert: impl FnMut(Key, Done) -> T) -> usize {
        let done = std::mem::take(&mut lock(&self.shared.0).finished);
        let n = done.len();
        for (key, d) in done {
            self.pending.remove(&key);
            let size = d.bytes();
            self.bytes = self.bytes.saturating_add(size);
            if let Some((_, old)) = self.ready.insert(key, (convert(key, d), size)) {
                self.bytes = self.bytes.saturating_sub(old);
            }
            self.touch(key);
        }
        self.evict();
        n
    }

    fn touch(&mut self, key: Key) {
        if let Some(i) = self.order.iter().position(|k| *k == key) {
            self.order.remove(i);
        }
        self.order.push_back(key);
    }

    fn evict(&mut self) {
        while self.bytes > BUDGET_BYTES && self.order.len() > 1 {
            let Some(k) = self.order.pop_front() else {
                break;
            };
            if let Some((_, old)) = self.ready.remove(&k) {
                self.bytes = self.bytes.saturating_sub(old);
            }
        }
    }

    /// The exact image if it is ready; otherwise the nearest-width image of
    /// the same page and rotation, so the window can show something while
    /// the sharper one renders. Marks what it returns as recently used.
    pub fn get(&mut self, key: Key) -> Option<(Key, T)> {
        let found = if self.ready.contains_key(&key) {
            Some(key)
        } else {
            self.ready
                .keys()
                .filter(|k| k.doc == key.doc && k.page == key.page && k.rotation == key.rotation)
                .min_by_key(|k| k.width.abs_diff(key.width))
                .copied()
        };
        let k = found?;
        let d = self.ready.get(&k)?.0.clone();
        self.touch(k);
        Some((k, d))
    }

    /// Ask for a page if it is neither ready nor on its way. Earlier requests
    /// in a frame are drawn first.
    pub fn request(&mut self, key: Key, doc: &Arc<Document>, page: ObjRef) {
        if self.ready.contains_key(&key) || self.pending.contains(&key) {
            return;
        }
        self.pending.insert(key);
        let (m, cv) = &*self.shared;
        lock(m).queue.push_back(Job {
            key,
            doc: doc.clone(),
            page,
            max_side: self.max_side,
        });
        cv.notify_one();
    }

    /// Drop queued requests that are not in `wanted` (pages scrolled away).
    pub fn retain(&mut self, wanted: &HashSet<Key>) {
        let mut s = lock(&self.shared.0);
        s.queue.retain(|j| wanted.contains(&j.key));
        self.pending = s
            .queue
            .iter()
            .map(|j| j.key)
            .chain(s.running.iter().copied())
            .chain(s.finished.iter().map(|f| f.0))
            .collect();
    }

    /// Forget every image of a document (its tab was closed).
    pub fn forget(&mut self, doc: u64) {
        self.ready.retain(|k, _| k.doc != doc);
        self.order.retain(|k| k.doc != doc);
        self.bytes = self.ready.values().map(|d| d.1).sum();
        lock(&self.shared.0).queue.retain(|j| j.key.doc != doc);
    }

    /// True while pages are queued or being drawn.
    pub fn busy(&self) -> bool {
        let s = lock(&self.shared.0);
        !s.queue.is_empty() || !s.running.is_empty() || !s.finished.is_empty()
    }

    /// Wait until every queued page is drawn (tests, scripts); `collect`
    /// then picks them up. Returns false on timeout.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let end = Instant::now() + timeout;
        let (m, cv) = &*self.shared;
        let mut s = lock(m);
        while !(s.queue.is_empty() && s.running.is_empty()) {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            s = match cv.wait_timeout(s, left) {
                Ok((g, _)) => g,
                Err(e) => e.into_inner().0,
            };
        }
        true
    }

    pub fn cached_bytes(&self) -> usize {
        self.bytes
    }
}

impl<T> Drop for Renderer<T> {
    fn drop(&mut self) {
        let (m, cv) = &*self.shared;
        {
            let mut s = lock(m);
            s.stop = true;
            s.queue.clear();
        }
        cv.notify_all();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

fn work(shared: &(Mutex<Shared>, Condvar), notify: &(dyn Fn() + Send + Sync)) {
    let (m, cv) = shared;
    loop {
        let job = {
            let mut s = lock(m);
            loop {
                if s.stop {
                    return;
                }
                if let Some(j) = s.queue.pop_front() {
                    s.running.push(j.key);
                    break j;
                }
                s = cv.wait(s).unwrap_or_else(|e| e.into_inner());
            }
        };
        let done = draw(&job);
        {
            let mut s = lock(m);
            if let Some(i) = s.running.iter().position(|k| *k == job.key) {
                s.running.swap_remove(i);
            }
            s.finished.push((job.key, done));
        }
        cv.notify_all();
        notify();
    }
}

/// Render one page at the requested width, then apply the view rotation.
fn draw(job: &Job) -> Done {
    let Key {
        width, rotation, ..
    } = job.key;
    // The width asked for is after view rotation: at 90/270 it is the page's height.
    let (w_pt, h_pt) = folio_render::page_size_points(&job.doc, job.page);
    let side_pt = if rotation % 180 == 90 { h_pt } else { w_pt };
    let side_pt = if side_pt.is_finite() {
        side_pt.max(1.0)
    } else {
        1.0
    };
    // One pixel of margin: sizes are rounded up after scaling.
    let max_side = f64::from(job.max_side.saturating_sub(1).max(1));
    let mut dpi = f64::from(width.max(1)).min(max_side) / side_pt * 72.0;
    // Keep the image within the pixel caps whatever the page's shape.
    let long_pt = w_pt.max(h_pt).max(1.0);
    dpi = dpi.min(max_side / long_pt * 72.0);
    let area = (w_pt * h_pt).max(1.0) * (dpi / 72.0) * (dpi / 72.0);
    if area > MAX_IMAGE_PIXELS as f64 {
        dpi *= (MAX_IMAGE_PIXELS as f64 / area).sqrt();
    }
    let dpi = dpi.clamp(1.0, 9600.0);
    let opts = folio_render::Options {
        dpi,
        ..Default::default()
    };
    match folio_render::render_page(&job.doc, job.page, &opts) {
        Ok(r) => {
            let (w, h) = (r.pixmap.width(), r.pixmap.height());
            let (rgba, width, height) = rotate(r.pixmap.data(), w, h, rotation);
            Done::Image {
                width,
                height,
                rgba,
            }
        }
        Err(e) => Done::Failed(e.to_string()),
    }
}

/// Rotate an RGBA image clockwise by 0, 90, 180 or 270 degrees.
pub fn rotate(src: &[u8], w: u32, h: u32, rotation: u16) -> (Vec<u8>, u32, u32) {
    let (wu, hu) = (w as usize, h as usize);
    if src.len() != wu.saturating_mul(hu).saturating_mul(4) {
        return (Vec::new(), 0, 0);
    }
    let (nw, nh) = match rotation % 360 {
        90 | 270 => (hu, wu),
        _ => (wu, hu),
    };
    if rotation.is_multiple_of(360) {
        return (src.to_vec(), w, h);
    }
    let mut out = vec![0u8; src.len()];
    for (i, px) in src.as_chunks::<4>().0.iter().enumerate() {
        let (x, y) = (i % wu, i / wu);
        let (nx, ny) = match rotation % 360 {
            90 => (hu - 1 - y, x),
            180 => (wu - 1 - x, hu - 1 - y),
            _ => (y, wu - 1 - x),
        };
        let o = (ny * nw + nx) * 4;
        if let Some(d) = out.get_mut(o..o + 4) {
            d.copy_from_slice(px);
        }
    }
    (out, nw as u32, nh as u32)
}
