//! Reading bookmarks (the document outline, §12.3.3) and resolving their
//! destinations, including named destinations (§12.3.2.4).

use std::collections::HashSet;

use folio_core::text_string::decode_text;
use folio_core::{Dict, Document, ObjRef, Object};

/// More bookmarks than this at one level are ignored (hostile files loop).
const MAX_ITEMS: usize = 100_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Bookmark {
    pub title: String,
    /// The page the bookmark goes to, if it can be resolved.
    pub page: Option<ObjRef>,
    /// The bookmark's own object.
    pub item: ObjRef,
}

/// The first-level bookmarks, in order.
pub fn top_level(doc: &Document) -> Vec<Bookmark> {
    let Ok(cat) = doc.catalog() else {
        return Vec::new();
    };
    let Some(root) = doc.resolve_key(cat, b"Outlines").and_then(Object::as_dict) else {
        return Vec::new();
    };
    siblings(doc, root.get(b"First").and_then(Object::as_objref))
        .into_iter()
        .filter_map(|r| {
            let d = doc.get(r)?.as_dict()?;
            Some(Bookmark {
                title: doc
                    .resolve_key(d, b"Title")
                    .and_then(Object::as_str)
                    .map(decode_text)
                    .unwrap_or_default(),
                page: item_page(doc, d),
                item: r,
            })
        })
        .collect()
}

/// An outline item and the items after it, following `/Next`.
pub fn siblings(doc: &Document, first: Option<ObjRef>) -> Vec<ObjRef> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut cur = first;
    while let Some(r) = cur {
        if !seen.insert(r.num) || out.len() >= MAX_ITEMS {
            break;
        }
        let Some(d) = doc.get(r).and_then(Object::as_dict) else {
            break;
        };
        out.push(r);
        cur = d.get(b"Next").and_then(Object::as_objref);
    }
    out
}

/// Every outline item in the tree (depth-first, bounded).
pub fn all_items(doc: &Document) -> Vec<ObjRef> {
    let Ok(cat) = doc.catalog() else {
        return Vec::new();
    };
    let Some(root) = doc.resolve_key(cat, b"Outlines").and_then(Object::as_dict) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut stack: Vec<ObjRef> = root
        .get(b"First")
        .and_then(Object::as_objref)
        .into_iter()
        .collect();
    while let Some(r) = stack.pop() {
        if !seen.insert(r.num) || out.len() >= MAX_ITEMS {
            continue;
        }
        let Some(d) = doc.get(r).and_then(Object::as_dict) else {
            continue;
        };
        out.push(r);
        if let Some(n) = d.get(b"Next").and_then(Object::as_objref) {
            stack.push(n);
        }
        if let Some(c) = d.get(b"First").and_then(Object::as_objref) {
            stack.push(c);
        }
    }
    out
}

/// Replace named destinations in bookmarks with explicit ones, so the
/// bookmarks keep working when the pages move to another document.
pub fn make_dests_explicit(doc: &mut Document) {
    for item in all_items(doc) {
        let Some(d) = doc.get(item).and_then(Object::as_dict) else {
            continue;
        };
        let named = match d.get(b"Dest").map(|o| doc.resolve(o)) {
            Some(Object::Name(_) | Object::Str(_)) => d.get(b"Dest").cloned(),
            _ => doc
                .resolve_key(d, b"A")
                .and_then(Object::as_dict)
                .filter(|a| a.get_name(b"S") == Some(b"GoTo"))
                .and_then(|a| a.get(b"D"))
                .filter(|o| matches!(doc.resolve(o), Object::Name(_) | Object::Str(_)))
                .cloned(),
        };
        let Some(named) = named else { continue };
        let Some(page) = dest_page(doc, &named, 0) else {
            continue;
        };
        if let Some(d) = doc.get_mut(item).and_then(Object::as_dict_mut) {
            d.remove(b"A");
            d.set(
                "Dest",
                Object::Array(vec![page.into(), Object::name("Fit")]),
            );
        }
    }
}

fn item_page(doc: &Document, item: &Dict) -> Option<ObjRef> {
    if let Some(dest) = item.get(b"Dest") {
        return dest_page(doc, dest, 0);
    }
    let action = doc.resolve_key(item, b"A")?.as_dict()?;
    if action.get_name(b"S") == Some(b"GoTo") {
        return dest_page(doc, action.get(b"D")?, 0);
    }
    None
}

/// The page of a destination: an explicit array, or a name to look up.
pub fn dest_page(doc: &Document, dest: &Object, depth: usize) -> Option<ObjRef> {
    if depth > 8 {
        return None;
    }
    match doc.resolve(dest) {
        Object::Array(a) => a.first()?.as_objref(),
        Object::Dict(d) => dest_page(doc, d.get(b"D")?, depth + 1),
        Object::Name(n) | Object::Str(n) => {
            let target = named_dest(doc, n)?;
            dest_page(doc, &target, depth + 1)
        }
        _ => None,
    }
}

fn named_dest(doc: &Document, name: &[u8]) -> Option<Object> {
    let cat = doc.catalog().ok()?;
    // PDF 1.1 style: /Dests dictionary in the catalog.
    if let Some(d) = doc.resolve_key(cat, b"Dests").and_then(Object::as_dict)
        && let Some(v) = d.get(name)
    {
        return Some(v.clone());
    }
    // PDF 1.2+: /Names /Dests name tree.
    let names = doc.resolve_key(cat, b"Names")?.as_dict()?;
    let tree = doc.resolve_key(names, b"Dests")?.as_dict()?;
    let mut seen = HashSet::new();
    lookup_name_tree(doc, tree, name, 0, &mut seen)
}

fn lookup_name_tree(
    doc: &Document,
    node: &Dict,
    key: &[u8],
    depth: usize,
    seen: &mut HashSet<u32>,
) -> Option<Object> {
    if depth > 32 {
        return None;
    }
    if let Some(Object::Array(a)) = doc.resolve_key(node, b"Names") {
        for pair in a.chunks(2) {
            if let (Some(k), Some(v)) = (pair.first().map(|k| doc.resolve(k)), pair.get(1))
                && k.as_str() == Some(key)
            {
                return Some(v.clone());
            }
        }
    }
    if let Some(Object::Array(kids)) = doc.resolve_key(node, b"Kids") {
        for k in kids {
            let Some(r) = k.as_objref() else { continue };
            if !seen.insert(r.num) {
                continue;
            }
            if let Some(d) = doc.get(r).and_then(Object::as_dict)
                && let Some(v) = lookup_name_tree(doc, d, key, depth + 1, seen)
            {
                return Some(v);
            }
        }
    }
    None
}
