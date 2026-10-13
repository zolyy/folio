//! Page labels (ISO 32000-2 §12.4.2), as in Acrobat's Page Labels dialog.

use std::collections::HashSet;

use folio_core::text_string::{decode_text, encode_text};
use folio_core::{Dict, Document, Error, Object, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Prefix only, no number.
    None,
    Decimal,
    RomanLower,
    RomanUpper,
    AlphaLower,
    AlphaUpper,
}

/// One section of numbering, starting at a page index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub start: usize,
    pub style: Style,
    pub prefix: String,
    pub first: i64,
}

/// The label of one page: which style/prefix it uses and its number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageLabel {
    pub style: Style,
    pub prefix: String,
    pub value: i64,
}

impl PageLabel {
    pub fn text(&self) -> String {
        format!("{}{}", self.prefix, format_number(self.style, self.value))
    }
}

pub fn format_number(style: Style, n: i64) -> String {
    match style {
        Style::None => String::new(),
        Style::Decimal => n.to_string(),
        Style::RomanLower => roman(n).to_lowercase(),
        Style::RomanUpper => roman(n),
        Style::AlphaLower => alpha(n).to_lowercase(),
        Style::AlphaUpper => alpha(n),
    }
}

fn roman(n: i64) -> String {
    if !(1..=4999).contains(&n) {
        return n.to_string();
    }
    let table = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut n = n;
    let mut s = String::new();
    for (v, r) in table {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

/// A..Z, then AA..ZZ, AAA.. (the letter repeats, §12.4.2).
fn alpha(n: i64) -> String {
    if !(1..=26 * 1000).contains(&n) {
        return n.to_string();
    }
    let letter = char::from(b'A' + ((n - 1) % 26) as u8);
    std::iter::repeat_n(letter, ((n - 1) / 26 + 1) as usize).collect()
}

fn style_from(name: Option<&[u8]>) -> Style {
    match name {
        Some(b"D") => Style::Decimal,
        Some(b"r") => Style::RomanLower,
        Some(b"R") => Style::RomanUpper,
        Some(b"a") => Style::AlphaLower,
        Some(b"A") => Style::AlphaUpper,
        _ => Style::None,
    }
}

fn style_name(s: Style) -> Option<&'static str> {
    match s {
        Style::None => None,
        Style::Decimal => Some("D"),
        Style::RomanLower => Some("r"),
        Style::RomanUpper => Some("R"),
        Style::AlphaLower => Some("a"),
        Style::AlphaUpper => Some("A"),
    }
}

/// The sections defined in the document, sorted by start page.
pub fn sections(doc: &Document) -> Vec<Section> {
    let Ok(cat) = doc.catalog() else {
        return Vec::new();
    };
    let Some(tree) = doc
        .resolve_key(cat, b"PageLabels")
        .and_then(Object::as_dict)
    else {
        return Vec::new();
    };
    let mut nums = Vec::new();
    let mut seen = HashSet::new();
    collect_nums(doc, tree, 0, &mut seen, &mut nums);
    let mut out: Vec<Section> = nums
        .into_iter()
        .filter_map(|(k, v)| {
            let start = usize::try_from(k).ok()?;
            let d = doc.resolve(&v).as_dict()?;
            Some(Section {
                start,
                style: style_from(d.get_name(b"S")),
                prefix: doc
                    .resolve_key(d, b"P")
                    .and_then(Object::as_str)
                    .map(decode_text)
                    .unwrap_or_default(),
                first: d.get_int(b"St").filter(|s| *s >= 1).unwrap_or(1),
            })
        })
        .collect();
    out.sort_by_key(|s| s.start);
    out.dedup_by_key(|s| s.start);
    out
}

fn collect_nums(
    doc: &Document,
    node: &Dict,
    depth: usize,
    seen: &mut HashSet<u32>,
    out: &mut Vec<(i64, Object)>,
) {
    if depth > 32 || out.len() > 1_000_000 {
        return;
    }
    if let Some(Object::Array(a)) = doc.resolve_key(node, b"Nums") {
        for pair in a.chunks(2) {
            if let (Some(k), Some(v)) = (pair.first().and_then(Object::as_int), pair.get(1)) {
                out.push((k, v.clone()));
            }
        }
    }
    if let Some(Object::Array(kids)) = doc.resolve_key(node, b"Kids") {
        for k in kids {
            if let Some(r) = k.as_objref()
                && seen.insert(r.num)
                && let Some(d) = doc.get(r).and_then(Object::as_dict)
            {
                collect_nums(doc, d, depth + 1, seen, out);
            }
        }
    }
}

/// Labels of every page, or `None` when the document defines none (then
/// readers show plain page numbers).
pub fn per_page(doc: &Document, page_count: usize) -> Option<Vec<PageLabel>> {
    let secs = sections(doc);
    if secs.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(page_count);
    for i in 0..page_count {
        let label = match secs.iter().rev().find(|s| s.start <= i) {
            Some(s) => PageLabel {
                style: s.style,
                prefix: s.prefix.clone(),
                value: s.first.saturating_add((i - s.start) as i64),
            },
            // Pages before the first section have no label (§12.4.2); show the number.
            None => PageLabel {
                style: Style::Decimal,
                prefix: String::new(),
                value: i as i64 + 1,
            },
        };
        out.push(label);
    }
    Some(out)
}

/// Write labels back as the smallest set of sections that reproduces them.
pub fn write(doc: &mut Document, labels: &[PageLabel]) -> Result<()> {
    let mut nums = Vec::new();
    let mut prev: Option<&PageLabel> = None;
    for (i, l) in labels.iter().enumerate() {
        let continues = prev.is_some_and(|p| {
            p.style == l.style && p.prefix == l.prefix && p.value.checked_add(1) == Some(l.value)
        });
        if !continues {
            let mut d = Dict::new();
            d.set("Type", Object::name("PageLabel"));
            if let Some(s) = style_name(l.style) {
                d.set("S", Object::name(s));
            }
            if !l.prefix.is_empty() {
                d.set("P", Object::Str(encode_text(&l.prefix)));
            }
            if l.value != 1 {
                d.set("St", Object::Int(l.value.max(1)));
            }
            nums.push(Object::Int(i as i64));
            nums.push(d.into());
        }
        prev = Some(l);
    }
    let mut tree = Dict::new();
    tree.set("Nums", Object::Array(nums));
    doc.catalog_mut()?.set("PageLabels", tree.into());
    Ok(())
}

/// What the Page Labels dialog asks for.
#[derive(Clone, Debug)]
pub enum LabelChange {
    /// "Begin new section".
    NewSection {
        style: Style,
        prefix: String,
        start: i64,
    },
    /// "Extend numbering used in preceding section to selected pages".
    Extend,
}

/// Apply a label change to the pages `first..=last` (0-based). Pages after
/// the range keep the labels they had.
pub fn apply(doc: &mut Document, first: usize, last: usize, change: &LabelChange) -> Result<()> {
    let count = doc.pages()?.len();
    if first > last || last >= count {
        return Err(Error::Invalid(format!(
            "page labels: pages {}-{} are outside the document's {count} pages",
            first + 1,
            last + 1
        )));
    }
    let mut labels = per_page(doc, count).unwrap_or_else(|| {
        (0..count)
            .map(|i| PageLabel {
                style: Style::Decimal,
                prefix: String::new(),
                value: i as i64 + 1,
            })
            .collect()
    });
    let base = match change {
        LabelChange::NewSection {
            style,
            prefix,
            start,
        } => PageLabel {
            style: *style,
            prefix: prefix.clone(),
            value: (*start).max(1),
        },
        LabelChange::Extend => match first.checked_sub(1).and_then(|p| labels.get(p)) {
            Some(prev) => PageLabel {
                value: prev.value.saturating_add(1),
                ..prev.clone()
            },
            None => PageLabel {
                style: Style::Decimal,
                prefix: String::new(),
                value: 1,
            },
        },
    };
    for (k, i) in (first..=last).enumerate() {
        if let Some(slot) = labels.get_mut(i) {
            *slot = PageLabel {
                value: base.value.saturating_add(k as i64),
                ..base.clone()
            };
        }
    }
    write(doc, &labels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_styles() {
        assert_eq!(format_number(Style::RomanLower, 14), "xiv");
        assert_eq!(format_number(Style::RomanUpper, 1999), "MCMXCIX");
        assert_eq!(format_number(Style::AlphaUpper, 28), "BB");
        assert_eq!(format_number(Style::AlphaLower, 1), "a");
        assert_eq!(format_number(Style::None, 5), "");
    }
}
