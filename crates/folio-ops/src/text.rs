//! Text extraction for Save As Text (Plain). Characters come from ToUnicode
//! CMaps, standard encodings with /Differences, or Type0 fonts; spaces and
//! line breaks come from where the text is placed on the page.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use folio_core::filters::decode_stream;
use folio_core::lexer::Token;
use folio_core::parser::Parser;
use folio_core::{Dict, Document, ObjRef, Object, Result};

/// Operators per page before giving up (hostile content streams).
const MAX_OPS: usize = 5_000_000;
/// Form XObjects nested deeper than this are skipped.
const MAX_FORM_DEPTH: usize = 12;

/// The text of one page.
pub fn page_text(doc: &Document, page: ObjRef) -> Result<String> {
    let mut ex = Extractor {
        doc,
        out: String::new(),
        last: None,
        ops: 0,
        fonts: HashMap::new(),
        forms_seen: HashSet::new(),
    };
    let content = page_content(doc, page);
    let res = doc
        .page_attr(page, b"Resources")
        .and_then(|o| o.as_dict().cloned())
        .unwrap_or_default();
    ex.run(&content, &res, IDENTITY, 0);
    Ok(ex.out.trim_end().to_string())
}

/// The decoded content of a page, all of its content streams joined.
pub fn page_content(doc: &Document, page: ObjRef) -> Vec<u8> {
    let Some(d) = doc.get(page).and_then(Object::as_dict) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let parts: Vec<&Object> = match d.get(b"Contents").map(|c| doc.resolve(c)) {
        Some(Object::Array(a)) => a.iter().collect(),
        Some(o @ Object::Stream(_)) => vec![o],
        _ => Vec::new(),
    };
    for p in parts {
        if let Object::Stream(s) = doc.resolve(p)
            && let Ok(data) = decode_stream(s)
        {
            out.extend_from_slice(&data);
            out.push(b'\n');
        }
    }
    out
}

type Matrix = [f64; 6];
const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

fn mul(a: &Matrix, b: &Matrix) -> Matrix {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

struct Font {
    two_byte: bool,
    to_unicode: HashMap<u32, String>,
    encoding: [Option<char>; 256],
    widths: HashMap<u32, f64>,
    default_width: f64,
}

impl Font {
    fn decode(&self, bytes: &[u8]) -> Vec<(u32, String)> {
        let mut out = Vec::new();
        if self.two_byte {
            for c in bytes.chunks(2) {
                let code = c.iter().fold(0u32, |a, &b| (a << 8) | u32::from(b));
                let s = self.to_unicode.get(&code).cloned().unwrap_or_default();
                out.push((code, s));
            }
        } else {
            for &b in bytes {
                let code = u32::from(b);
                let s = match self.to_unicode.get(&code) {
                    Some(s) => s.clone(),
                    None => self
                        .encoding
                        .get(usize::from(b))
                        .copied()
                        .flatten()
                        .map(String::from)
                        .unwrap_or_default(),
                };
                out.push((code, s));
            }
        }
        out
    }

    fn width(&self, code: u32) -> f64 {
        self.widths
            .get(&code)
            .copied()
            .unwrap_or(self.default_width)
    }
}

#[derive(Clone)]
struct GState {
    ctm: Matrix,
    font: Option<Rc<Font>>,
    size: f64,
    char_space: f64,
    word_space: f64,
    scale: f64,
    leading: f64,
    rise: f64,
}

struct Extractor<'a> {
    doc: &'a Document,
    out: String,
    /// End of the last text run: x, y, font size (device space).
    last: Option<(f64, f64, f64)>,
    ops: usize,
    fonts: HashMap<u32, Rc<Font>>,
    forms_seen: HashSet<u32>,
}

impl Extractor<'_> {
    fn run(&mut self, content: &[u8], res: &Dict, ctm: Matrix, depth: usize) {
        let mut gs = GState {
            ctm,
            font: None,
            size: 12.0,
            char_space: 0.0,
            word_space: 0.0,
            scale: 1.0,
            leading: 0.0,
            rise: 0.0,
        };
        let mut stack: Vec<GState> = Vec::new();
        let mut tm = IDENTITY;
        let mut tlm = IDENTITY;
        let mut operands: Vec<Object> = Vec::new();
        let mut p = Parser::content(content);
        loop {
            let at = p.lex.pos;
            let tok = match p.lex.token() {
                Ok(Some(t)) => t,
                _ => break,
            };
            let op = match tok {
                Token::Keyword(k) => k,
                t => {
                    match p.from_token(t, 0, at) {
                        Ok(o) if operands.len() < 64 => operands.push(o),
                        Ok(_) => {}
                        Err(_) => break,
                    }
                    continue;
                }
            };
            self.ops += 1;
            if self.ops > MAX_OPS {
                break;
            }
            let f = |i: usize| operands.get(i).and_then(Object::as_f64).unwrap_or(0.0);
            match op.as_slice() {
                b"q" => {
                    if stack.len() < 256 {
                        stack.push(gs.clone());
                    }
                }
                b"Q" => {
                    if let Some(s) = stack.pop() {
                        gs = s;
                    }
                }
                b"cm" => {
                    let m = [f(0), f(1), f(2), f(3), f(4), f(5)];
                    gs.ctm = mul(&m, &gs.ctm);
                }
                b"BT" => {
                    tm = IDENTITY;
                    tlm = IDENTITY;
                }
                b"Tf" => {
                    gs.size = f(1);
                    gs.font = operands
                        .first()
                        .and_then(Object::as_name)
                        .and_then(|n| self.font(res, n));
                }
                b"Tc" => gs.char_space = f(0),
                b"Tw" => gs.word_space = f(0),
                b"Tz" => gs.scale = f(0) / 100.0,
                b"TL" => gs.leading = f(0),
                b"Ts" => gs.rise = f(0),
                b"Td" => {
                    tlm = mul(&[1.0, 0.0, 0.0, 1.0, f(0), f(1)], &tlm);
                    tm = tlm;
                }
                b"TD" => {
                    gs.leading = -f(1);
                    tlm = mul(&[1.0, 0.0, 0.0, 1.0, f(0), f(1)], &tlm);
                    tm = tlm;
                }
                b"Tm" => {
                    tlm = [f(0), f(1), f(2), f(3), f(4), f(5)];
                    tm = tlm;
                }
                b"T*" => {
                    tlm = mul(&[1.0, 0.0, 0.0, 1.0, 0.0, -gs.leading], &tlm);
                    tm = tlm;
                }
                b"Tj" | b"'" | b"\"" => {
                    if op.as_slice() != b"Tj" {
                        if op.as_slice() == b"\"" {
                            gs.word_space = f(0);
                            gs.char_space = f(1);
                        }
                        tlm = mul(&[1.0, 0.0, 0.0, 1.0, 0.0, -gs.leading], &tlm);
                        tm = tlm;
                    }
                    if let Some(s) = operands.last().and_then(Object::as_str) {
                        let s = s.to_vec();
                        self.show(&s, &gs, &mut tm);
                    }
                }
                b"TJ" => {
                    if let Some(Object::Array(items)) = operands.first() {
                        let items = items.clone();
                        for it in &items {
                            match it {
                                Object::Str(s) => self.show(s, &gs, &mut tm),
                                o => {
                                    let n = o.as_f64().unwrap_or(0.0);
                                    let tx = -n / 1000.0 * gs.size * gs.scale;
                                    tm = mul(&[1.0, 0.0, 0.0, 1.0, tx, 0.0], &tm);
                                }
                            }
                        }
                    }
                }
                b"Do" => {
                    if let Some(name) = operands.first().and_then(Object::as_name) {
                        let name = name.to_vec();
                        self.form(res, &name, &gs.ctm, depth);
                    }
                }
                b"BI" => {
                    // Inline image: skip its header and data.
                    loop {
                        match p.lex.token() {
                            Ok(Some(Token::Keyword(k))) if k == b"ID" => break,
                            Ok(Some(_)) => continue,
                            _ => break,
                        }
                    }
                    p.lex.skip_inline_image_data();
                }
                _ => {}
            }
            operands.clear();
        }
    }

    fn show(&mut self, bytes: &[u8], gs: &GState, tm: &mut Matrix) {
        let Some(font) = gs.font.clone() else {
            return;
        };
        let start = mul(&mul(&[1.0, 0.0, 0.0, 1.0, 0.0, gs.rise], tm), &gs.ctm);
        let mut text = String::new();
        for (code, s) in font.decode(bytes) {
            text.push_str(&s);
            let w = font.width(code) / 1000.0;
            let mut tx = w * gs.size + gs.char_space;
            if !font.two_byte && code == 32 {
                tx += gs.word_space;
            }
            *tm = mul(&[1.0, 0.0, 0.0, 1.0, tx * gs.scale, 0.0], tm);
        }
        let end = mul(&mul(&[1.0, 0.0, 0.0, 1.0, 0.0, gs.rise], tm), &gs.ctm);
        let size = (gs.size * (start[2].hypot(start[3]))).abs().max(1.0);
        self.emit(&text, (start[4], start[5]), (end[4], end[5]), size);
    }

    fn emit(&mut self, text: &str, start: (f64, f64), end: (f64, f64), size: f64) {
        if text.is_empty() {
            return;
        }
        if let Some((lx, ly, lsize)) = self.last {
            let line = size.max(lsize);
            if (start.1 - ly).abs() > line * 0.5 {
                if !self.out.ends_with('\n') {
                    self.out.push('\n');
                }
            } else if start.0 - lx > line * 0.15
                && !self.out.ends_with(' ')
                && !self.out.ends_with('\n')
                && !text.starts_with(' ')
            {
                self.out.push(' ');
            }
        }
        if self.out.len() < 64 * 1024 * 1024 {
            self.out.push_str(text);
        }
        self.last = Some((end.0, end.1, size));
    }

    fn form(&mut self, res: &Dict, name: &[u8], ctm: &Matrix, depth: usize) {
        if depth >= MAX_FORM_DEPTH {
            return;
        }
        let doc = self.doc;
        let Some(xobjs) = doc.resolve_key(res, b"XObject").and_then(Object::as_dict) else {
            return;
        };
        let Some(r) = xobjs.get(name).and_then(Object::as_objref) else {
            return;
        };
        if !self.forms_seen.insert(r.num) {
            return;
        }
        let Some(Object::Stream(s)) = doc.get(r) else {
            return;
        };
        if s.dict.get_name(b"Subtype") != Some(b"Form") {
            return;
        }
        let Ok(data) = decode_stream(s) else { return };
        let m = match doc.resolve_key(&s.dict, b"Matrix") {
            Some(Object::Array(a)) => {
                let v: Vec<f64> = a.iter().filter_map(|x| doc.resolve(x).as_f64()).collect();
                match v.as_slice() {
                    [a, b, c, d, e, f] => [*a, *b, *c, *d, *e, *f],
                    _ => IDENTITY,
                }
            }
            _ => IDENTITY,
        };
        let inner = doc
            .resolve_key(&s.dict, b"Resources")
            .and_then(Object::as_dict)
            .cloned()
            .unwrap_or_else(|| res.clone());
        self.run(&data, &inner, mul(&m, ctm), depth + 1);
        self.forms_seen.remove(&r.num);
    }

    fn font(&mut self, res: &Dict, name: &[u8]) -> Option<Rc<Font>> {
        let doc = self.doc;
        let fonts = doc.resolve_key(res, b"Font")?.as_dict()?;
        let fref = fonts.get(name)?;
        let key = fref.as_objref().map(|r| r.num);
        if let Some(k) = key
            && let Some(f) = self.fonts.get(&k)
        {
            return Some(f.clone());
        }
        let fd = doc.resolve(fref).as_dict()?;
        let f = Rc::new(load_font(doc, fd));
        if let Some(k) = key {
            self.fonts.insert(k, f.clone());
        }
        Some(f)
    }
}

fn load_font(doc: &Document, fd: &Dict) -> Font {
    let two_byte = fd.get_name(b"Subtype") == Some(b"Type0");
    let to_unicode = match doc.resolve_key(fd, b"ToUnicode") {
        Some(Object::Stream(s)) => decode_stream(s).map(|d| parse_cmap(&d)).unwrap_or_default(),
        _ => HashMap::new(),
    };
    let mut encoding = win_ansi();
    let mut widths = HashMap::new();
    let mut default_width = 500.0;
    if two_byte {
        default_width = 1000.0;
        if let Some(Object::Array(desc)) = doc.resolve_key(fd, b"DescendantFonts")
            && let Some(cid) = desc
                .first()
                .map(|d| doc.resolve(d))
                .and_then(Object::as_dict)
        {
            if let Some(dw) = doc.resolve_key(cid, b"DW").and_then(Object::as_f64) {
                default_width = dw;
            }
            if let Some(Object::Array(w)) = doc.resolve_key(cid, b"W") {
                parse_cid_widths(doc, w, &mut widths);
            }
        }
    } else {
        match doc.resolve_key(fd, b"Encoding") {
            Some(Object::Dict(e)) => {
                if let Some(Object::Array(diffs)) = doc.resolve_key(e, b"Differences") {
                    let mut code: i64 = 0;
                    for d in diffs {
                        match doc.resolve(d) {
                            Object::Int(n) => code = *n,
                            Object::Name(g) => {
                                if let (Ok(c), Some(ch)) = (u8::try_from(code), glyph_char(g))
                                    && let Some(slot) = encoding.get_mut(usize::from(c))
                                {
                                    *slot = Some(ch);
                                }
                                code += 1;
                            }
                            _ => {}
                        }
                    }
                }
            }
            Some(Object::Name(n)) if n == b"MacRomanEncoding" => {}
            _ => {}
        }
        let first = fd.get_int(b"FirstChar").unwrap_or(0);
        if let Some(Object::Array(w)) = doc.resolve_key(fd, b"Widths") {
            for (k, v) in w.iter().enumerate().take(256) {
                if let (Some(v), Ok(code)) = (
                    doc.resolve(v).as_f64(),
                    u32::try_from(first.saturating_add(k as i64)),
                ) {
                    widths.insert(code, v);
                }
            }
        } else if let Some(base) = fd.get_name(b"BaseFont") {
            default_width = if base.starts_with(b"Courier") {
                600.0
            } else {
                500.0
            };
        }
    }
    Font {
        two_byte,
        to_unicode,
        encoding,
        widths,
        default_width,
    }
}

/// `/W` arrays: `c [w1 w2 ...]` and `cfirst clast w`.
fn parse_cid_widths(doc: &Document, w: &[Object], out: &mut HashMap<u32, f64>) {
    let mut i = 0;
    while i < w.len() && out.len() < 200_000 {
        let Some(first) = w.get(i).and_then(|o| doc.resolve(o).as_int()) else {
            break;
        };
        match w.get(i + 1).map(|o| doc.resolve(o)) {
            Some(Object::Array(list)) => {
                for (k, v) in list.iter().enumerate() {
                    if let (Some(v), Ok(c)) = (
                        doc.resolve(v).as_f64(),
                        u32::try_from(first.saturating_add(k as i64)),
                    ) {
                        out.insert(c, v);
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
                        out.insert(c, v);
                    }
                }
                i += 3;
            }
            None => break,
        }
    }
}

/// ToUnicode CMap: `bfchar` and `bfrange` sections (§9.10.3).
pub fn parse_cmap(data: &[u8]) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    let mut p = Parser::content(data);
    let mut mode = 0u8; // 1 = bfchar, 2 = bfrange
    let mut items: Vec<Object> = Vec::new();
    let code_of = |b: &[u8]| b.iter().take(4).fold(0u32, |a, &x| (a << 8) | u32::from(x));
    loop {
        let at = p.lex.pos;
        let tok = match p.lex.token() {
            Ok(Some(t)) => t,
            _ => break,
        };
        match tok {
            Token::Keyword(k) => match k.as_slice() {
                b"beginbfchar" => {
                    mode = 1;
                    items.clear();
                }
                b"beginbfrange" => {
                    mode = 2;
                    items.clear();
                }
                b"endbfchar" => {
                    for pair in items.chunks(2) {
                        if let (Some(Object::Str(src)), Some(Object::Str(dst))) =
                            (pair.first(), pair.get(1))
                        {
                            map.insert(code_of(src), utf16be(dst));
                        }
                    }
                    mode = 0;
                }
                b"endbfrange" => {
                    for tri in items.chunks(3) {
                        let (Some(Object::Str(lo)), Some(Object::Str(hi)), Some(dst)) =
                            (tri.first(), tri.get(1), tri.get(2))
                        else {
                            continue;
                        };
                        let (lo, hi) = (code_of(lo), code_of(hi));
                        if hi < lo || hi - lo > 65_535 {
                            continue;
                        }
                        match dst {
                            Object::Str(d) => {
                                let mut units: Vec<u16> = d
                                    .chunks(2)
                                    .map(|c| {
                                        u16::from_be_bytes([
                                            c.first().copied().unwrap_or(0),
                                            c.get(1).copied().unwrap_or(0),
                                        ])
                                    })
                                    .collect();
                                for code in lo..=hi {
                                    map.insert(code, String::from_utf16_lossy(&units));
                                    if let Some(last) = units.last_mut() {
                                        *last = last.wrapping_add(1);
                                    }
                                }
                            }
                            Object::Array(list) => {
                                for (k, d) in list.iter().enumerate() {
                                    if let Some(d) = d.as_str() {
                                        map.insert(lo.saturating_add(k as u32), utf16be(d));
                                    }
                                }
                            }
                            _ => {}
                        }
                        if map.len() > 1_000_000 {
                            return map;
                        }
                    }
                    mode = 0;
                }
                _ => {}
            },
            t if mode != 0 => {
                if let Ok(o) = p.from_token(t, 0, at)
                    && items.len() < 300_000
                {
                    items.push(o);
                }
            }
            _ => {}
        }
    }
    map
}

fn utf16be(b: &[u8]) -> String {
    let units: Vec<u16> = b
        .chunks(2)
        .map(|c| {
            u16::from_be_bytes([
                c.first().copied().unwrap_or(0),
                c.get(1).copied().unwrap_or(0),
            ])
        })
        .collect();
    String::from_utf16_lossy(&units)
}

/// WinAnsiEncoding (Annex D); also used for StandardEncoding's ASCII range.
fn win_ansi() -> [Option<char>; 256] {
    const HIGH: [char; 32] = [
        '€', '\0', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\0', 'Ž', '\0', '\0',
        '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\0', 'ž', 'Ÿ',
    ];
    let mut t = [None; 256];
    for (i, slot) in t.iter_mut().enumerate() {
        let b = i as u8;
        *slot = match b {
            0x20..=0x7E => Some(char::from(b)),
            0x80..=0x9F => HIGH
                .get(usize::from(b - 0x80))
                .copied()
                .filter(|c| *c != '\0'),
            0xA0..=0xFF => Some(char::from(b)),
            _ => None,
        };
    }
    t
}

/// Unicode for a glyph name: `uniXXXX`, `uXXXX`, single letters, and the
/// common names (including those needed for Romanian and Hungarian).
fn glyph_char(name: &[u8]) -> Option<char> {
    let s = std::str::from_utf8(name).ok()?;
    if let Some(hex) = s.strip_prefix("uni").filter(|h| h.len() >= 4) {
        return u32::from_str_radix(hex.get(..4)?, 16)
            .ok()
            .and_then(char::from_u32);
    }
    if let Some(hex) = s.strip_prefix('u').filter(|h| (4..=6).contains(&h.len()))
        && let Some(c) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
    {
        return Some(c);
    }
    if s.len() == 1 {
        return s.chars().next();
    }
    let c = match s {
        "space" => ' ',
        "exclam" => '!',
        "quotedbl" => '"',
        "numbersign" => '#',
        "dollar" => '$',
        "percent" => '%',
        "ampersand" => '&',
        "quotesingle" | "quoteright" => '\'',
        "parenleft" => '(',
        "parenright" => ')',
        "asterisk" => '*',
        "plus" => '+',
        "comma" => ',',
        "hyphen" | "minus" => '-',
        "period" => '.',
        "slash" => '/',
        "zero" => '0',
        "one" => '1',
        "two" => '2',
        "three" => '3',
        "four" => '4',
        "five" => '5',
        "six" => '6',
        "seven" => '7',
        "eight" => '8',
        "nine" => '9',
        "colon" => ':',
        "semicolon" => ';',
        "less" => '<',
        "equal" => '=',
        "greater" => '>',
        "question" => '?',
        "at" => '@',
        "bracketleft" => '[',
        "backslash" => '\\',
        "bracketright" => ']',
        "underscore" => '_',
        "quoteleft" => '‘',
        "quotedblleft" => '“',
        "quotedblright" => '”',
        "quotedblbase" => '„',
        "endash" => '–',
        "emdash" => '—',
        "bullet" => '•',
        "ellipsis" => '…',
        "fi" => 'ﬁ',
        "fl" => 'ﬂ',
        "Euro" => '€',
        "degree" => '°',
        "abreve" => 'ă',
        "Abreve" => 'Ă',
        "acircumflex" => 'â',
        "Acircumflex" => 'Â',
        "icircumflex" => 'î',
        "Icircumflex" => 'Î',
        "scommaaccent" => 'ș',
        "Scommaaccent" => 'Ș',
        "tcommaaccent" => 'ț',
        "Tcommaaccent" => 'Ț',
        "scedilla" => 'ş',
        "Scedilla" => 'Ş',
        "tcedilla" => 'ţ',
        "Tcedilla" => 'Ţ',
        "aacute" => 'á',
        "Aacute" => 'Á',
        "eacute" => 'é',
        "Eacute" => 'É',
        "iacute" => 'í',
        "Iacute" => 'Í',
        "oacute" => 'ó',
        "Oacute" => 'Ó',
        "odieresis" => 'ö',
        "Odieresis" => 'Ö',
        "ohungarumlaut" => 'ő',
        "Ohungarumlaut" => 'Ő',
        "uacute" => 'ú',
        "Uacute" => 'Ú',
        "udieresis" => 'ü',
        "Udieresis" => 'Ü',
        "uhungarumlaut" => 'ű',
        "Uhungarumlaut" => 'Ű',
        "adieresis" => 'ä',
        "Adieresis" => 'Ä',
        "germandbls" => 'ß',
        "ccedilla" => 'ç',
        "egrave" => 'è',
        "agrave" => 'à',
        _ => return None,
    };
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmap_bfchar_and_bfrange() {
        let cmap = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
            2 beginbfchar <0001> <0218> <0002> <0074> endbfchar\n\
            1 beginbfrange <0003> <0005> <0061> endbfrange\n\
            1 beginbfrange <0010> <0011> [<0041> <00660069>] endbfrange\n\
            endcmap";
        let m = parse_cmap(cmap);
        assert_eq!(m.get(&1).map(String::as_str), Some("Ș"));
        assert_eq!(m.get(&5).map(String::as_str), Some("c"));
        assert_eq!(m.get(&0x11).map(String::as_str), Some("fi"));
    }

    #[test]
    fn glyph_names() {
        assert_eq!(glyph_char(b"scommaaccent"), Some('ș'));
        assert_eq!(glyph_char(b"uni0103"), Some('ă'));
        assert_eq!(glyph_char(b"A"), Some('A'));
        assert_eq!(glyph_char(b"nonsense"), None);
    }
}
