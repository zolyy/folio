//! File attachments (ISO 32000-2 §7.11.4, §12.5.6.15): the embedded files of
//! the document's name tree and of FileAttachment annotations, as Acrobat's
//! Attachments panel lists them, and their contents.

use std::collections::HashSet;

use folio_core::filters::decode_stream;
use folio_core::text_string::decode_text;
use folio_core::{Dict, Document, Error, Object, Result};

use crate::meta::pretty_date;

/// Bounds for hostile files.
const MAX_ITEMS: usize = 10_000;
const MAX_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq)]
pub struct Attachment {
    pub name: String,
    pub description: Option<String>,
    /// Uncompressed size, from /Params /Size when given.
    pub size: Option<u64>,
    pub modified: Option<String>,
    /// For attachments in a FileAttachment comment: the page (0-based).
    pub page: Option<usize>,
    spec: Object,
}

/// Every attachment: document-level ones first (in name-tree order), then
/// those in comments, page by page.
pub fn list(doc: &Document) -> Vec<Attachment> {
    let mut out = Vec::new();
    if let Ok(cat) = doc.catalog()
        && let Some(names) = doc.resolve_key(cat, b"Names").and_then(Object::as_dict)
        && let Some(tree) = doc
            .resolve_key(names, b"EmbeddedFiles")
            .and_then(Object::as_dict)
    {
        let mut pairs = Vec::new();
        let mut seen = HashSet::new();
        name_tree(doc, tree, 0, &mut seen, &mut pairs);
        for (key, spec) in pairs {
            if let Some(a) = describe(doc, &spec, Some(&key), None) {
                out.push(a);
            }
        }
    }
    for (i, &p) in doc.pages().unwrap_or_default().iter().enumerate() {
        let Some(pd) = doc.get(p).and_then(Object::as_dict) else {
            continue;
        };
        let Some(Object::Array(annots)) = doc.resolve_key(pd, b"Annots") else {
            continue;
        };
        for a in annots.iter().take(MAX_ITEMS) {
            if out.len() >= MAX_ITEMS {
                return out;
            }
            let Some(ad) = doc.resolve(a).as_dict() else {
                continue;
            };
            if ad.get_name(b"Subtype") != Some(b"FileAttachment") {
                continue;
            }
            if let Some(fs) = ad.get(b"FS")
                && let Some(mut att) = describe(doc, fs, None, Some(i))
            {
                if att.description.is_none() {
                    att.description = doc
                        .resolve_key(ad, b"Contents")
                        .and_then(Object::as_str)
                        .map(decode_text)
                        .filter(|s| !s.is_empty());
                }
                out.push(att);
            }
        }
    }
    out
}

fn name_tree(
    doc: &Document,
    node: &Dict,
    depth: usize,
    seen: &mut HashSet<u32>,
    out: &mut Vec<(Vec<u8>, Object)>,
) {
    if depth > MAX_DEPTH || out.len() >= MAX_ITEMS {
        return;
    }
    if let Some(Object::Array(a)) = doc.resolve_key(node, b"Names") {
        for pair in a.chunks(2) {
            if let (Some(k), Some(v)) = (pair.first().map(|k| doc.resolve(k)), pair.get(1))
                && let Some(k) = k.as_str()
            {
                out.push((k.to_vec(), v.clone()));
            }
        }
    }
    if let Some(Object::Array(kids)) = doc.resolve_key(node, b"Kids") {
        for k in kids {
            let Some(r) = k.as_objref() else { continue };
            if seen.insert(r.num)
                && let Some(d) = doc.get(r).and_then(Object::as_dict)
            {
                name_tree(doc, d, depth + 1, seen, out);
            }
        }
    }
}

fn embedded_stream<'a>(doc: &'a Document, spec: &'a Dict) -> Option<&'a folio_core::Stream> {
    let ef = doc.resolve_key(spec, b"EF")?.as_dict()?;
    ef.get(b"UF")
        .or_else(|| ef.get(b"F"))
        .map(|o| doc.resolve(o))
        .and_then(Object::as_stream)
}

fn describe(
    doc: &Document,
    spec: &Object,
    key: Option<&[u8]>,
    page: Option<usize>,
) -> Option<Attachment> {
    let d = doc.resolve(spec).as_dict()?;
    let stream = embedded_stream(doc, d)?;
    let text = |k: &[u8]| {
        doc.resolve_key(d, k)
            .and_then(Object::as_str)
            .map(decode_text)
            .filter(|s| !s.trim().is_empty())
    };
    let name = text(b"UF")
        .or_else(|| text(b"F"))
        .or_else(|| key.map(decode_text))
        .unwrap_or_else(|| "attachment".into());
    let params = doc
        .resolve_key(&stream.dict, b"Params")
        .and_then(Object::as_dict);
    let size = params
        .and_then(|p| p.get_int(b"Size"))
        .and_then(|s| u64::try_from(s).ok());
    let modified = params
        .and_then(|p| doc.resolve_key(p, b"ModDate"))
        .and_then(Object::as_str)
        .map(|s| pretty_date(&decode_text(s)));
    Some(Attachment {
        name,
        description: text(b"Desc"),
        size,
        modified,
        page,
        spec: spec.clone(),
    })
}

/// The attachment's file contents, decoded.
pub fn contents(doc: &Document, a: &Attachment) -> Result<Vec<u8>> {
    let d = doc
        .resolve(&a.spec)
        .as_dict()
        .ok_or_else(|| Error::Invalid("the attachment's file specification is damaged".into()))?;
    let s = embedded_stream(doc, d)
        .ok_or_else(|| Error::Invalid(format!("'{}' has no embedded file", a.name)))?;
    decode_stream(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_without_attachments_list_nothing() {
        let mut doc = Document::new();
        doc.add_text_page("x").unwrap();
        assert!(list(&doc).is_empty());
    }
}
