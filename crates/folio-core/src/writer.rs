//! Serialisation. Folio always writes a complete, clean file: objects
//! renumbered densely from 1, a classic cross-reference table, one trailer.
//! Damaged input therefore comes out repaired.

use std::collections::BTreeMap;
use std::io::Write as _;

use crate::document::Document;
use crate::error::Result;
use crate::lexer::is_regular;
use crate::object::{Dict, Object};

/// The bytes of one direct object (a stream is written as its dictionary).
pub fn serialize(o: &Object) -> Vec<u8> {
    let mut out = Vec::new();
    write_obj(&mut out, o);
    out
}

pub fn write(doc: &Document) -> Result<Vec<u8>> {
    // Dense renumbering; references to missing objects become null (§7.3.10).
    let map: BTreeMap<u32, u32> = doc
        .objects
        .keys()
        .enumerate()
        .map(|(i, &n)| (n, u32::try_from(i + 1).unwrap_or(u32::MAX)))
        .collect();
    let remap = |o: &mut Object| {
        o.for_each_ref_mut(&mut |r| {
            let new = match r {
                Object::Ref(x) => map.get(&x.num).copied(),
                _ => None,
            };
            *r = match new {
                Some(n) => Object::Ref(crate::ObjRef::new(n, 0)),
                None => Object::Null,
            };
        });
    };

    let version = if doc.version.is_empty() {
        "1.7"
    } else {
        &doc.version
    };
    let mut out = Vec::with_capacity(1024);
    let _ = write!(out, "%PDF-{version}\n%\u{e2}\u{e3}\u{cf}\u{d3}\n");
    let mut offsets = Vec::with_capacity(map.len());
    for (num, obj) in &doc.objects {
        let new = map.get(num).copied().unwrap_or(0);
        let mut obj = obj.clone();
        remap(&mut obj);
        if let Some(sec) = &doc.security {
            sec.encrypt_object(new, 0, &mut obj);
        }
        offsets.push(out.len());
        let _ = writeln!(out, "{new} 0 obj");
        match &mut obj {
            Object::Stream(s) => {
                s.dict.set("Length", Object::Int(s.data.len() as i64));
                write_dict(&mut out, &s.dict);
                out.extend_from_slice(b"\nstream\n");
                out.extend_from_slice(&s.data);
                out.extend_from_slice(b"\nendstream");
            }
            other => write_obj(&mut out, other),
        }
        out.extend_from_slice(b"\nendobj\n");
    }

    // The encryption dictionary itself is written in the clear.
    let mut encrypt_ref = None;
    if let Some(sec) = &doc.security {
        let num = u32::try_from(offsets.len() + 1).unwrap_or(u32::MAX);
        offsets.push(out.len());
        let _ = writeln!(out, "{num} 0 obj");
        write_dict(&mut out, &sec.dict);
        out.extend_from_slice(b"\nendobj\n");
        encrypt_ref = Some(num);
    }

    let xref_at = out.len();
    let _ = write!(out, "xref\n0 {}\n0000000000 65535 f\r\n", offsets.len() + 1);
    for off in &offsets {
        let _ = write!(out, "{off:010} 00000 n\r\n");
    }
    let mut trailer = Dict::new();
    trailer.set("Size", Object::Int(offsets.len() as i64 + 1));
    for key in [&b"Root"[..], b"Info", b"ID"] {
        if let Some(v) = doc.trailer.get(key) {
            let mut v = v.clone();
            remap(&mut v);
            if v != Object::Null {
                trailer.set(key, v);
            }
        }
    }
    if let Some(n) = encrypt_ref {
        trailer.set("Encrypt", Object::Ref(crate::ObjRef::new(n, 0)));
    }
    out.extend_from_slice(b"trailer\n");
    write_dict(&mut out, &trailer);
    let _ = write!(out, "\nstartxref\n{xref_at}\n%%EOF\n");
    Ok(out)
}

fn write_obj(out: &mut Vec<u8>, o: &Object) {
    match o {
        Object::Null => out.extend_from_slice(b"null"),
        Object::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Object::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Object::Real(r) => out.extend_from_slice(fmt_real(*r).as_bytes()),
        Object::Name(n) => write_name(out, n),
        Object::Str(s) => write_string(out, s),
        Object::Array(a) => {
            out.push(b'[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(b' ');
                }
                write_obj(out, x);
            }
            out.push(b']');
        }
        Object::Dict(d) => write_dict(out, d),
        Object::Stream(s) => write_dict(out, &s.dict),
        Object::Ref(r) => {
            let _ = write!(out, "{} {} R", r.num, r.generation);
        }
    }
}

fn write_dict(out: &mut Vec<u8>, d: &Dict) {
    out.extend_from_slice(b"<<");
    for (k, v) in d.iter() {
        write_name(out, k);
        out.push(b' ');
        write_obj(out, v);
    }
    out.extend_from_slice(b">>");
}

fn write_name(out: &mut Vec<u8>, n: &[u8]) {
    out.push(b'/');
    for &b in n {
        if is_regular(b) && b != b'#' && (0x21..=0x7e).contains(&b) {
            out.push(b);
        } else {
            let _ = write!(out, "#{b:02X}");
        }
    }
}

fn write_string(out: &mut Vec<u8>, s: &[u8]) {
    let printable = s
        .iter()
        .filter(|&&b| (0x20..0x7f).contains(&b) || b == b'\n' || b == b'\r' || b == b'\t')
        .count();
    if printable * 10 >= s.len() * 9 {
        out.push(b'(');
        for &b in s {
            match b {
                b'(' | b')' | b'\\' => {
                    out.push(b'\\');
                    out.push(b);
                }
                b'\r' => out.extend_from_slice(b"\\r"),
                b'\n' => out.extend_from_slice(b"\\n"),
                0x20..=0x7e | b'\t' => out.push(b),
                _ => {
                    let _ = write!(out, "\\{b:03o}");
                }
            }
        }
        out.push(b')');
    } else {
        out.push(b'<');
        for &b in s {
            let _ = write!(out, "{b:02X}");
        }
        out.push(b'>');
    }
}

/// Shortest faithful form, never exponent notation (PDF has none).
pub fn fmt_real(r: f64) -> String {
    if !r.is_finite() {
        return "0".into();
    }
    let mut s = format!("{r:.6}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" || s.is_empty() {
        s = "0".into();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn roundtrip(o: &Object) -> Object {
        let bytes = serialize(o);
        Parser::new(&bytes, 0).object().unwrap()
    }

    #[test]
    fn objects_roundtrip() {
        let mut d = Dict::new();
        d.set("A B", Object::Str(b"x(y)\\z\n".to_vec()));
        d.set("Bin", Object::Str(vec![0, 1, 2, 255, 254]));
        d.set("R", Object::Real(-0.125));
        d.set("L", Object::Array(vec![Object::Null, Object::Bool(true)]));
        let o = Object::Dict(d);
        assert_eq!(roundtrip(&o), o);
    }

    #[test]
    fn reals_never_use_exponents() {
        assert_eq!(fmt_real(1e-7), "0");
        assert_eq!(fmt_real(2.5), "2.5");
        assert_eq!(fmt_real(f64::NAN), "0");
        assert_eq!(fmt_real(-0.0), "0");
    }
}
