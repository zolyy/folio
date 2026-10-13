//! Combine Files. Like Acrobat Pro, every source file gets a bookmark named
//! after it, and that file's own bookmarks are kept underneath it.

use folio_core::text_string::encode_text;
use folio_core::{Dict, Document, Error, ObjRef, Object, Result};

use crate::outline;

pub struct Input {
    pub doc: Document,
    /// Shown as the file's bookmark title (Acrobat uses the file name).
    pub name: String,
    /// The pages to take (0-based), or all of them.
    pub pages: Option<Vec<usize>>,
}

pub struct Options {
    /// Add a bookmark per file (Acrobat's default).
    pub bookmarks: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { bookmarks: true }
    }
}

struct FileMark {
    title: String,
    first_page: ObjRef,
    children: Vec<ObjRef>,
}

pub fn merge(inputs: Vec<Input>, opts: &Options) -> Result<Document> {
    if inputs.is_empty() {
        return Err(Error::Invalid("give at least one file to combine".into()));
    }
    let mut out = Document::new();
    out.version = inputs
        .iter()
        .map(|i| i.doc.version.clone())
        .max_by(|a, b| {
            let fa: f64 = a.parse().unwrap_or(0.0);
            let fb: f64 = b.parse().unwrap_or(0.0);
            fa.total_cmp(&fb)
        })
        .unwrap_or_else(|| "1.7".into());
    let mut all_pages = Vec::new();
    let mut marks = Vec::new();
    for mut input in inputs {
        let name = input.name;
        outline::make_dests_explicit(&mut input.doc);
        // Bookmarks of this file, by object number before renumbering.
        let own_marks: Vec<ObjRef> = outline::top_level(&input.doc)
            .into_iter()
            .map(|b| b.item)
            .collect();
        let offset = out.max_num();
        let pages = out.import(input.doc)?;
        let chosen: Vec<ObjRef> = match &input.pages {
            Some(idx) => idx
                .iter()
                .map(|&i| {
                    pages.get(i).copied().ok_or_else(|| {
                        Error::Invalid(format!("{name}: there is no page {}", i + 1))
                    })
                })
                .collect::<Result<_>>()?,
            None => pages,
        };
        let Some(&first) = chosen.first() else {
            return Err(Error::Invalid(format!("{name}: no pages selected")));
        };
        marks.push(FileMark {
            title: name,
            first_page: first,
            children: own_marks
                .iter()
                .map(|r| ObjRef::new(r.num.saturating_add(offset), 0))
                .collect(),
        });
        all_pages.extend(chosen);
    }
    out.set_pages(&all_pages)?;
    if opts.bookmarks {
        build_outline(&mut out, &marks)?;
    }
    out.prune();
    Ok(out)
}

fn build_outline(doc: &mut Document, marks: &[FileMark]) -> Result<()> {
    let root = doc.add(Object::Null);
    let items: Vec<ObjRef> = marks.iter().map(|_| doc.add(Object::Null)).collect();
    for (k, (m, &item)) in marks.iter().zip(&items).enumerate() {
        let mut d = Dict::new();
        d.set("Title", Object::Str(encode_text(&m.title)));
        d.set("Parent", root.into());
        d.set(
            "Dest",
            Object::Array(vec![
                m.first_page.into(),
                Object::name("XYZ"),
                Object::Null,
                Object::Null,
                Object::Null,
            ]),
        );
        if k > 0
            && let Some(&p) = items.get(k - 1)
        {
            d.set("Prev", p.into());
        }
        if let Some(&n) = items.get(k + 1) {
            d.set("Next", n.into());
        }
        if let (Some(&first), Some(&last)) = (m.children.first(), m.children.last()) {
            d.set("First", first.into());
            d.set("Last", last.into());
            // Negative: the file's bookmarks start collapsed, as in Acrobat.
            d.set("Count", Object::Int(-(m.children.len() as i64)));
            for &c in &m.children {
                if let Some(cd) = doc.get_mut(c).and_then(Object::as_dict_mut) {
                    cd.set("Parent", item.into());
                }
            }
        }
        doc.objects.insert(item.num, d.into());
    }
    let mut r = Dict::new();
    r.set("Type", Object::name("Outlines"));
    if let (Some(&f), Some(&l)) = (items.first(), items.last()) {
        r.set("First", f.into());
        r.set("Last", l.into());
    }
    r.set("Count", Object::Int(items.len() as i64));
    doc.objects.insert(root.num, r.into());
    let cat = doc.catalog_mut()?;
    cat.set("Outlines", root.into());
    cat.set("PageMode", Object::name("UseOutlines"));
    Ok(())
}
