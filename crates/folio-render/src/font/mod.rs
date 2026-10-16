//! PDF fonts (ISO 32000-2 §9.5–9.10): simple fonts (Type1, TrueType, Type3),
//! composite fonts (Type0 with CIDFontType0/2), embedded programs
//! (FontFile, FontFile2, FontFile3 Type1C/CIDFontType0C/OpenType), the
//! encoding → glyph selection rules, and widths. Glyph outlines come back
//! in "em" units: one text-space unit per unit of font size.

pub mod cmap;
pub mod system;
pub mod type1;

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use folio_core::filters::decode_stream;
use folio_core::glyphs::{BaseEncoding, name_char};
use folio_core::{Dict, Document, Object};
use tiny_skia::{Path, PathBuilder, Transform};

use cmap::CMap;
use type1::Type1;

enum Program {
    /// TrueType or OpenType (glyf or CFF inside an sfnt).
    Sfnt(Arc<Vec<u8>>),
    /// A bare CFF font (FontFile3 /Type1C or /CIDFontType0C).
    Cff(Arc<Vec<u8>>),
    Type1(Box<Type1>),
    None,
}

/// How a code reaches a glyph in a simple font.
#[derive(Clone, Debug)]
enum Lookup {
    Gid(u16),
    Name(String),
    Missing,
}

pub struct Type3 {
    pub matrix: [f64; 6],
    pub resources: Option<Dict>,
    procs: HashMap<u32, Vec<u8>>,
}

pub struct PdfFont {
    program: Program,
    /// Simple fonts: 256 lookups. Composite fonts: empty.
    simple: Vec<Lookup>,
    composite: Option<(CMap, CidToGid)>,
    /// Widths in em units (1/1000 of the PDF value, Type 3 via its matrix).
    widths: HashMap<u32, f64>,
    default_width: f64,
    /// Glyph units → em.
    matrix: Transform,
    /// Drawn with an installed font instead of the missing one.
    pub substituted: bool,
    pub type3: Option<Type3>,
    /// The font has nothing Folio can draw with.
    pub unavailable: bool,
    cache: RefCell<HashMap<u32, Option<Arc<Path>>>>,
}

enum CidToGid {
    Identity,
    Map(Vec<u16>),
    /// CID-keyed CFF: CID → GID through the charset.
    Charset(HashMap<u16, u16>),
}

/// Adapter from ttf-parser's outline callbacks to a tiny-skia path.
struct Outline(PathBuilder);

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

fn nums(doc: &Document, d: &Dict, key: &[u8]) -> Vec<f64> {
    match doc.resolve_key(d, key) {
        Some(Object::Array(a)) => a
            .iter()
            .filter_map(|x| doc.resolve(x).as_f64())
            .take(65_536)
            .collect(),
        _ => Vec::new(),
    }
}

fn matrix6(v: &[f64]) -> Option<[f64; 6]> {
    let m: [f64; 6] = v.get(..6)?.try_into().ok()?;
    (m.iter().all(|x| x.is_finite()) && m[0] * m[3] - m[1] * m[2] != 0.0).then_some(m)
}

fn to_transform(m: [f64; 6]) -> Transform {
    Transform::from_row(
        m[0] as f32,
        m[1] as f32,
        m[2] as f32,
        m[3] as f32,
        m[4] as f32,
        m[5] as f32,
    )
}

impl PdfFont {
    pub fn load(doc: &Document, fd: &Dict, system_fonts: bool) -> PdfFont {
        let subtype = fd.get_name(b"Subtype").unwrap_or(b"Type1").to_vec();
        match subtype.as_slice() {
            b"Type0" => Self::load_type0(doc, fd, system_fonts),
            b"Type3" => Self::load_type3(doc, fd),
            _ => Self::load_simple(doc, fd, system_fonts, &subtype),
        }
    }

    fn empty() -> PdfFont {
        PdfFont {
            program: Program::None,
            simple: vec![Lookup::Missing; 256],
            composite: None,
            widths: HashMap::new(),
            default_width: 0.5,
            matrix: Transform::from_scale(0.001, 0.001),
            substituted: false,
            type3: None,
            unavailable: true,
            cache: RefCell::new(HashMap::new()),
        }
    }

    /// The embedded font program named by a descriptor.
    fn program(doc: &Document, desc: Option<&Dict>) -> Program {
        let Some(desc) = desc else {
            return Program::None;
        };
        let stream = |k: &[u8]| doc.resolve_key(desc, k).and_then(Object::as_stream);
        if let Some(s) = stream(b"FontFile2")
            && let Ok(d) = decode_stream(s)
        {
            return Program::Sfnt(Arc::new(d));
        }
        if let Some(s) = stream(b"FontFile3")
            && let Ok(d) = decode_stream(s)
        {
            return match s.dict.get_name(b"Subtype") {
                Some(b"OpenType") => Program::Sfnt(Arc::new(d)),
                _ if d.starts_with(b"OTTO") || d.starts_with(&[0, 1, 0, 0]) => {
                    Program::Sfnt(Arc::new(d))
                }
                _ => Program::Cff(Arc::new(d)),
            };
        }
        if let Some(s) = stream(b"FontFile")
            && let Ok(d) = decode_stream(s)
            && let Some(t1) = Type1::parse(&d)
        {
            return Program::Type1(Box::new(t1));
        }
        Program::None
    }

    fn load_simple(doc: &Document, fd: &Dict, system_fonts: bool, subtype: &[u8]) -> PdfFont {
        let mut f = Self::empty();
        let desc = doc
            .resolve_key(fd, b"FontDescriptor")
            .and_then(Object::as_dict);
        let flags = desc.and_then(|d| d.get_int(b"Flags")).unwrap_or(0);
        let symbolic = flags & 4 != 0;
        let base = fd
            .get_name(b"BaseFont")
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_default();
        f.program = Self::program(doc, desc);
        if matches!(f.program, Program::None)
            && system_fonts
            && let Some((data, family)) = system::substitute(&base, flags)
        {
            f.program = Program::Sfnt(data);
            f.substituted = true;
            if family == system::Family::Symbol {
                f.default_width = 0.5;
            }
        }
        f.unavailable = matches!(f.program, Program::None);

        // Widths.
        let first = fd.get_int(b"FirstChar").unwrap_or(0);
        for (k, w) in nums(doc, fd, b"Widths").iter().enumerate().take(256) {
            if let Ok(code) = u32::try_from(first.saturating_add(k as i64)) {
                f.widths.insert(code, w / 1000.0);
            }
        }
        if let Some(mw) = desc
            .and_then(|d| doc.resolve_key(d, b"MissingWidth"))
            .and_then(Object::as_f64)
        {
            f.default_width = mw / 1000.0;
        }

        // Encoding: base names, then /Differences.
        let enc = doc.resolve_key(fd, b"Encoding");
        let (base_enc, diffs) = match enc {
            Some(Object::Name(n)) => (BaseEncoding::from_name(n), None),
            Some(Object::Dict(d)) => (
                d.get_name(b"BaseEncoding")
                    .and_then(BaseEncoding::from_name),
                doc.resolve_key(d, b"Differences")
                    .and_then(Object::as_array),
            ),
            _ => (None, None),
        };
        let is_tt = subtype == b"TrueType" || matches!(f.program, Program::Sfnt(_));
        let mut names: Vec<Option<String>> = vec![None; 256];
        let default_base = if base_enc.is_some() {
            base_enc
        } else if f.substituted {
            Some(BaseEncoding::WinAnsi)
        } else if (is_tt || enc.is_some()) && !symbolic {
            Some(BaseEncoding::Standard)
        } else {
            None
        };
        if let Some(b) = default_base {
            for (c, slot) in names.iter_mut().enumerate() {
                *slot = b.glyph_name(c as u8).map(String::from);
            }
        }
        let mut differed = vec![false; 256];
        if let Some(diffs) = diffs {
            let mut code: i64 = 0;
            for d in diffs.iter().take(4096) {
                match doc.resolve(d) {
                    Object::Int(n) => code = *n,
                    Object::Name(n) => {
                        if let Ok(c) = usize::try_from(code)
                            && let Some(slot) = names.get_mut(c)
                        {
                            *slot = Some(String::from_utf8_lossy(n).into_owned());
                            if let Some(dflag) = differed.get_mut(c) {
                                *dflag = true;
                            }
                        }
                        code += 1;
                    }
                    _ => {}
                }
            }
        }

        let cff_names = match &f.program {
            Program::Cff(data) => cff_name_map(data),
            _ => HashMap::new(),
        };
        // Code → glyph.
        let lookups: Vec<Lookup> = (0..256u32)
            .map(|code| {
                let name = names.get(code as usize).cloned().flatten();
                match &f.program {
                    Program::Sfnt(data) => {
                        let symbolic_font = symbolic && !f.substituted
                            || f.substituted
                                && system::classify(&base, flags).0 == system::Family::Symbol;
                        sfnt_lookup(
                            data,
                            code,
                            name.as_deref(),
                            symbolic_font,
                            *differed.get(code as usize).unwrap_or(&false),
                        )
                    }
                    Program::Cff(data) => cff_lookup(data, &cff_names, code, name.as_deref()),
                    Program::Type1(t1) => match name {
                        Some(n) if t1.has_glyph(&n) => Lookup::Name(n),
                        _ => t1
                            .builtin_name(code as u8)
                            .filter(|n| t1.has_glyph(n))
                            .map_or(Lookup::Missing, Lookup::Name),
                    },
                    Program::None => Lookup::Missing,
                }
            })
            .collect();
        f.simple = lookups;
        f.matrix = match &f.program {
            Program::Sfnt(data) => ttf_parser::Face::parse(data, 0)
                .map(|face| {
                    let s = 1.0 / f32::from(face.units_per_em().max(1));
                    Transform::from_scale(s, s)
                })
                .unwrap_or(Transform::from_scale(0.001, 0.001)),
            Program::Cff(data) => cff_matrix(data),
            Program::Type1(t1) => to_transform([
                t1.matrix[0],
                t1.matrix[1],
                t1.matrix[2],
                t1.matrix[3],
                t1.matrix[4],
                t1.matrix[5],
            ]),
            Program::None => Transform::from_scale(0.001, 0.001),
        };
        f
    }

    fn load_type3(doc: &Document, fd: &Dict) -> PdfFont {
        let mut f = Self::empty();
        let m =
            matrix6(&nums(doc, fd, b"FontMatrix")).unwrap_or([0.001, 0.0, 0.0, 0.001, 0.0, 0.0]);
        let procs_dict = doc.resolve_key(fd, b"CharProcs").and_then(Object::as_dict);
        let mut procs = HashMap::new();
        if let Some(Object::Dict(enc)) = doc.resolve_key(fd, b"Encoding")
            && let Some(Object::Array(diffs)) = doc.resolve_key(enc, b"Differences")
        {
            let mut code: i64 = 0;
            for d in diffs.iter().take(4096) {
                match doc.resolve(d) {
                    Object::Int(n) => code = *n,
                    Object::Name(n) => {
                        if let (Ok(c), Some(s)) = (
                            u32::try_from(code),
                            procs_dict
                                .and_then(|p| p.get(n))
                                .map(|o| doc.resolve(o))
                                .and_then(Object::as_stream),
                        ) && c < 256
                            && let Ok(data) = decode_stream(s)
                        {
                            procs.insert(c, data);
                        }
                        code += 1;
                    }
                    _ => {}
                }
            }
        }
        let first = fd.get_int(b"FirstChar").unwrap_or(0);
        for (k, w) in nums(doc, fd, b"Widths").iter().enumerate().take(256) {
            if let Ok(code) = u32::try_from(first.saturating_add(k as i64)) {
                // Type 3 widths are in glyph space.
                f.widths.insert(code, w * m[0]);
            }
        }
        f.default_width = 0.0;
        f.unavailable = procs.is_empty();
        f.type3 = Some(Type3 {
            matrix: m,
            resources: doc
                .resolve_key(fd, b"Resources")
                .and_then(Object::as_dict)
                .cloned(),
            procs,
        });
        f
    }

    fn load_type0(doc: &Document, fd: &Dict, system_fonts: bool) -> PdfFont {
        let mut f = Self::empty();
        let cmap = match doc.resolve_key(fd, b"Encoding") {
            Some(Object::Name(n)) => CMap::identity(n.ends_with(b"-V")),
            Some(Object::Stream(s)) => decode_stream(s)
                .map(|d| CMap::parse(&d))
                .unwrap_or_else(|_| CMap::identity(false)),
            _ => CMap::identity(false),
        };
        let Some(cid) = doc
            .resolve_key(fd, b"DescendantFonts")
            .and_then(Object::as_array)
            .and_then(|a| a.first())
            .map(|o| doc.resolve(o))
            .and_then(Object::as_dict)
        else {
            return f;
        };
        let desc = doc
            .resolve_key(cid, b"FontDescriptor")
            .and_then(Object::as_dict);
        f.program = Self::program(doc, desc);
        if matches!(f.program, Program::None) && system_fonts {
            let base = cid
                .get_name(b"BaseFont")
                .map(|n| String::from_utf8_lossy(n).into_owned())
                .unwrap_or_default();
            let flags = desc.and_then(|d| d.get_int(b"Flags")).unwrap_or(0);
            if let Some((data, _)) = system::substitute(&base, flags) {
                f.program = Program::Sfnt(data);
                f.substituted = true;
            }
        }
        f.unavailable = matches!(f.program, Program::None);
        let map = match doc.resolve_key(cid, b"CIDToGIDMap") {
            Some(Object::Stream(s)) => {
                let d = decode_stream(s).unwrap_or_default();
                CidToGid::Map(
                    d.chunks(2)
                        .take(65_536)
                        .map(|c| {
                            u16::from_be_bytes([
                                c.first().copied().unwrap_or(0),
                                c.get(1).copied().unwrap_or(0),
                            ])
                        })
                        .collect(),
                )
            }
            _ => match &f.program {
                Program::Cff(data) => {
                    cid_charset(data).map_or(CidToGid::Identity, CidToGid::Charset)
                }
                _ => CidToGid::Identity,
            },
        };
        f.default_width = doc
            .resolve_key(cid, b"DW")
            .and_then(Object::as_f64)
            .unwrap_or(1000.0)
            / 1000.0;
        if let Some(Object::Array(w)) = doc.resolve_key(cid, b"W") {
            cid_widths(doc, w, &mut f.widths);
        }
        f.matrix = match &f.program {
            Program::Sfnt(data) => ttf_parser::Face::parse(data, 0)
                .map(|face| {
                    let s = 1.0 / f32::from(face.units_per_em().max(1));
                    Transform::from_scale(s, s)
                })
                .unwrap_or(Transform::from_scale(0.001, 0.001)),
            Program::Cff(data) => cff_matrix(data),
            _ => Transform::from_scale(0.001, 0.001),
        };
        f.composite = Some((cmap, map));
        f
    }

    /// Split a shown string into character codes.
    pub fn codes(&self, bytes: &[u8]) -> Vec<(u32, usize)> {
        match &self.composite {
            Some((cmap, _)) => cmap.codes(bytes),
            None => bytes.iter().map(|&b| (u32::from(b), 1)).collect(),
        }
    }

    /// The advance of `code` in em units. Composite fonts are keyed by CID.
    pub fn width(&self, code: u32, len: usize) -> f64 {
        let key = match &self.composite {
            Some((cmap, _)) => cmap.cid(code, len),
            None => code,
        };
        self.widths.get(&key).copied().unwrap_or(self.default_width)
    }

    pub fn type3_proc(&self, code: u32) -> Option<&[u8]> {
        self.type3.as_ref()?.procs.get(&code).map(Vec::as_slice)
    }

    /// The glyph outline for `code`, in em units, cached.
    pub fn glyph(&self, code: u32, len: usize) -> Option<Arc<Path>> {
        if let Ok(c) = self.cache.try_borrow()
            && let Some(p) = c.get(&code)
        {
            return p.clone();
        }
        let p = self.build_glyph(code, len).map(Arc::new);
        if let Ok(mut c) = self.cache.try_borrow_mut()
            && c.len() < 65_536
        {
            c.insert(code, p.clone());
        }
        p
    }

    fn build_glyph(&self, code: u32, len: usize) -> Option<Path> {
        let mut out = Outline(PathBuilder::new());
        let mut advance: Option<f64> = None;
        match &self.composite {
            Some((cmap, map)) => {
                let cid = cmap.cid(code, len);
                let gid = match map {
                    CidToGid::Identity => u16::try_from(cid).ok()?,
                    CidToGid::Map(m) => *m.get(cid as usize)?,
                    CidToGid::Charset(c) => *c.get(&u16::try_from(cid).ok()?)?,
                };
                advance = self.draw_gid(gid, &mut out);
            }
            None => match self.simple.get(code as usize)? {
                Lookup::Gid(g) => advance = self.draw_gid(*g, &mut out),
                Lookup::Name(n) => match &self.program {
                    Program::Type1(t1) => advance = t1.outline(n, &mut out.0),
                    Program::Cff(data) => {
                        let t = ttf_parser::cff::Table::parse(data)?;
                        let g = t.glyph_index_by_name(n)?;
                        t.outline(g, &mut out).ok()?;
                    }
                    _ => return None,
                },
                Lookup::Missing => return None,
            },
        }
        let path = out.0.finish()?;
        let mut t = self.matrix;
        // An installed font stands in: stretch each glyph to the PDF's width.
        if self.substituted
            && let Some(adv) = advance
        {
            let want = self.width(code, len);
            let have = adv * f64::from(self.matrix.sx);
            if have > 0.0 && want > 0.0 {
                let k = (want / have).clamp(0.5, 1.5) as f32;
                t = t.post_concat(Transform::from_scale(k, 1.0));
            }
        }
        path.transform(t)
    }

    /// Draw glyph `gid` of an sfnt or CFF program; returns its advance in glyph units.
    fn draw_gid(&self, gid: u16, out: &mut Outline) -> Option<f64> {
        let g = ttf_parser::GlyphId(gid);
        match &self.program {
            Program::Sfnt(data) => {
                let face = ttf_parser::Face::parse(data, 0).ok()?;
                face.outline_glyph(g, out)?;
                face.glyph_hor_advance(g).map(f64::from)
            }
            Program::Cff(data) => {
                let t = ttf_parser::cff::Table::parse(data)?;
                t.outline(g, out).ok()?;
                t.glyph_width(g).map(f64::from)
            }
            _ => None,
        }
    }
}

fn cff_matrix(data: &[u8]) -> Transform {
    ttf_parser::cff::Table::parse(data)
        .map(|t| {
            let m = t.matrix();
            Transform::from_row(m.sx, m.ky, m.kx, m.sy, m.tx, m.ty)
        })
        .unwrap_or(Transform::from_scale(0.001, 0.001))
}

fn cid_charset(data: &[u8]) -> Option<HashMap<u16, u16>> {
    let t = ttf_parser::cff::Table::parse(data)?;
    let n = t.number_of_glyphs();
    let mut map = HashMap::new();
    for gid in 0..n {
        if let Some(cid) = t.glyph_cid(ttf_parser::GlyphId(gid)) {
            map.insert(cid, gid);
        }
    }
    (!map.is_empty()).then_some(map)
}

/// Glyph name → glyph id, read once from the CFF charset.
fn cff_name_map(data: &[u8]) -> HashMap<String, u16> {
    let mut map = HashMap::new();
    if let Some(t) = ttf_parser::cff::Table::parse(data) {
        for gid in 0..t.number_of_glyphs() {
            if let Some(n) = t.glyph_name(ttf_parser::GlyphId(gid)) {
                map.entry(n.to_string()).or_insert(gid);
            }
        }
    }
    map
}

fn cff_lookup(data: &[u8], names: &HashMap<String, u16>, code: u32, name: Option<&str>) -> Lookup {
    if let Some(g) = name.and_then(|n| names.get(n)) {
        return Lookup::Gid(*g);
    }
    let Some(t) = ttf_parser::cff::Table::parse(data) else {
        return Lookup::Missing;
    };
    u8::try_from(code)
        .ok()
        .and_then(|c| t.glyph_index(c))
        .map_or(Lookup::Missing, |g| Lookup::Gid(g.0))
}

/// TrueType glyph selection (§9.6.5.4).
fn sfnt_lookup(
    data: &[u8],
    code: u32,
    name: Option<&str>,
    symbolic: bool,
    differed: bool,
) -> Lookup {
    let Ok(face) = ttf_parser::Face::parse(data, 0) else {
        return Lookup::Missing;
    };
    let Some(cmap) = face.tables().cmap else {
        return name
            .and_then(|n| face.glyph_index_by_name(n))
            .map_or(Lookup::Missing, |g| Lookup::Gid(g.0));
    };
    let sub = |platform: ttf_parser::PlatformId, enc: u16| {
        cmap.subtables
            .into_iter()
            .find(|s| s.platform_id == platform && s.encoding_id == enc)
    };
    let unicode = sub(ttf_parser::PlatformId::Windows, 1)
        .or_else(|| sub(ttf_parser::PlatformId::Windows, 10))
        .or_else(|| sub(ttf_parser::PlatformId::Unicode, 3));
    let found = |s: Option<ttf_parser::cmap::Subtable<'_>>, cp: u32| {
        s.and_then(|s| s.glyph_index(cp)).filter(|g| g.0 != 0)
    };
    if (!symbolic || differed)
        && let Some(n) = name
        && let Some(c) = name_char(n)
        && let Some(g) = found(unicode, u32::from(c))
    {
        return Lookup::Gid(g.0);
    }
    let ms_symbol = sub(ttf_parser::PlatformId::Windows, 0);
    for cp in [0xF000 + code, code, 0xF100 + code, 0xF200 + code] {
        if let Some(g) = found(ms_symbol, cp) {
            return Lookup::Gid(g.0);
        }
    }
    if let Some(g) = found(sub(ttf_parser::PlatformId::Macintosh, 0), code) {
        return Lookup::Gid(g.0);
    }
    if let Some(n) = name {
        if let Some(g) = face.glyph_index_by_name(n) {
            return Lookup::Gid(g.0);
        }
        if let Some(c) = name_char(n)
            && let Some(g) = found(unicode, u32::from(c))
        {
            return Lookup::Gid(g.0);
        }
    }
    if let Some(g) = found(unicode, code) {
        return Lookup::Gid(g.0);
    }
    Lookup::Missing
}

/// `/W` arrays: `c [w1 w2 ...]` and `cfirst clast w` (§9.7.4.3).
fn cid_widths(doc: &Document, w: &[Object], out: &mut HashMap<u32, f64>) {
    let mut i = 0;
    while i < w.len() && out.len() < 200_000 {
        let Some(first) = w.get(i).and_then(|o| doc.resolve(o).as_int()) else {
            break;
        };
        match w.get(i + 1).map(|o| doc.resolve(o)) {
            Some(Object::Array(list)) => {
                for (k, v) in list.iter().enumerate().take(65_536) {
                    if let (Some(v), Ok(c)) = (
                        doc.resolve(v).as_f64(),
                        u32::try_from(first.saturating_add(k as i64)),
                    ) {
                        out.insert(c, v / 1000.0);
                    }
                }
                i += 2;
            }
            Some(o) => {
                let last = o.as_int().unwrap_or(first);
                let v = w
                    .get(i + 2)
                    .and_then(|o| doc.resolve(o).as_f64())
                    .unwrap_or(1000.0);
                let span = last.saturating_sub(first).clamp(0, 65_535);
                for c in first..=first.saturating_add(span) {
                    if let Ok(c) = u32::try_from(c) {
                        out.insert(c, v / 1000.0);
                    }
                }
                i += 3;
            }
            None => break,
        }
    }
}
