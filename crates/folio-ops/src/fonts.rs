//! Document Properties → Fonts: every font the pages use, with the type,
//! encoding and embedding Acrobat shows (ISO 32000-2 §9.6–9.9).

use std::collections::HashSet;

use folio_core::{Dict, Document, Object};

const MAX_FONTS: usize = 10_000;
const MAX_FORM_DEPTH: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontInfo {
    /// BaseFont, without a subset prefix (`ABCDEF+`).
    pub name: String,
    /// "Type 1", "TrueType", "Type 3", "Type 1 (CID)", "TrueType (CID)", "OpenType".
    pub kind: String,
    /// "Ansi", "Roman", "Standard", "Custom", "Built-in", "Identity-H"...
    pub encoding: Option<String>,
    pub embedded: bool,
    pub subset: bool,
}

/// Fonts of every page and of the forms they use, each listed once.
pub fn list(doc: &Document) -> Vec<FontInfo> {
    let mut out = Vec::new();
    let mut seen_fonts = HashSet::new();
    let mut seen_res = HashSet::new();
    for &p in &doc.pages().unwrap_or_default() {
        if let Some(Object::Dict(res)) = doc.page_attr(p, b"Resources") {
            walk(doc, &res, 0, &mut seen_fonts, &mut seen_res, &mut out);
        }
        if out.len() >= MAX_FONTS {
            break;
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

fn walk(
    doc: &Document,
    res: &Dict,
    depth: usize,
    seen_fonts: &mut HashSet<Vec<u8>>,
    seen_res: &mut HashSet<u32>,
    out: &mut Vec<FontInfo>,
) {
    if depth > MAX_FORM_DEPTH {
        return;
    }
    if let Some(fonts) = doc.resolve_key(res, b"Font").and_then(Object::as_dict) {
        for (_, f) in fonts.iter() {
            if out.len() >= MAX_FONTS {
                return;
            }
            // Fonts are identified by object, or by content when direct.
            let key = match f.as_objref() {
                Some(r) => format!("ref {}", r.num).into_bytes(),
                None => folio_core::writer::serialize(f),
            };
            if !seen_fonts.insert(key) {
                continue;
            }
            if let Some(fd) = doc.resolve(f).as_dict() {
                out.push(describe(doc, fd));
            }
        }
    }
    if let Some(xo) = doc.resolve_key(res, b"XObject").and_then(Object::as_dict) {
        for (_, x) in xo.iter() {
            let Some(r) = x.as_objref() else { continue };
            if !seen_res.insert(r.num) {
                continue;
            }
            if let Some(Object::Stream(s)) = doc.get(r)
                && s.dict.get_name(b"Subtype") == Some(b"Form")
                && let Some(inner) = doc
                    .resolve_key(&s.dict, b"Resources")
                    .and_then(Object::as_dict)
            {
                walk(doc, inner, depth + 1, seen_fonts, seen_res, out);
            }
        }
    }
}

fn describe(doc: &Document, fd: &Dict) -> FontInfo {
    let subtype = fd.get_name(b"Subtype").unwrap_or_default();
    let base = fd
        .get_name(b"BaseFont")
        .map(|n| String::from_utf8_lossy(n).into_owned())
        .unwrap_or_else(|| "(unnamed)".into());
    let subset = base.len() > 7
        && base.as_bytes().get(6) == Some(&b'+')
        && base.bytes().take(6).all(|b| b.is_ascii_uppercase());
    let name = if subset {
        base.get(7..).unwrap_or(&base).to_string()
    } else {
        base
    };
    // For composite fonts the descriptor and kind come from the descendant.
    let descendant = match doc.resolve_key(fd, b"DescendantFonts") {
        Some(Object::Array(a)) => a.first().map(|d| doc.resolve(d)).and_then(Object::as_dict),
        _ => None,
    };
    let desc_owner = descendant.unwrap_or(fd);
    let descriptor = doc
        .resolve_key(desc_owner, b"FontDescriptor")
        .and_then(Object::as_dict);
    let file3 = descriptor.and_then(|d| doc.resolve_key(d, b"FontFile3"));
    let opentype = file3
        .and_then(Object::as_dict)
        .and_then(|d| d.get_name(b"Subtype"))
        == Some(b"OpenType");
    let embedded = subtype == b"Type3"
        || descriptor.is_some_and(|d| {
            d.contains(b"FontFile") || d.contains(b"FontFile2") || d.contains(b"FontFile3")
        });
    let kind = match subtype {
        b"Type0" => match descendant.and_then(|d| d.get_name(b"Subtype")) {
            Some(b"CIDFontType2") => "TrueType (CID)",
            _ if opentype => "OpenType (CID)",
            _ => "Type 1 (CID)",
        },
        b"TrueType" => "TrueType",
        b"Type3" => "Type 3",
        b"MMType1" => "Type 1 (MM)",
        _ if opentype => "OpenType",
        _ => "Type 1",
    }
    .to_string();
    let encoding = match doc.resolve_key(fd, b"Encoding") {
        Some(Object::Name(n)) => Some(match n.as_slice() {
            b"WinAnsiEncoding" => "Ansi".to_string(),
            b"MacRomanEncoding" => "Roman".to_string(),
            b"StandardEncoding" => "Standard".to_string(),
            b"MacExpertEncoding" => "Expert".to_string(),
            other => String::from_utf8_lossy(other).into_owned(),
        }),
        Some(Object::Dict(_) | Object::Stream(_)) => Some("Custom".to_string()),
        _ if embedded => Some("Built-in".to_string()),
        _ => None,
    };
    FontInfo {
        name,
        kind,
        encoding,
        embedded,
        subset,
    }
}
