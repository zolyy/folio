//! Organize Pages: rotate, delete, extract, insert, replace, reverse,
//! reorder and crop — with the options Acrobat Pro offers for each
//! (`docs/acrobat/organize.md`). Page labels follow the pages they belong to.

use folio_core::document::{num, rect};
use folio_core::{Dict, Document, Error, ObjRef, Object, Result};

use crate::labels::{self, PageLabel, Style};

/// Acrobat's "Even and Odd Pages / Even Pages Only / Odd Pages Only".
/// Even and odd refer to page numbers counted from 1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Subset {
    #[default]
    All,
    Even,
    Odd,
}

/// Acrobat's "Landscape and Portrait / Landscape Pages / Portrait Pages",
/// judged on the page as displayed (after its rotation). Square pages count
/// as portrait (**Folio choice**).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orientation {
    #[default]
    Any,
    Landscape,
    Portrait,
}

/// Keep the pages of `idx` that match `subset` and `orientation`.
pub fn filter(
    doc: &Document,
    idx: &[usize],
    subset: Subset,
    orientation: Orientation,
) -> Result<Vec<usize>> {
    let pages = doc.pages()?;
    Ok(idx
        .iter()
        .copied()
        .filter(|&i| match subset {
            Subset::All => true,
            Subset::Even => (i + 1) % 2 == 0,
            Subset::Odd => (i + 1) % 2 == 1,
        })
        .filter(|&i| {
            let Some(&p) = pages.get(i) else { return false };
            let [w, h] = displayed_size(doc, p);
            match orientation {
                Orientation::Any => true,
                Orientation::Landscape => w > h,
                Orientation::Portrait => h >= w,
            }
        })
        .collect())
}

/// The page's rotation, normalised to 0, 90, 180 or 270.
pub fn rotation(doc: &Document, page: ObjRef) -> i64 {
    let r = doc
        .page_attr(page, b"Rotate")
        .and_then(|o| o.as_int())
        .unwrap_or(0);
    let r = r.rem_euclid(360);
    r - r % 90
}

/// Width and height as displayed (CropBox if present, rotation applied).
pub fn displayed_size(doc: &Document, page: ObjRef) -> [f64; 2] {
    let b = visible_box(doc, page);
    let (w, h) = ((b[2] - b[0]).abs(), (b[3] - b[1]).abs());
    if rotation(doc, page) % 180 == 90 {
        [h, w]
    } else {
        [w, h]
    }
}

fn visible_box(doc: &Document, page: ObjRef) -> [f64; 4] {
    match doc
        .page_attr(page, b"CropBox")
        .and_then(|o| box_of(doc, &o))
    {
        Some(b) => b,
        None => normalized(doc.media_box(page)),
    }
}

fn box_of(doc: &Document, o: &Object) -> Option<[f64; 4]> {
    let a = o.as_array()?;
    let v: Vec<f64> = a.iter().filter_map(|x| doc.resolve(x).as_f64()).collect();
    match v.as_slice() {
        [a, b, c, d] => Some(normalized([*a, *b, *c, *d])),
        _ => None,
    }
}

fn normalized(b: [f64; 4]) -> [f64; 4] {
    [
        b[0].min(b[2]),
        b[1].min(b[3]),
        b[0].max(b[2]),
        b[1].max(b[3]),
    ]
}

/// Rotate pages by a multiple of 90 degrees (positive = clockwise, as
/// `/Rotate` is defined). Returns how many pages were rotated.
pub fn rotate(doc: &mut Document, idx: &[usize], degrees: i64) -> Result<usize> {
    if degrees % 90 != 0 {
        return Err(Error::Invalid(format!(
            "pages can only be rotated by multiples of 90 degrees, not {degrees}"
        )));
    }
    let pages = doc.materialize_pages()?;
    let mut n = 0;
    for &i in idx {
        let Some(&p) = pages.get(i) else { continue };
        let new = (rotation(doc, p) + degrees).rem_euclid(360);
        if let Some(d) = doc.get_mut(p).and_then(Object::as_dict_mut) {
            if new == 0 {
                d.remove(b"Rotate");
            } else {
                d.set("Rotate", Object::Int(new));
            }
            n += 1;
        }
    }
    Ok(n)
}

/// Where a page in the new order comes from.
#[derive(Clone, Copy, Debug)]
enum Slot {
    Own(usize),
    New(ObjRef),
}

/// Rebuild the page tree from `slots`, carry page labels along, drop
/// objects nothing uses any more.
fn rearrange(doc: &mut Document, pages: &[ObjRef], slots: &[Slot]) -> Result<()> {
    if slots.is_empty() {
        return Err(Error::Invalid(
            "this would leave a document with no pages; at least one page must remain".into(),
        ));
    }
    let old_labels = labels::per_page(doc, pages.len());
    let refs = slots
        .iter()
        .map(|s| match *s {
            Slot::Own(i) => pages
                .get(i)
                .copied()
                .ok_or_else(|| Error::Invalid(format!("page {} does not exist", i + 1))),
            Slot::New(r) => Ok(r),
        })
        .collect::<Result<Vec<_>>>()?;
    doc.set_pages(&refs)?;
    if let Some(old) = old_labels {
        let new: Vec<PageLabel> = slots
            .iter()
            .enumerate()
            .map(|(k, s)| {
                match *s {
                    Slot::Own(i) => old.get(i).cloned(),
                    Slot::New(_) => None,
                }
                .unwrap_or(PageLabel {
                    style: Style::Decimal,
                    prefix: String::new(),
                    value: k as i64 + 1,
                })
            })
            .collect();
        labels::write(doc, &new)?;
    }
    doc.prune();
    Ok(())
}

/// Delete the given pages. Deleting all of them is refused, as in Acrobat.
pub fn delete(doc: &mut Document, idx: &[usize]) -> Result<()> {
    let pages = doc.materialize_pages()?;
    let gone: std::collections::HashSet<usize> = idx.iter().copied().collect();
    let keep: Vec<Slot> = (0..pages.len())
        .filter(|i| !gone.contains(i))
        .map(Slot::Own)
        .collect();
    if keep.is_empty() {
        return Err(Error::Invalid(
            "you cannot delete all pages; at least one page must remain".into(),
        ));
    }
    rearrange(doc, &pages, &keep)
}

/// Keep only the given pages, in the given order (Extract, and reordering).
pub fn select(doc: &mut Document, idx: &[usize]) -> Result<()> {
    let pages = doc.materialize_pages()?;
    let slots: Vec<Slot> = idx.iter().map(|&i| Slot::Own(i)).collect();
    rearrange(doc, &pages, &slots)
}

/// Reverse the page order.
pub fn reverse(doc: &mut Document) -> Result<()> {
    let n = doc.pages()?.len();
    let order: Vec<usize> = (0..n).rev().collect();
    select(doc, &order)
}

/// Put every page in a new order. `order` must name each page exactly once.
pub fn reorder(doc: &mut Document, order: &[usize]) -> Result<()> {
    let n = doc.pages()?.len();
    let mut seen = vec![false; n];
    for &i in order {
        match seen.get_mut(i) {
            Some(s) if !*s => *s = true,
            Some(_) => {
                return Err(Error::Invalid(format!(
                    "page {} appears twice in the new order",
                    i + 1
                )));
            }
            None => return Err(Error::Invalid(format!("page {} does not exist", i + 1))),
        }
    }
    if let Some(missing) = seen.iter().position(|s| !s) {
        return Err(Error::Invalid(format!(
            "the new order must list every page once; page {} is missing",
            missing + 1
        )));
    }
    select(doc, order)
}

/// Insert pages of `other` (all, or `other_idx`) so they start at position
/// `at` (0 = before the first page, page count = after the last).
pub fn insert(
    doc: &mut Document,
    other: Document,
    other_idx: Option<&[usize]>,
    at: usize,
) -> Result<usize> {
    let pages = doc.materialize_pages()?;
    if at > pages.len() {
        return Err(Error::Invalid(format!(
            "cannot insert at position {}; the document has {} pages",
            at,
            pages.len()
        )));
    }
    let theirs = doc.import(other)?;
    let chosen: Vec<ObjRef> = match other_idx {
        Some(idx) => idx
            .iter()
            .map(|&i| {
                theirs.get(i).copied().ok_or_else(|| {
                    Error::Invalid(format!("the inserted file has no page {}", i + 1))
                })
            })
            .collect::<Result<_>>()?,
        None => theirs,
    };
    let n = chosen.len();
    let mut slots: Vec<Slot> = (0..at).map(Slot::Own).collect();
    slots.extend(chosen.into_iter().map(Slot::New));
    slots.extend((at..pages.len()).map(Slot::Own));
    rearrange(doc, &pages, &slots)?;
    Ok(n)
}

/// Insert `count` blank pages at `at`, the same size as the page they are
/// placed next to (as Acrobat does).
pub fn insert_blank(doc: &mut Document, at: usize, count: usize) -> Result<()> {
    let pages = doc.materialize_pages()?;
    if at > pages.len() {
        return Err(Error::Invalid(format!(
            "cannot insert at position {}; the document has {} pages",
            at,
            pages.len()
        )));
    }
    if count == 0 || count > 10_000 {
        return Err(Error::Invalid(
            "the number of blank pages must be between 1 and 10000".into(),
        ));
    }
    let neighbour = pages
        .get(at.saturating_sub(1))
        .or_else(|| pages.get(at))
        .copied();
    let media = neighbour
        .map(|p| doc.media_box(p))
        .unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let rot = neighbour.map(|p| rotation(doc, p)).unwrap_or(0);
    let mut slots: Vec<Slot> = (0..at).map(Slot::Own).collect();
    for _ in 0..count {
        let mut d = Dict::new();
        d.set("Type", Object::name("Page"));
        d.set("MediaBox", rect(media));
        d.set("Resources", Dict::new().into());
        if rot != 0 {
            d.set("Rotate", Object::Int(rot));
        }
        slots.push(Slot::New(doc.add(d.into())));
    }
    slots.extend((at..pages.len()).map(Slot::Own));
    rearrange(doc, &pages, &slots)
}

/// Replace the pages `idx` with as many pages of `other`, starting at its
/// page `from_start` (0-based) — Acrobat's Replace Pages.
pub fn replace(
    doc: &mut Document,
    idx: &[usize],
    other: Document,
    from_start: usize,
) -> Result<()> {
    let pages = doc.materialize_pages()?;
    let theirs = doc.import(other)?;
    let need = idx.len();
    let avail = theirs.len().saturating_sub(from_start);
    if avail < need {
        return Err(Error::Invalid(format!(
            "replacing {need} page(s) needs {need} page(s) from the other file starting at page {}, but it only has {avail}",
            from_start + 1
        )));
    }
    let mut slots: Vec<Slot> = (0..pages.len()).map(Slot::Own).collect();
    for (k, &i) in idx.iter().enumerate() {
        let (Some(slot), Some(&r)) = (slots.get_mut(i), theirs.get(from_start + k)) else {
            return Err(Error::Invalid(format!("page {} does not exist", i + 1)));
        };
        *slot = Slot::New(r);
    }
    rearrange(doc, &pages, &slots)
}

/// The page boxes Acrobat's Crop Pages dialog can set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageBox {
    Crop,
    Art,
    Trim,
    Bleed,
}

impl PageBox {
    pub fn key(self) -> &'static str {
        match self {
            PageBox::Crop => "CropBox",
            PageBox::Art => "ArtBox",
            PageBox::Trim => "TrimBox",
            PageBox::Bleed => "BleedBox",
        }
    }
}

/// Margins as Acrobat shows them: top, bottom, left, right, in points,
/// relative to the page as displayed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Margins {
    pub top: f64,
    pub bottom: f64,
    pub left: f64,
    pub right: f64,
}

/// Set a page box inset from the MediaBox by `m`, on the given pages.
pub fn crop(doc: &mut Document, idx: &[usize], which: PageBox, m: Margins) -> Result<usize> {
    for v in [m.top, m.bottom, m.left, m.right] {
        if !v.is_finite() || v < 0.0 {
            return Err(Error::Invalid(
                "crop margins must be zero or positive numbers".into(),
            ));
        }
    }
    let pages = doc.materialize_pages()?;
    let mut n = 0;
    for &i in idx {
        let Some(&p) = pages.get(i) else { continue };
        let mb = normalized(doc.media_box(p));
        // Map displayed sides onto the unrotated page.
        let (mt, mbm, ml, mr) = match rotation(doc, p) {
            90 => (m.right, m.left, m.top, m.bottom),
            180 => (m.bottom, m.top, m.right, m.left),
            270 => (m.left, m.right, m.bottom, m.top),
            _ => (m.top, m.bottom, m.left, m.right),
        };
        let b = [mb[0] + ml, mb[1] + mbm, mb[2] - mr, mb[3] - mt];
        if b[2] - b[0] < 1.0 || b[3] - b[1] < 1.0 {
            return Err(Error::Invalid(format!(
                "the margins leave nothing of page {} ({} x {} pt)",
                i + 1,
                num_text(mb[2] - mb[0]),
                num_text(mb[3] - mb[1])
            )));
        }
        if let Some(d) = doc.get_mut(p).and_then(Object::as_dict_mut) {
            d.set(which.key(), rect(b));
            n += 1;
        }
    }
    Ok(n)
}

/// Acrobat's "Set to zero": remove the box so it falls back to its default.
pub fn reset_box(doc: &mut Document, idx: &[usize], which: PageBox) -> Result<usize> {
    let pages = doc.materialize_pages()?;
    let mut n = 0;
    for &i in idx {
        if let Some(d) = pages
            .get(i)
            .and_then(|&p| doc.get_mut(p))
            .and_then(Object::as_dict_mut)
            && d.remove(which.key().as_bytes()).is_some()
        {
            n += 1;
        }
    }
    Ok(n)
}

fn num_text(v: f64) -> String {
    folio_core::writer::serialize(&num((v * 100.0).round() / 100.0))
        .into_iter()
        .map(char::from)
        .collect()
}
