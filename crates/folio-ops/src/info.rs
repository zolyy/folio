//! Document Properties (Description and Advanced), as Acrobat shows them.

use folio_core::{Document, Object, Result};

use crate::{json, labels, meta, outline, pages};

#[derive(Clone, Debug)]
pub struct PageInfo {
    pub number: usize,
    pub label: Option<String>,
    /// Displayed size in points (crop box, rotation applied).
    pub width: f64,
    pub height: f64,
    pub rotate: i64,
}

#[derive(Clone, Debug)]
pub struct DocInfo {
    pub version: String,
    pub file_size: Option<u64>,
    pub meta: meta::Meta,
    pub page_count: usize,
    pub pages: Vec<PageInfo>,
    pub tagged: bool,
    pub fast_web_view: bool,
    pub bookmarks: Vec<String>,
    pub repairs: Vec<String>,
}

pub fn info(doc: &Document, file_size: Option<u64>) -> Result<DocInfo> {
    let list = doc.pages()?;
    let labels = labels::per_page(doc, list.len());
    let page_infos = list
        .iter()
        .enumerate()
        .map(|(i, &p)| {
            let [w, h] = pages::displayed_size(doc, p);
            PageInfo {
                number: i + 1,
                label: labels.as_ref().and_then(|l| l.get(i)).map(|l| l.text()),
                width: w,
                height: h,
                rotate: pages::rotation(doc, p),
            }
        })
        .collect();
    let cat = doc.catalog()?;
    let tagged = doc
        .resolve_key(cat, b"MarkInfo")
        .and_then(Object::as_dict)
        .and_then(|m| m.get(b"Marked"))
        == Some(&Object::Bool(true));
    let fast_web_view = doc
        .objects
        .values()
        .any(|o| matches!(o, Object::Dict(d) if d.contains(b"Linearized")));
    Ok(DocInfo {
        version: doc.version.clone(),
        file_size,
        meta: meta::read(doc),
        page_count: list.len(),
        pages: page_infos,
        tagged,
        fast_web_view,
        bookmarks: outline::top_level(doc)
            .into_iter()
            .map(|b| b.title)
            .collect(),
        repairs: doc.repairs.clone(),
    })
}

/// Paper name for a size in points, as Acrobat shows next to the size.
pub fn paper_name(w: f64, h: f64) -> Option<&'static str> {
    let (a, b) = if w < h { (w, h) } else { (h, w) };
    let near = |x: f64, y: f64| (a - x).abs() < 2.0 && (b - y).abs() < 2.0;
    [
        (595.28, 841.89, "A4"),
        (419.53, 595.28, "A5"),
        (841.89, 1190.55, "A3"),
        (612.0, 792.0, "Letter"),
        (612.0, 1008.0, "Legal"),
        (792.0, 1224.0, "Tabloid"),
    ]
    .iter()
    .find(|(x, y, _)| near(*x, *y))
    .map(|(_, _, n)| *n)
}

impl DocInfo {
    pub fn to_json(&self) -> String {
        let m = &self.meta;
        let pages: Vec<String> = self
            .pages
            .iter()
            .map(|p| {
                json::obj(&[
                    ("number", p.number.to_string()),
                    ("label", json::opt(p.label.as_deref())),
                    ("width_pt", json::n(p.width)),
                    ("height_pt", json::n(p.height)),
                    ("rotate", p.rotate.to_string()),
                    ("paper", json::opt(paper_name(p.width, p.height))),
                ])
            })
            .collect();
        json::obj(&[
            ("ok", "true".into()),
            ("version", json::s(&self.version)),
            (
                "file_size",
                self.file_size.map_or("null".into(), |s| s.to_string()),
            ),
            ("title", json::opt(m.title.as_deref())),
            ("author", json::opt(m.author.as_deref())),
            ("subject", json::opt(m.subject.as_deref())),
            ("keywords", json::opt(m.keywords.as_deref())),
            ("application", json::opt(m.creator.as_deref())),
            ("producer", json::opt(m.producer.as_deref())),
            ("created", json::opt(m.created.as_deref())),
            ("modified", json::opt(m.modified.as_deref())),
            ("page_count", self.page_count.to_string()),
            ("tagged", self.tagged.to_string()),
            ("fast_web_view", self.fast_web_view.to_string()),
            (
                "bookmarks",
                json::arr(
                    &self
                        .bookmarks
                        .iter()
                        .map(|b| json::s(b))
                        .collect::<Vec<_>>(),
                ),
            ),
            ("pages", json::arr(&pages)),
            (
                "repairs",
                json::arr(&self.repairs.iter().map(|r| json::s(r)).collect::<Vec<_>>()),
            ),
        ])
    }

    pub fn to_text(&self) -> String {
        let m = &self.meta;
        let yn = |b: bool| if b { "Yes" } else { "No" };
        let dash = |v: &Option<String>| v.clone().unwrap_or_else(|| "—".into());
        let mut t = String::new();
        let mut line = |k: &str, v: String| t.push_str(&format!("{k:<15}{v}\n"));
        line("Title:", dash(&m.title));
        line("Author:", dash(&m.author));
        line("Subject:", dash(&m.subject));
        line("Keywords:", dash(&m.keywords));
        line("Created:", dash(&m.created));
        line("Modified:", dash(&m.modified));
        line("Application:", dash(&m.creator));
        line("PDF Producer:", dash(&m.producer));
        line("PDF Version:", self.version.clone());
        if let Some(s) = self.file_size {
            line("File Size:", format!("{} ({s} bytes)", human_size(s)));
        }
        if let Some(p) = self.pages.first() {
            let paper = paper_name(p.width, p.height)
                .map(|n| format!(" ({n})"))
                .unwrap_or_default();
            line(
                "Page Size:",
                format!("{:.2} x {:.2} in{paper}", p.width / 72.0, p.height / 72.0),
            );
        }
        line("Pages:", self.page_count.to_string());
        line("Tagged PDF:", yn(self.tagged).into());
        line("Fast Web View:", yn(self.fast_web_view).into());
        if !self.bookmarks.is_empty() {
            line("Bookmarks:", self.bookmarks.join(" | "));
        }
        if self.pages.iter().any(|p| p.label.is_some()) {
            let labels: Vec<String> = self
                .pages
                .iter()
                .take(20)
                .map(|p| p.label.clone().unwrap_or_default())
                .collect();
            line("Page Labels:", labels.join(", "));
        }
        for r in &self.repairs {
            line("Repaired:", r.clone());
        }
        t
    }
}

pub fn human_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1024.0 * 1024.0 {
        format!("{:.2} MB", b / (1024.0 * 1024.0))
    } else if b >= 1024.0 {
        format!("{:.2} KB", b / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
