//! Loading a file: cross-reference tables and streams (§7.5.4, §7.5.8),
//! object streams (§7.5.7), incremental updates, and a full rebuild by
//! scanning when the cross-reference data is missing or wrong.

use std::collections::{BTreeMap, HashSet};

use crate::crypt::Security;
use crate::error::{Error, Result};
use crate::filters::decode_stream;
use crate::lexer::{Lexer, Token, is_regular, is_ws};
use crate::object::{Dict, Object};
use crate::parser::{Parser, find, parse_indirect, rfind};

/// Hard cap on objects per file, against xref tables that claim billions.
pub const MAX_OBJECTS: usize = 5_000_000;

#[derive(Clone, Copy, Debug)]
enum Entry {
    Free,
    Offset(usize),
    InStream { stm: u32 },
}

pub(crate) struct Loaded {
    pub objects: BTreeMap<u32, Object>,
    pub trailer: Dict,
    pub repairs: Vec<String>,
    pub security: Option<Security>,
}

/// Objects read straight from the file, with their generation numbers,
/// before decryption and before object streams are unpacked.
type Direct = BTreeMap<u32, (u16, Object)>;

struct Raw {
    direct: Direct,
    /// Object stream number -> objects the cross-reference data puts in it.
    pending: BTreeMap<u32, Vec<u32>>,
    trailer: Dict,
    failures: usize,
}

pub(crate) fn load(data: &[u8], password: &str) -> Result<Loaded> {
    let mut repairs = Vec::new();
    let mut security: Option<Security> = None;
    let mut enc_num: Option<u32> = None;
    let (mut objects, mut trailer, failures) = match read_xref(data) {
        Ok(raw) => {
            if let Some((s, n)) = open_security(&raw.trailer, &raw.direct, password)? {
                security = Some(s);
                enc_num = n;
            }
            let (objs, f) = finish(raw.direct, Some(&raw.pending), security.as_ref(), enc_num);
            (objs, raw.trailer, raw.failures + f)
        }
        Err(e) => {
            repairs.push(format!(
                "cross-reference data unusable ({e}); rebuilt by scanning"
            ));
            (BTreeMap::new(), Dict::new(), usize::MAX)
        }
    };

    let root_ok = |t: &Dict, objs: &BTreeMap<u32, Object>| {
        t.get(b"Root")
            .and_then(Object::as_objref)
            .is_some_and(|r| objs.contains_key(&r.num))
    };
    if failures > 0 || !root_ok(&trailer, &objects) {
        if failures > 0 && failures != usize::MAX {
            repairs.push(format!(
                "{failures} cross-reference entries pointed to the wrong place; rebuilt by scanning"
            ));
        } else if failures == 0 {
            repairs.push(
                "document catalog not reachable from the trailer; rebuilt by scanning".into(),
            );
        }
        let (direct, candidates) = scan(data);
        if security.is_none()
            && let Some(t) = candidates.iter().rev().find(|t| t.contains(b"Encrypt"))
            && let Some((s, n)) = open_security(t, &direct, password)?
        {
            security = Some(s);
            enc_num = n;
        }
        let (scanned, _) = finish(direct, None, security.as_ref(), enc_num);
        for (num, obj) in scanned {
            objects.entry(num).or_insert(obj);
        }
        if !root_ok(&trailer, &objects) {
            trailer = candidates
                .into_iter()
                .rev()
                .find(|t| root_ok(t, &objects))
                .unwrap_or_default();
        }
    }
    if objects.is_empty() {
        return Err(Error::Invalid(
            "this file contains no PDF objects; it is not a PDF or it is truncated".into(),
        ));
    }
    if trailer.get(b"Root").and_then(Object::as_objref).is_none() {
        // Last resort: any object that says it is a catalog.
        let cat = objects
            .iter()
            .find_map(|(n, o)| (o.as_dict()?.get_name(b"Type")? == b"Catalog").then_some(*n));
        match cat {
            Some(n) => {
                trailer.set("Root", Object::Ref(crate::ObjRef::new(n, 0)));
                repairs
                    .push("trailer had no /Root; used the catalog object found by scanning".into());
            }
            None => {
                return Err(Error::Invalid(
                    "no document catalog found; the file is too damaged to open".into(),
                ));
            }
        }
    }
    trailer.remove(b"Encrypt");
    Ok(Loaded {
        objects,
        trailer,
        repairs,
        security,
    })
}

/// The security handler named by the trailer, opened with `password`.
/// Returns it with the number of the /Encrypt object (never decrypted).
fn open_security(
    trailer: &Dict,
    direct: &Direct,
    password: &str,
) -> Result<Option<(Security, Option<u32>)>> {
    let (dict, num) = match trailer.get(b"Encrypt") {
        None | Some(Object::Null) => return Ok(None),
        Some(Object::Dict(d)) => (d.clone(), None),
        Some(Object::Ref(r)) => match direct.get(&r.num).and_then(|(_, o)| o.as_dict()) {
            Some(d) => (d.clone(), Some(r.num)),
            None => {
                return Err(Error::Invalid(
                    "the file is encrypted but its encryption dictionary is missing".into(),
                ));
            }
        },
        Some(_) => {
            return Err(Error::Invalid(
                "the encryption dictionary of the file is damaged".into(),
            ));
        }
    };
    let id0 = match trailer.get(b"ID") {
        Some(Object::Array(a)) => a
            .first()
            .and_then(Object::as_str)
            .unwrap_or_default()
            .to_vec(),
        _ => Vec::new(),
    };
    Ok(Some((Security::open(&dict, &id0, password)?, num)))
}

/// Decrypt the direct objects, then unpack object streams: the ones the
/// cross-reference data asks for, or (when rebuilding) every one found.
fn finish(
    direct: Direct,
    pending: Option<&BTreeMap<u32, Vec<u32>>>,
    sec: Option<&Security>,
    enc_num: Option<u32>,
) -> (BTreeMap<u32, Object>, usize) {
    let mut objects = BTreeMap::new();
    for (num, (generation, mut obj)) in direct {
        if Some(num) == enc_num {
            continue;
        }
        if let Some(s) = sec {
            s.decrypt_object(num, generation, &mut obj);
        }
        objects.insert(num, obj);
    }
    let mut failures = 0usize;
    match pending {
        Some(pending) => {
            for (stm, wanted) in pending {
                let found = match objects.get(stm) {
                    Some(Object::Stream(s)) => read_object_stream(s).ok(),
                    _ => None,
                };
                let Some(found) = found else {
                    failures += wanted.len();
                    continue;
                };
                for num in wanted {
                    match found.get(num) {
                        Some(o) => {
                            objects.insert(*num, o.clone());
                        }
                        None => failures += 1,
                    }
                }
            }
        }
        None => {
            let streams: Vec<crate::Stream> = objects
                .values()
                .filter_map(|o| match o {
                    Object::Stream(s) if s.dict.get_name(b"Type") == Some(b"ObjStm") => {
                        Some(s.clone())
                    }
                    _ => None,
                })
                .collect();
            for s in streams {
                if let Ok(found) = read_object_stream(&s) {
                    for (num, o) in found {
                        objects.entry(num).or_insert(o);
                    }
                }
            }
        }
    }
    (objects, failures)
}

fn read_xref(data: &[u8]) -> Result<Raw> {
    let start = find_startxref(data)?;
    let mut entries: BTreeMap<u32, Entry> = BTreeMap::new();
    let mut trailer: Option<Dict> = None;
    let mut queue = vec![start];
    let mut seen = HashSet::new();
    while let Some(off) = queue.pop() {
        if !seen.insert(off) || seen.len() > 256 {
            continue;
        }
        let (sect, dict) = parse_section(data, off)?;
        merge(&mut entries, sect);
        // Hybrid files: the /XRefStm section takes precedence over /Prev.
        if let Some(x) = dict
            .get_int(b"XRefStm")
            .and_then(|x| usize::try_from(x).ok())
            && seen.insert(x)
            && let Ok((sect, _)) = parse_section(data, x)
        {
            merge(&mut entries, sect);
        }
        if let Some(p) = dict.get_int(b"Prev").and_then(|p| usize::try_from(p).ok()) {
            queue.push(p);
        }
        if trailer.is_none() {
            trailer = Some(dict);
        }
    }
    let trailer = trailer.ok_or_else(|| Error::parse(start, "no trailer"))?;

    let mut direct = BTreeMap::new();
    let mut failures = 0usize;
    let mut pending: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (&num, &e) in &entries {
        match e {
            Entry::Free => {}
            Entry::Offset(off) => match parse_indirect(data, off) {
                Ok((id, obj)) if id.num == num => {
                    direct.insert(num, (id.generation, obj));
                }
                _ => failures += 1,
            },
            Entry::InStream { stm, .. } => pending.entry(stm).or_default().push(num),
        }
    }
    Ok(Raw {
        direct,
        pending,
        trailer,
        failures,
    })
}

fn merge(entries: &mut BTreeMap<u32, Entry>, sect: Vec<(u32, Entry)>) {
    // Sections are read newest first, so the first entry seen wins.
    for (num, e) in sect {
        if entries.len() >= MAX_OBJECTS {
            return;
        }
        entries.entry(num).or_insert(e);
    }
}

fn find_startxref(data: &[u8]) -> Result<usize> {
    let tail_start = data.len().saturating_sub(4096);
    let tail = data.get(tail_start..).unwrap_or_default();
    let at = rfind(tail, b"startxref")
        .map(|p| p + tail_start)
        .or_else(|| rfind(data, b"startxref"))
        .ok_or_else(|| Error::parse(data.len(), "no `startxref`"))?;
    let mut lex = Lexer::new(data, at + 9);
    match lex.token()? {
        Some(Token::Obj(Object::Int(n))) => {
            usize::try_from(n).map_err(|_| Error::parse(at, "negative startxref"))
        }
        _ => Err(Error::parse(at, "startxref without an offset")),
    }
}

fn parse_section(data: &[u8], off: usize) -> Result<(Vec<(u32, Entry)>, Dict)> {
    if off >= data.len() {
        return Err(Error::parse(
            off,
            "cross-reference offset is past the end of the file",
        ));
    }
    let mut lex = Lexer::new(data, off);
    lex.skip_ws();
    if data.get(lex.pos..).is_some_and(|r| r.starts_with(b"xref")) {
        lex.pos += 4;
        return parse_table(lex);
    }
    parse_xref_stream(data, off)
}

fn parse_table(mut lex: Lexer<'_>) -> Result<(Vec<(u32, Entry)>, Dict)> {
    let mut out = Vec::new();
    loop {
        let at = lex.pos;
        match lex.token()? {
            Some(Token::Obj(Object::Int(first))) => {
                let Some(Token::Obj(Object::Int(count))) = lex.token()? else {
                    return Err(Error::parse(at, "bad cross-reference subsection header"));
                };
                let first =
                    u32::try_from(first).map_err(|_| Error::parse(at, "bad object number"))?;
                let count = usize::try_from(count).unwrap_or(0).min(MAX_OBJECTS);
                for i in 0..count {
                    let a = lex.token()?;
                    let _gen = lex.token()?;
                    let kind = lex.token()?;
                    let (Some(Token::Obj(Object::Int(a))), Some(Token::Keyword(k))) = (a, kind)
                    else {
                        return Err(Error::parse(lex.pos, "bad cross-reference entry"));
                    };
                    let Some(num) = u32::try_from(i).ok().and_then(|i| first.checked_add(i)) else {
                        break;
                    };
                    let e = match (k.as_slice(), usize::try_from(a)) {
                        (b"n", Ok(o)) if o > 0 => Entry::Offset(o),
                        _ => Entry::Free,
                    };
                    out.push((num, e));
                }
            }
            Some(Token::Keyword(k)) if k == b"trailer" => {
                let mut p = Parser::new(lex.data(), lex.pos);
                let t = p.object()?;
                let dict = t
                    .as_dict()
                    .cloned()
                    .ok_or_else(|| Error::parse(lex.pos, "trailer is not a dictionary"))?;
                return Ok((out, dict));
            }
            _ => {
                return Err(Error::parse(
                    at,
                    "expected `trailer` after the cross-reference table",
                ));
            }
        }
    }
}

fn parse_xref_stream(data: &[u8], off: usize) -> Result<(Vec<(u32, Entry)>, Dict)> {
    let (_, obj) = parse_indirect(data, off)?;
    let Object::Stream(s) = obj else {
        return Err(Error::parse(off, "expected a cross-reference stream"));
    };
    let raw = decode_stream(&s)?;
    let w: Vec<usize> = s
        .dict
        .get(b"W")
        .and_then(Object::as_array)
        .map(|a| {
            a.iter()
                .map(|x| {
                    x.as_int()
                        .and_then(|i| usize::try_from(i).ok())
                        .unwrap_or(0)
                        .min(8)
                })
                .collect()
        })
        .unwrap_or_default();
    let (w0, w1, w2) = match w.as_slice() {
        [a, b, c] => (*a, *b, *c),
        _ => return Err(Error::parse(off, "cross-reference stream has a bad /W")),
    };
    let row = w0 + w1 + w2;
    if row == 0 {
        return Err(Error::parse(off, "cross-reference stream has empty rows"));
    }
    let size = s.dict.get_int(b"Size").unwrap_or(0).max(0);
    let index: Vec<i64> = match s.dict.get(b"Index").and_then(Object::as_array) {
        Some(a) => a.iter().filter_map(Object::as_int).collect(),
        None => vec![0, size],
    };
    let mut out = Vec::new();
    let mut rows = raw.chunks_exact(row);
    for pair in index.chunks(2) {
        let (Some(&first), Some(&count)) = (pair.first(), pair.get(1)) else {
            break;
        };
        for i in 0..count.clamp(0, MAX_OBJECTS as i64) {
            let Some(r) = rows.next() else {
                return Ok((out, s.dict));
            };
            let Some(num) = u32::try_from(first + i).ok() else {
                continue;
            };
            let f = |from: usize, len: usize| {
                r.get(from..from + len)
                    .unwrap_or_default()
                    .iter()
                    .fold(0u64, |acc, &b| (acc << 8) | u64::from(b))
            };
            let kind = if w0 == 0 { 1 } else { f(0, w0) };
            let b = f(w0, w1);
            let c = f(w0 + w1, w2);
            let e = match kind {
                1 => usize::try_from(b).map_or(Entry::Free, Entry::Offset),
                2 => match (u32::try_from(b), u32::try_from(c)) {
                    (Ok(stm), Ok(_)) => Entry::InStream { stm },
                    _ => Entry::Free,
                },
                _ => Entry::Free,
            };
            out.push((num, e));
        }
    }
    Ok((out, s.dict))
}

/// All objects stored inside one object stream, by number.
pub(crate) fn read_object_stream(s: &crate::Stream) -> Result<BTreeMap<u32, Object>> {
    let data = decode_stream(s)?;
    let n = s.dict.get_int(b"N").unwrap_or(0).clamp(0, 1_000_000) as usize;
    let first = usize::try_from(s.dict.get_int(b"First").unwrap_or(0)).unwrap_or(0);
    let mut lex = Lexer::new(&data, 0);
    let mut pairs = Vec::with_capacity(n.min(4096));
    for _ in 0..n {
        let (Some(Token::Obj(Object::Int(num))), Some(Token::Obj(Object::Int(rel)))) =
            (lex.token()?, lex.token()?)
        else {
            break;
        };
        if let (Ok(num), Ok(rel)) = (u32::try_from(num), usize::try_from(rel)) {
            pairs.push((num, rel));
        }
    }
    let mut out = BTreeMap::new();
    for (num, rel) in pairs {
        let Some(at) = first.checked_add(rel) else {
            continue;
        };
        let mut p = Parser::new(&data, at);
        if let Ok(o) = p.object() {
            out.entry(num).or_insert(o);
        }
    }
    Ok(out)
}

/// Rebuild the object table by scanning the whole file for `n g obj`.
/// Returns the objects found and every trailer-like dictionary, oldest first.
fn scan(data: &[u8]) -> (Direct, Vec<Dict>) {
    let mut objects = BTreeMap::new();
    let mut candidates: Vec<Dict> = Vec::new();
    let mut i = 0usize;
    while let Some(at) = find(data, b"obj", i) {
        i = at + 3;
        if data.get(at + 3).copied().is_some_and(is_regular) {
            continue;
        }
        let Some(start) = obj_header_start(data, at) else {
            continue;
        };
        if let Ok((id, obj)) = parse_indirect(data, start) {
            if obj.as_dict().and_then(|d| d.get_name(b"Type")) == Some(b"XRef")
                && let Some(d) = obj.as_dict()
            {
                candidates.push(d.clone());
            }
            // Later definitions (incremental updates) replace earlier ones.
            objects.insert(id.num, (id.generation, obj));
            if objects.len() >= MAX_OBJECTS {
                break;
            }
        }
    }
    let mut j = 0usize;
    while let Some(at) = find(data, b"trailer", j) {
        j = at + 7;
        let mut p = Parser::new(data, at + 7);
        if let Ok(Object::Dict(d)) = p.object() {
            candidates.push(d);
        }
    }
    (objects, candidates)
}

/// Given the position of `obj`, find where `num gen` begins, if it is there.
fn obj_header_start(data: &[u8], obj_at: usize) -> Option<usize> {
    let mut k = obj_at;
    let skip_ws_back = |k: &mut usize| {
        while *k > 0 && data.get(*k - 1).copied().is_some_and(is_ws) {
            *k -= 1;
        }
    };
    let digits_back = |k: &mut usize| -> bool {
        let end = *k;
        while *k > 0 && data.get(*k - 1).is_some_and(u8::is_ascii_digit) {
            *k -= 1;
        }
        *k < end
    };
    skip_ws_back(&mut k);
    if !digits_back(&mut k) {
        return None;
    }
    let before_gen = k;
    skip_ws_back(&mut k);
    if k == before_gen {
        return None;
    }
    if !digits_back(&mut k) {
        return None;
    }
    if k > 0 && data.get(k - 1).copied().is_some_and(is_regular) {
        return None;
    }
    Some(k)
}
