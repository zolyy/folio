//! A whole PDF held in memory: objects, trailer, page tree helpers.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use crate::error::{Error, Result};
use crate::filters::flate_encode;
use crate::object::{Dict, ObjRef, Object, Stream};
use crate::parser::find;

/// Largest file Folio will read into memory.
pub const MAX_FILE: u64 = 2 * 1024 * 1024 * 1024;
/// Maximum page-tree depth and length of reference chains.
const MAX_TREE_DEPTH: usize = 256;
/// Attributes a page inherits from its ancestors (§7.7.3.4).
const INHERITABLE: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

#[derive(Clone, Debug)]
pub struct Document {
    /// Header version, e.g. `"1.7"`.
    pub version: String,
    /// Every object by number. Generations are normalised to 0 on load.
    pub objects: BTreeMap<u32, Object>,
    pub trailer: Dict,
    /// What was wrong with the file and how it was repaired, in plain words.
    pub repairs: Vec<String>,
}

impl Document {
    /// An empty document with a catalog and no pages.
    pub fn new() -> Self {
        let mut doc = Document {
            version: "1.7".into(),
            objects: BTreeMap::new(),
            trailer: Dict::new(),
            repairs: Vec::new(),
        };
        let mut pages = Dict::new();
        pages.set("Type", Object::name("Pages"));
        pages.set("Kids", Object::Array(Vec::new()));
        pages.set("Count", Object::Int(0));
        let pages = doc.add(pages.into());
        let mut cat = Dict::new();
        cat.set("Type", Object::name("Catalog"));
        cat.set("Pages", pages.into());
        let cat = doc.add(cat.into());
        doc.trailer.set("Root", cat.into());
        doc
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let meta = std::fs::metadata(path)?;
        if meta.len() > MAX_FILE {
            return Err(Error::Limit(format!(
                "{} is larger than {} GB",
                path.display(),
                MAX_FILE >> 30
            )));
        }
        Self::from_bytes(&std::fs::read(path)?)
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let head = data.get(..data.len().min(1024)).unwrap_or_default();
        let version = match find(head, b"%PDF-", 0) {
            Some(p) => data
                .get(p + 5..)
                .unwrap_or_default()
                .iter()
                .take(8)
                .take_while(|b| b.is_ascii_digit() || **b == b'.')
                .map(|&b| char::from(b))
                .collect::<String>(),
            None => String::new(),
        };
        let loaded = crate::xref::load(data)?;
        let mut doc = Document {
            version: if version.is_empty() {
                "1.7".into()
            } else {
                version
            },
            objects: loaded.objects,
            trailer: loaded.trailer,
            repairs: loaded.repairs,
        };
        if find(head, b"%PDF-", 0).is_none() {
            doc.repairs.push("missing %PDF- header".into());
        }
        if doc.trailer.contains(b"Encrypt") {
            return Err(Error::Unsupported(
                "encrypted PDFs (decryption arrives in stage 2 of PLAN.md)".into(),
            ));
        }
        // Normalise generations so references match object numbers alone.
        for o in doc.objects.values_mut() {
            o.for_each_ref_mut(&mut |r| {
                if let Object::Ref(r) = r {
                    r.generation = 0;
                }
            });
        }
        doc.trailer.values_mut().for_each(|o| {
            o.for_each_ref_mut(&mut |r| {
                if let Object::Ref(r) = r {
                    r.generation = 0;
                }
            })
        });
        Ok(doc)
    }

    pub fn get(&self, r: ObjRef) -> Option<&Object> {
        self.objects.get(&r.num)
    }

    pub fn get_mut(&mut self, r: ObjRef) -> Option<&mut Object> {
        self.objects.get_mut(&r.num)
    }

    /// Follow references until a direct object. Missing objects are null.
    pub fn resolve<'a>(&'a self, mut o: &'a Object) -> &'a Object {
        for _ in 0..MAX_TREE_DEPTH {
            match o {
                Object::Ref(r) => match self.get(*r) {
                    Some(next) => o = next,
                    None => return &Object::Null,
                },
                _ => return o,
            }
        }
        &Object::Null
    }

    /// Look up `key` in `dict` and resolve it.
    pub fn resolve_key<'a>(&'a self, dict: &'a Dict, key: &[u8]) -> Option<&'a Object> {
        match self.resolve(dict.get(key)?) {
            Object::Null => None,
            o => Some(o),
        }
    }

    pub fn max_num(&self) -> u32 {
        self.objects.keys().next_back().copied().unwrap_or(0)
    }

    /// Add an object and return its reference.
    pub fn add(&mut self, o: Object) -> ObjRef {
        let num = self.max_num().saturating_add(1);
        self.objects.insert(num, o);
        ObjRef::new(num, 0)
    }

    pub fn catalog_ref(&self) -> Result<ObjRef> {
        self.trailer
            .get(b"Root")
            .and_then(Object::as_objref)
            .ok_or_else(|| Error::Invalid("the document has no catalog".into()))
    }

    pub fn catalog(&self) -> Result<&Dict> {
        let r = self.catalog_ref()?;
        self.get(r)
            .and_then(Object::as_dict)
            .ok_or_else(|| Error::Invalid("the document catalog is not a dictionary".into()))
    }

    pub fn catalog_mut(&mut self) -> Result<&mut Dict> {
        let r = self.catalog_ref()?;
        self.get_mut(r)
            .and_then(Object::as_dict_mut)
            .ok_or_else(|| Error::Invalid("the document catalog is not a dictionary".into()))
    }

    /// The pages in order. Cycles and absurd depths in the tree are cut.
    pub fn pages(&self) -> Result<Vec<ObjRef>> {
        let root = self
            .catalog()?
            .get(b"Pages")
            .and_then(Object::as_objref)
            .ok_or_else(|| Error::Invalid("the catalog has no page tree".into()))?;
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut stack = vec![(root, 0usize)];
        while let Some((r, depth)) = stack.pop() {
            if depth > MAX_TREE_DEPTH || !seen.insert(r.num) {
                continue;
            }
            let Some(d) = self.get(r).and_then(Object::as_dict) else {
                continue;
            };
            let kids = match self.resolve_key(d, b"Kids") {
                Some(Object::Array(k)) => Some(k),
                _ => None,
            };
            let is_page = d.get_name(b"Type") == Some(b"Page") || kids.is_none();
            if is_page {
                out.push(r);
            } else if let Some(kids) = kids {
                for k in kids.iter().rev() {
                    if let Some(kr) = k.as_objref() {
                        stack.push((kr, depth + 1));
                    }
                }
            }
        }
        Ok(out)
    }

    /// A page attribute, looking up the tree for inheritable ones.
    pub fn page_attr(&self, page: ObjRef, key: &[u8]) -> Option<Object> {
        let mut cur = page;
        for _ in 0..MAX_TREE_DEPTH {
            let d = self.get(cur)?.as_dict()?;
            if let Some(v) = d.get(key) {
                return Some(self.resolve(v).clone());
            }
            if !INHERITABLE.contains(&key) {
                return None;
            }
            cur = d.get(b"Parent")?.as_objref()?;
        }
        None
    }

    /// The page's MediaBox as `[x0, y0, x1, y1]`, US Letter if absent or broken.
    pub fn media_box(&self, page: ObjRef) -> [f64; 4] {
        let def = [0.0, 0.0, 612.0, 792.0];
        let Some(Object::Array(a)) = self.page_attr(page, b"MediaBox") else {
            return def;
        };
        let v: Vec<f64> = a.iter().filter_map(|x| self.resolve(x).as_f64()).collect();
        match v.as_slice() {
            [a, b, c, d] => [*a, *b, *c, *d],
            _ => def,
        }
    }

    /// Copy inherited attributes onto every page, so pages can be moved
    /// between trees (or documents) without changing how they look.
    pub fn materialize_pages(&mut self) -> Result<Vec<ObjRef>> {
        let pages = self.pages()?;
        for &p in &pages {
            let mut add = Vec::new();
            for key in INHERITABLE {
                let has = self
                    .get(p)
                    .and_then(Object::as_dict)
                    .is_some_and(|d| d.contains(key));
                if !has && let Some(v) = self.page_attr(p, key) {
                    add.push((key, v));
                }
            }
            if let Some(d) = self.get_mut(p).and_then(Object::as_dict_mut) {
                for (k, v) in add {
                    d.set(k, v);
                }
                if !d.contains(b"MediaBox") {
                    d.set("MediaBox", rect([0.0, 0.0, 612.0, 792.0]));
                }
            }
        }
        Ok(pages)
    }

    /// Replace the page tree with a single flat node holding `pages` in order.
    /// Call [`Document::materialize_pages`] first. A page listed twice is copied.
    pub fn set_pages(&mut self, pages: &[ObjRef]) -> Result<()> {
        let node = self.add(Object::Null);
        let mut kids = Vec::with_capacity(pages.len());
        let mut used = HashSet::new();
        for &p in pages {
            let Some(obj) = self.get(p).cloned() else {
                return Err(Error::Invalid(format!("page object {} is missing", p.num)));
            };
            let r = if used.insert(p.num) { p } else { self.add(obj) };
            if let Some(d) = self.get_mut(r).and_then(Object::as_dict_mut) {
                d.set("Parent", node.into());
                d.set("Type", Object::name("Page"));
            }
            kids.push(Object::Ref(r));
        }
        let mut d = Dict::new();
        d.set("Type", Object::name("Pages"));
        d.set("Count", Object::Int(kids.len() as i64));
        d.set("Kids", Object::Array(kids));
        self.objects.insert(node.num, d.into());
        self.catalog_mut()?.set("Pages", node.into());
        Ok(())
    }

    /// Add a page with a content stream (compressed) and optional resources.
    pub fn add_page(&mut self, size: [f64; 2], content: &[u8], resources: Dict) -> Result<ObjRef> {
        let mut sd = Dict::new();
        sd.set("Filter", Object::name("FlateDecode"));
        let contents = self.add(Object::Stream(Stream {
            dict: sd,
            data: flate_encode(content),
        }));
        let mut page = Dict::new();
        page.set("Type", Object::name("Page"));
        page.set("MediaBox", rect([0.0, 0.0, size[0], size[1]]));
        page.set("Resources", resources.into());
        page.set("Contents", contents.into());
        let page = self.add(page.into());
        let mut pages = self.materialize_pages()?;
        pages.push(page);
        self.set_pages(&pages)?;
        Ok(page)
    }

    /// Add a page showing `text` in Helvetica, one line per `\n`.
    pub fn add_text_page(&mut self, text: &str) -> Result<ObjRef> {
        let mut font = Dict::new();
        font.set("Type", Object::name("Font"));
        font.set("Subtype", Object::name("Type1"));
        font.set("BaseFont", Object::name("Helvetica"));
        font.set("Encoding", Object::name("WinAnsiEncoding"));
        let font = self.add(font.into());
        let mut fonts = Dict::new();
        fonts.set("F1", font.into());
        let mut res = Dict::new();
        res.set("Font", fonts.into());
        let mut content = b"BT /F1 12 Tf 14 TL 72 720 Td\n".to_vec();
        for line in text.lines() {
            content.extend_from_slice(&crate::writer::serialize(&Object::Str(
                line.chars()
                    .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
                    .collect(),
            )));
            content.extend_from_slice(b" Tj T*\n");
        }
        content.extend_from_slice(b"ET\n");
        self.add_page([612.0, 792.0], &content, res)
    }

    /// Drop every object not reachable from the trailer. Returns how many went.
    pub fn prune(&mut self) -> usize {
        let mut reach = HashSet::new();
        let mut stack = Vec::new();
        for (_, v) in self.trailer.iter() {
            v.collect_refs(&mut stack);
        }
        while let Some(r) = stack.pop() {
            if !reach.insert(r.num) {
                continue;
            }
            if let Some(o) = self.objects.get(&r.num) {
                o.collect_refs(&mut stack);
            }
        }
        let before = self.objects.len();
        self.objects.retain(|n, _| reach.contains(n));
        before - self.objects.len()
    }

    /// Bring in every object of `other`, renumbered after ours. Returns the
    /// imported pages (with inherited attributes materialised) in order.
    pub fn import(&mut self, mut other: Document) -> Result<Vec<ObjRef>> {
        let pages = other.materialize_pages()?;
        let offset = self.max_num();
        let shift = |n: u32| {
            n.checked_add(offset)
                .ok_or_else(|| Error::Limit("too many objects to combine".into()))
        };
        for (num, mut obj) in std::mem::take(&mut other.objects) {
            obj.for_each_ref_mut(&mut |o| {
                if let Object::Ref(r) = o {
                    r.num = r.num.saturating_add(offset);
                }
            });
            self.objects.insert(shift(num)?, obj);
        }
        pages
            .into_iter()
            .map(|p| Ok(ObjRef::new(shift(p.num)?, 0)))
            .collect()
    }

    /// The document information dictionary, if any.
    pub fn info(&self) -> Option<&Dict> {
        self.trailer
            .get(b"Info")
            .map(|o| self.resolve(o))
            .and_then(Object::as_dict)
    }

    /// The information dictionary, created if missing.
    pub fn info_mut(&mut self) -> Result<&mut Dict> {
        let r = match self.trailer.get(b"Info") {
            Some(Object::Ref(r)) if self.get(*r).and_then(Object::as_dict).is_some() => *r,
            other => {
                let d = match other {
                    Some(Object::Dict(d)) => d.clone(),
                    _ => Dict::new(),
                };
                let r = self.add(d.into());
                self.trailer.set("Info", r.into());
                r
            }
        };
        self.get_mut(r)
            .and_then(Object::as_dict_mut)
            .ok_or_else(|| Error::Invalid("the information dictionary is damaged".into()))
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        crate::writer::write(self)
    }

    /// Write atomically: a temporary file next to the target, then a rename,
    /// so a crash or full disk never leaves a half-written PDF behind.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let bytes = self.to_bytes()?;
        let dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "out.pdf".into());
        let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
        let res = (|| -> std::io::Result<()> {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&tmp, path)
        })();
        if res.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        Ok(res?)
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

pub fn rect(r: [f64; 4]) -> Object {
    Object::Array(r.iter().map(|&v| num(v)).collect())
}

/// An integer object when the value is whole, otherwise a real.
pub fn num(v: f64) -> Object {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        Object::Int(v as i64)
    } else {
        Object::Real(v)
    }
}
