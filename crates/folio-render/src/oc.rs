//! Optional content — Acrobat's Layers (ISO 32000-2 §8.11): the layer list
//! in the order of the document's /Order tree, the default on/off state from
//! the /D configuration, and visibility of optional-content groups (OCG) and
//! membership dictionaries (OCMD, including visibility expressions).

use std::collections::{HashMap, HashSet};

use folio_core::text_string::decode_text;
use folio_core::{Dict, Document, ObjRef, Object};

/// Bounds for hostile files.
const MAX_LAYERS: usize = 10_000;
const MAX_DEPTH: usize = 32;

/// One row of the Layers panel: a layer (`ocg` set) or a title (`ocg` none).
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub ocg: Option<ObjRef>,
    pub name: String,
    /// Nesting level in the /Order tree (0 = top).
    pub depth: usize,
    /// Shown when the document opens (/D BaseState, ON, OFF).
    pub on_by_default: bool,
    /// Listed in /D /Locked: Acrobat greys out its checkbox.
    pub locked: bool,
}

fn catalog_oc(doc: &Document) -> Option<&Dict> {
    let cat = doc.catalog().ok()?;
    doc.resolve_key(cat, b"OCProperties")?.as_dict()
}

fn default_config(doc: &Document) -> Option<&Dict> {
    doc.resolve_key(catalog_oc(doc)?, b"D")?.as_dict()
}

fn refs_in(doc: &Document, d: Option<&Dict>, key: &[u8]) -> HashSet<u32> {
    match d.and_then(|d| doc.resolve_key(d, key)) {
        Some(Object::Array(a)) => a
            .iter()
            .filter_map(Object::as_objref)
            .map(|r| r.num)
            .take(MAX_LAYERS)
            .collect(),
        _ => HashSet::new(),
    }
}

/// Every OCG of the document (from /OCProperties /OCGs).
pub fn all_groups(doc: &Document) -> Vec<ObjRef> {
    match catalog_oc(doc).and_then(|oc| doc.resolve_key(oc, b"OCGs")) {
        Some(Object::Array(a)) => a
            .iter()
            .filter_map(Object::as_objref)
            .take(MAX_LAYERS)
            .collect(),
        _ => Vec::new(),
    }
}

/// The groups hidden when the document opens.
pub fn default_hidden(doc: &Document) -> HashSet<u32> {
    let d = default_config(doc);
    let base_off = d.and_then(|d| d.get_name(b"BaseState")) == Some(b"OFF");
    let on = refs_in(doc, d, b"ON");
    let off = refs_in(doc, d, b"OFF");
    all_groups(doc)
        .into_iter()
        .map(|r| r.num)
        .filter(|n| {
            if base_off {
                !on.contains(n)
            } else {
                off.contains(n)
            }
        })
        .collect()
}

fn group_name(doc: &Document, r: ObjRef) -> String {
    doc.get(r)
        .and_then(Object::as_dict)
        .and_then(|d| doc.resolve_key(d, b"Name"))
        .and_then(Object::as_str)
        .map(decode_text)
        .unwrap_or_else(|| format!("Layer {}", r.num))
}

/// The Layers panel: rows in /Order order (titles and nesting kept); groups
/// the /Order tree leaves out are listed after it, as Acrobat does not show
/// them only when the configuration says so (/ListMode).
pub fn layers(doc: &Document) -> Vec<Layer> {
    let groups = all_groups(doc);
    if groups.is_empty() {
        return Vec::new();
    }
    let hidden = default_hidden(doc);
    let locked = refs_in(doc, default_config(doc), b"Locked");
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    if let Some(Object::Array(order)) =
        default_config(doc).and_then(|d| doc.resolve_key(d, b"Order"))
    {
        walk_order(doc, order, 0, &hidden, &locked, &mut seen, &mut out);
    }
    for g in groups {
        if out.len() >= MAX_LAYERS {
            break;
        }
        if seen.insert(g.num) {
            out.push(Layer {
                ocg: Some(g),
                name: group_name(doc, g),
                depth: 0,
                on_by_default: !hidden.contains(&g.num),
                locked: locked.contains(&g.num),
            });
        }
    }
    out
}

fn walk_order(
    doc: &Document,
    items: &[Object],
    depth: usize,
    hidden: &HashSet<u32>,
    locked: &HashSet<u32>,
    seen: &mut HashSet<u32>,
    out: &mut Vec<Layer>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    // An array whose first element is a string is a titled group of rows.
    let (title, rest) = match items.first().map(|o| doc.resolve(o)) {
        Some(Object::Str(s)) => (Some(decode_text(s)), items.get(1..).unwrap_or_default()),
        _ => (None, items),
    };
    let depth = match title {
        Some(t) => {
            out.push(Layer {
                ocg: None,
                name: t,
                depth,
                on_by_default: true,
                locked: false,
            });
            depth + 1
        }
        None => depth,
    };
    for it in rest {
        if out.len() >= MAX_LAYERS {
            return;
        }
        match it {
            Object::Ref(r) if matches!(doc.get(*r), Some(Object::Array(_))) => {
                if let Some(Object::Array(a)) = doc.get(*r) {
                    walk_order(doc, a, depth + 1, hidden, locked, seen, out);
                }
            }
            Object::Ref(r) => {
                if seen.insert(r.num) {
                    out.push(Layer {
                        ocg: Some(*r),
                        name: group_name(doc, *r),
                        depth,
                        on_by_default: !hidden.contains(&r.num),
                        locked: locked.contains(&r.num),
                    });
                }
            }
            Object::Array(a) => walk_order(doc, a, depth + 1, hidden, locked, seen, out),
            _ => {}
        }
    }
}

/// Decides what optional content is visible, with a cache per object.
pub struct Visibility<'a> {
    doc: &'a Document,
    hidden: HashSet<u32>,
    cache: HashMap<u32, bool>,
}

impl<'a> Visibility<'a> {
    /// `hidden`: the groups to hide; `None` uses the document's default.
    pub fn new(doc: &'a Document, hidden: Option<&[u32]>) -> Self {
        let hidden = match hidden {
            Some(h) => h.iter().copied().collect(),
            None => default_hidden(doc),
        };
        Visibility {
            doc,
            hidden,
            cache: HashMap::new(),
        }
    }

    /// Is content marked with this OCG or OCMD visible? Anything that is
    /// not a valid group or membership dictionary is visible (§8.11.2).
    pub fn visible(&mut self, o: &Object) -> bool {
        let key = o.as_objref().map(|r| r.num);
        if let Some(k) = key
            && let Some(&v) = self.cache.get(&k)
        {
            return v;
        }
        let v = self.eval(o, 0);
        if let Some(k) = key
            && self.cache.len() < 100_000
        {
            self.cache.insert(k, v);
        }
        v
    }

    fn group_on(&self, o: &Object) -> Option<bool> {
        let r = o.as_objref()?;
        let d = self.doc.get(r)?.as_dict()?;
        (d.get_name(b"Type") == Some(b"OCG")).then(|| !self.hidden.contains(&r.num))
    }

    fn eval(&self, o: &Object, depth: usize) -> bool {
        if depth > MAX_DEPTH {
            return true;
        }
        if let Some(on) = self.group_on(o) {
            return on;
        }
        let Some(d) = self.doc.resolve(o).as_dict() else {
            return true;
        };
        if d.get_name(b"Type") != Some(b"OCMD") {
            return true;
        }
        // A visibility expression takes precedence over /OCGs and /P.
        if let Some(ve) = self.doc.resolve_key(d, b"VE") {
            return self.expr(ve, depth + 1);
        }
        let states: Vec<bool> = match d.get(b"OCGs").map(|x| self.doc.resolve(x)) {
            Some(Object::Array(a)) => a
                .iter()
                .take(MAX_LAYERS)
                .filter_map(|g| self.group_on(g))
                .collect(),
            Some(_) => d
                .get(b"OCGs")
                .and_then(|g| self.group_on(g))
                .into_iter()
                .collect(),
            None => Vec::new(),
        };
        if states.is_empty() {
            return true;
        }
        match d.get_name(b"P") {
            Some(b"AllOn") => states.iter().all(|&s| s),
            Some(b"AnyOff") => states.iter().any(|&s| !s),
            Some(b"AllOff") => states.iter().all(|&s| !s),
            _ => states.iter().any(|&s| s), // AnyOn, the default
        }
    }

    fn expr(&self, o: &Object, depth: usize) -> bool {
        if depth > MAX_DEPTH {
            return true;
        }
        if let Some(on) = self.group_on(o) {
            return on;
        }
        let Some(a) = self.doc.resolve(o).as_array() else {
            return true;
        };
        let op = a.first().and_then(Object::as_name);
        let args = a.get(1..).unwrap_or_default().iter().take(MAX_LAYERS);
        match op {
            Some(b"And") => args.into_iter().all(|x| self.expr(x, depth + 1)),
            Some(b"Or") => args.into_iter().any(|x| self.expr(x, depth + 1)),
            Some(b"Not") => a.get(1).is_none_or(|x| !self.expr(x, depth + 1)),
            _ => true,
        }
    }
}
