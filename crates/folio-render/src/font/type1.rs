//! Type 1 fonts (Adobe Type 1 Font Format, as embedded with /FontFile):
//! PFB/PFA unwrapping, eexec decryption, Subrs and CharStrings, and the
//! Type 1 charstring interpreter (including flex and seac accents).
//! Every loop and recursion is bounded; bad data yields `None`.

use std::collections::HashMap;

use folio_core::glyphs::BaseEncoding;
use folio_core::lexer::{Lexer, Token};
use folio_core::object::Object;
use tiny_skia::PathBuilder;

const MAX_SUBR_DEPTH: usize = 10;
const MAX_OPS: usize = 20_000;

pub struct Type1 {
    pub matrix: [f64; 6],
    /// Built-in encoding: code → glyph name (`None` = StandardEncoding).
    pub encoding: Option<HashMap<u8, String>>,
    charstrings: HashMap<String, Vec<u8>>,
    subrs: Vec<Vec<u8>>,
}

impl Type1 {
    pub fn parse(data: &[u8]) -> Option<Type1> {
        let data = unwrap_pfb(data);
        let at = find(&data, b"eexec")?;
        let clear = data.get(..at)?;
        let mut body = data.get(at + 5..)?;
        while body
            .first()
            .is_some_and(|b| matches!(b, b' ' | b'\r' | b'\n' | b'\t'))
        {
            body = body.get(1..)?;
        }
        let binary = if body.len() >= 4 && body.iter().take(4).all(u8::is_ascii_hexdigit) {
            hex(body)
        } else {
            body.to_vec()
        };
        let private = decrypt(&binary, 55665, 4);

        let matrix = font_matrix(clear).unwrap_or([0.001, 0.0, 0.0, 0.001, 0.0, 0.0]);
        let encoding = builtin_encoding(clear);
        let len_iv = int_after(&private, b"/lenIV").unwrap_or(4);
        let subrs = read_subrs(&private, len_iv);
        let charstrings = read_charstrings(&private, len_iv);
        if charstrings.is_empty() {
            return None;
        }
        Some(Type1 {
            matrix,
            encoding,
            charstrings,
            subrs,
        })
    }

    pub fn has_glyph(&self, name: &str) -> bool {
        self.charstrings.contains_key(name)
    }

    /// The glyph name for `code` in the font's own encoding.
    pub fn builtin_name(&self, code: u8) -> Option<String> {
        match &self.encoding {
            Some(map) => map.get(&code).cloned(),
            None => BaseEncoding::Standard.glyph_name(code).map(String::from),
        }
    }

    /// Draw a glyph (in glyph units). Returns its advance width.
    pub fn outline(&self, name: &str, pb: &mut PathBuilder) -> Option<f64> {
        let cs = self.charstrings.get(name)?;
        let mut st = State::default();
        self.run(cs, pb, &mut st, 0, (0.0, 0.0))?;
        Some(st.width)
    }

    fn run(
        &self,
        cs: &[u8],
        pb: &mut PathBuilder,
        st: &mut State,
        depth: usize,
        off: (f64, f64),
    ) -> Option<bool> {
        if depth > MAX_SUBR_DEPTH {
            return None;
        }
        let mut i = 0usize;
        while let Some(&v) = cs.get(i) {
            st.ops += 1;
            if st.ops > MAX_OPS {
                return None;
            }
            i += 1;
            match v {
                32..=246 => st.push(f64::from(v) - 139.0),
                247..=250 => {
                    let w = f64::from(*cs.get(i)?);
                    i += 1;
                    st.push((f64::from(v) - 247.0) * 256.0 + w + 108.0);
                }
                251..=254 => {
                    let w = f64::from(*cs.get(i)?);
                    i += 1;
                    st.push(-(f64::from(v) - 251.0) * 256.0 - w - 108.0);
                }
                255 => {
                    let b = cs.get(i..i + 4)?;
                    i += 4;
                    st.push(f64::from(i32::from_be_bytes([b[0], b[1], b[2], b[3]])));
                }
                1 | 3 => st.stack.clear(), // hstem, vstem
                4 => {
                    let dy = st.arg(0);
                    st.rmove(pb, 0.0, dy, off);
                }
                5 => {
                    let (dx, dy) = (st.arg(0), st.arg(1));
                    st.rline(pb, dx, dy, off);
                }
                6 => {
                    let dx = st.arg(0);
                    st.rline(pb, dx, 0.0, off);
                }
                7 => {
                    let dy = st.arg(0);
                    st.rline(pb, 0.0, dy, off);
                }
                8 => {
                    let a: Vec<f64> = (0..6).map(|k| st.arg(k)).collect();
                    st.rcurve(pb, &a, off);
                }
                9 => {
                    pb.close();
                    st.stack.clear();
                }
                10 => {
                    let idx = st.stack.pop().unwrap_or(-1.0);
                    if idx >= 0.0
                        && let Some(sub) = self.subrs.get(idx as usize)
                        && self.run(sub, pb, st, depth + 1, off)?
                    {
                        return Some(true);
                    }
                }
                11 => return Some(false), // return
                13 => {
                    // hsbw: side bearing x, width
                    st.x = st.arg(0);
                    st.y = 0.0;
                    st.width = st.arg(1);
                    st.stack.clear();
                }
                14 => return Some(true), // endchar
                21 => {
                    let (dx, dy) = (st.arg(0), st.arg(1));
                    st.rmove(pb, dx, dy, off);
                }
                22 => {
                    let dx = st.arg(0);
                    st.rmove(pb, dx, 0.0, off);
                }
                30 => {
                    let a = [0.0, st.arg(0), st.arg(1), st.arg(2), st.arg(3), 0.0];
                    st.rcurve(pb, &a, off);
                }
                31 => {
                    let a = [st.arg(0), 0.0, st.arg(1), st.arg(2), 0.0, st.arg(3)];
                    st.rcurve(pb, &a, off);
                }
                12 => {
                    let e = *cs.get(i)?;
                    i += 1;
                    match e {
                        6 => {
                            // seac: accented character from two StandardEncoding glyphs.
                            let (asb, adx, ady) = (st.arg(0), st.arg(1), st.arg(2));
                            let (b, a) = (st.arg(3), st.arg(4));
                            st.stack.clear();
                            let base = BaseEncoding::Standard.glyph_name(b.clamp(0.0, 255.0) as u8);
                            let acc = BaseEncoding::Standard.glyph_name(a.clamp(0.0, 255.0) as u8);
                            let sbx = st.x;
                            if let Some(cs) = base.and_then(|n| self.charstrings.get(n)) {
                                let mut s2 = State {
                                    ops: st.ops,
                                    ..State::default()
                                };
                                self.run(cs, pb, &mut s2, depth + 1, off)?;
                                st.ops = s2.ops;
                            }
                            if let Some(cs) = acc.and_then(|n| self.charstrings.get(n)) {
                                let mut s2 = State {
                                    ops: st.ops,
                                    ..State::default()
                                };
                                let o = (off.0 + adx - asb + sbx, off.1 + ady);
                                self.run(cs, pb, &mut s2, depth + 1, o)?;
                                st.ops = s2.ops;
                            }
                            return Some(true);
                        }
                        7 => {
                            // sbw
                            st.x = st.arg(0);
                            st.y = st.arg(1);
                            st.width = st.arg(2);
                            st.stack.clear();
                        }
                        12 => {
                            let b = st.stack.pop().unwrap_or(1.0);
                            let a = st.stack.pop().unwrap_or(0.0);
                            st.push(if b == 0.0 { 0.0 } else { a / b });
                        }
                        16 => {
                            let other = st.stack.pop().unwrap_or(-1.0) as i64;
                            let n = (st.stack.pop().unwrap_or(0.0).max(0.0) as usize).min(16);
                            let mut args = Vec::with_capacity(n);
                            for _ in 0..n {
                                args.push(st.stack.pop().unwrap_or(0.0));
                            }
                            match other {
                                1 => {
                                    st.flex = Some(Vec::new());
                                }
                                0 => {
                                    // End of flex: two curves through seven points.
                                    if let Some(pts) = st.flex.take()
                                        && pts.len() >= 7
                                    {
                                        let p: Vec<(f64, f64)> =
                                            pts.iter().skip(1).copied().collect();
                                        for c in p.chunks(3).take(2) {
                                            if let [a, b, d] = c {
                                                pb.cubic_to(
                                                    (a.0 + off.0) as f32,
                                                    (a.1 + off.1) as f32,
                                                    (b.0 + off.0) as f32,
                                                    (b.1 + off.1) as f32,
                                                    (d.0 + off.0) as f32,
                                                    (d.1 + off.1) as f32,
                                                );
                                            }
                                        }
                                        if let Some(last) = pts.last() {
                                            st.x = last.0;
                                            st.y = last.1;
                                        }
                                    }
                                    st.ps.push(st.y);
                                    st.ps.push(st.x);
                                }
                                _ => {
                                    // Other subrs (hint replacement...): echo the arguments.
                                    st.ps.extend(args.iter().copied());
                                }
                            }
                        }
                        17 => {
                            let v = st.ps.pop().unwrap_or(0.0);
                            st.push(v);
                        }
                        33 => {
                            st.x = st.arg(0);
                            st.y = st.arg(1);
                            st.stack.clear();
                        }
                        _ => st.stack.clear(), // dotsection, vstem3, hstem3
                    }
                }
                _ => st.stack.clear(),
            }
        }
        Some(false)
    }
}

#[derive(Default)]
struct State {
    x: f64,
    y: f64,
    width: f64,
    stack: Vec<f64>,
    ps: Vec<f64>,
    flex: Option<Vec<(f64, f64)>>,
    ops: usize,
    open: bool,
}

impl State {
    fn push(&mut self, v: f64) {
        if self.stack.len() < 48 {
            self.stack.push(v);
        }
    }

    /// The `k`-th argument of the current command (from the bottom).
    fn arg(&self, k: usize) -> f64 {
        self.stack.get(k).copied().unwrap_or(0.0)
    }

    fn rmove(&mut self, pb: &mut PathBuilder, dx: f64, dy: f64, off: (f64, f64)) {
        self.x += dx;
        self.y += dy;
        self.stack.clear();
        if let Some(f) = &mut self.flex {
            if f.len() < 16 {
                f.push((self.x, self.y));
            }
            return;
        }
        pb.move_to((self.x + off.0) as f32, (self.y + off.1) as f32);
        self.open = true;
    }

    fn rline(&mut self, pb: &mut PathBuilder, dx: f64, dy: f64, off: (f64, f64)) {
        self.x += dx;
        self.y += dy;
        self.stack.clear();
        pb.line_to((self.x + off.0) as f32, (self.y + off.1) as f32);
    }

    fn rcurve(&mut self, pb: &mut PathBuilder, a: &[f64], off: (f64, f64)) {
        let g = |i: usize| a.get(i).copied().unwrap_or(0.0);
        let (x1, y1) = (self.x + g(0), self.y + g(1));
        let (x2, y2) = (x1 + g(2), y1 + g(3));
        let (x3, y3) = (x2 + g(4), y2 + g(5));
        pb.cubic_to(
            (x1 + off.0) as f32,
            (y1 + off.1) as f32,
            (x2 + off.0) as f32,
            (y2 + off.1) as f32,
            (x3 + off.0) as f32,
            (y3 + off.1) as f32,
        );
        self.x = x3;
        self.y = y3;
        self.stack.clear();
    }
}

fn unwrap_pfb(data: &[u8]) -> Vec<u8> {
    if data.first() != Some(&0x80) {
        return data.to_vec();
    }
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0usize;
    while data.get(i) == Some(&0x80) {
        let kind = data.get(i + 1).copied().unwrap_or(3);
        if kind == 3 {
            break;
        }
        let Some(len) = data.get(i + 2..i + 6) else {
            break;
        };
        let len = u32::from_le_bytes([len[0], len[1], len[2], len[3]]) as usize;
        let start = i + 6;
        let end = start.saturating_add(len).min(data.len());
        out.extend_from_slice(data.get(start..end).unwrap_or_default());
        i = end;
    }
    out
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn hex(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2);
    let mut hi: Option<u8> = None;
    for &b in data {
        let v = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            b' ' | b'\r' | b'\n' | b'\t' => continue,
            _ => break,
        };
        match hi.take() {
            Some(h) => out.push(h << 4 | v),
            None => hi = Some(v),
        }
    }
    out
}

/// Type 1 decryption (eexec r = 55665, charstrings r = 4330).
pub fn decrypt(data: &[u8], key: u16, skip: usize) -> Vec<u8> {
    let mut r = key;
    let mut out = Vec::with_capacity(data.len());
    for &c in data {
        out.push(c ^ (r >> 8) as u8);
        r = (u16::from(c).wrapping_add(r))
            .wrapping_mul(52845)
            .wrapping_add(22719);
    }
    out.into_iter().skip(skip).collect()
}

/// Type 1 encryption, the inverse of [`decrypt`] (used by tests).
pub fn encrypt(plain: &[u8], key: u16) -> Vec<u8> {
    let mut r = key;
    plain
        .iter()
        .map(|&p| {
            let c = p ^ (r >> 8) as u8;
            r = (u16::from(c).wrapping_add(r))
                .wrapping_mul(52845)
                .wrapping_add(22719);
            c
        })
        .collect()
}

fn font_matrix(clear: &[u8]) -> Option<[f64; 6]> {
    let at = find(clear, b"/FontMatrix")?;
    let mut lex = Lexer::new(clear, at + 11);
    let mut v = Vec::new();
    loop {
        match lex.token().ok()?? {
            Token::ArrayStart | Token::Keyword(_) if v.is_empty() => continue,
            Token::Obj(o) => {
                v.push(o.as_f64()?);
                if v.len() == 6 {
                    break;
                }
            }
            _ => break,
        }
    }
    let m: [f64; 6] = v.try_into().ok()?;
    let ok = m.iter().all(|x| x.is_finite()) && m[0] * m[3] - m[1] * m[2] != 0.0;
    ok.then_some(m)
}

fn builtin_encoding(clear: &[u8]) -> Option<HashMap<u8, String>> {
    let at = find(clear, b"/Encoding")?;
    let rest = clear.get(at + 9..)?;
    if rest
        .iter()
        .skip_while(|b| b.is_ascii_whitespace())
        .take(16)
        .copied()
        .collect::<Vec<u8>>()
        .starts_with(b"StandardEncoding")
    {
        return None;
    }
    let mut lex = Lexer::new(rest, 0);
    let mut map = HashMap::new();
    let mut window: Vec<Token> = Vec::new();
    let mut steps = 0;
    while let Ok(Some(t)) = lex.token() {
        steps += 1;
        if steps > 4096 {
            break;
        }
        if matches!(&t, Token::Keyword(k) if k == b"def" || k == b"readonly") && !map.is_empty() {
            break;
        }
        window.push(t);
        if window.len() > 4 {
            window.remove(0);
        }
        if let [
            Token::Keyword(dup),
            Token::Obj(Object::Int(code)),
            Token::Obj(Object::Name(name)),
            Token::Keyword(put),
        ] = window.as_slice()
            && dup == b"dup"
            && put == b"put"
            && let Ok(code) = u8::try_from(*code)
        {
            map.insert(code, String::from_utf8_lossy(name).into_owned());
        }
    }
    Some(map)
}

fn int_after(data: &[u8], key: &[u8]) -> Option<usize> {
    let at = find(data, key)?;
    let mut lex = Lexer::new(data, at + key.len());
    match lex.token().ok()?? {
        Token::Obj(Object::Int(n)) => Some(usize::try_from(n).unwrap_or(4)),
        _ => None,
    }
}

/// Read `n` bytes after an `RD`/`-|` token (one space separates them).
fn binary_after(lex: &mut Lexer<'_>, n: usize) -> Option<Vec<u8>> {
    let start = lex.pos + 1;
    let bytes = lex.data().get(start..start.checked_add(n)?)?.to_vec();
    lex.pos = start + n;
    Some(bytes)
}

fn is_rd(t: &Token) -> bool {
    matches!(t, Token::Keyword(k) if k == b"RD" || k == b"-|")
}

fn decrypt_cs(bytes: &[u8], len_iv: usize) -> Vec<u8> {
    if len_iv > 64 {
        return bytes.to_vec(); // lenIV -1: not encrypted
    }
    decrypt(bytes, 4330, len_iv)
}

fn read_subrs(private: &[u8], len_iv: usize) -> Vec<Vec<u8>> {
    let Some(at) = find(private, b"/Subrs") else {
        return Vec::new();
    };
    let mut lex = Lexer::new(private, at + 6);
    let count = match lex.token() {
        Ok(Some(Token::Obj(Object::Int(n)))) => usize::try_from(n).unwrap_or(0).min(65_536),
        _ => return Vec::new(),
    };
    let mut subrs = vec![Vec::new(); count];
    let mut guard = 0;
    while guard < count * 8 + 16 {
        guard += 1;
        let Ok(Some(t)) = lex.token() else { break };
        match t {
            Token::Keyword(k) if k == b"dup" => {
                let (Ok(Some(Token::Obj(Object::Int(i)))), Ok(Some(Token::Obj(Object::Int(n))))) =
                    (lex.token(), lex.token())
                else {
                    break;
                };
                let Ok(Some(rd)) = lex.token() else { break };
                if !is_rd(&rd) {
                    break;
                }
                let Some(bytes) = binary_after(&mut lex, usize::try_from(n).unwrap_or(0)) else {
                    break;
                };
                if let Some(slot) = usize::try_from(i).ok().and_then(|i| subrs.get_mut(i)) {
                    *slot = decrypt_cs(&bytes, len_iv);
                }
            }
            Token::Keyword(k) if k == b"ND" || k == b"|-" || k == b"def" => break,
            _ => {}
        }
    }
    subrs
}

fn read_charstrings(private: &[u8], len_iv: usize) -> HashMap<String, Vec<u8>> {
    let mut out = HashMap::new();
    let Some(at) = find(private, b"/CharStrings") else {
        return out;
    };
    let mut lex = Lexer::new(private, at + 12);
    let mut guard = 0;
    while guard < 200_000 {
        guard += 1;
        let Ok(Some(t)) = lex.token() else { break };
        match t {
            Token::Obj(Object::Name(name)) => {
                let Ok(Some(Token::Obj(Object::Int(n)))) = lex.token() else {
                    continue;
                };
                let Ok(Some(rd)) = lex.token() else { break };
                if !is_rd(&rd) {
                    continue;
                }
                let Some(bytes) = binary_after(&mut lex, usize::try_from(n).unwrap_or(0)) else {
                    break;
                };
                out.insert(
                    String::from_utf8_lossy(&name).into_owned(),
                    decrypt_cs(&bytes, len_iv),
                );
            }
            Token::Keyword(k) if k == b"end" => break,
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eexec_roundtrip() {
        let plain = b"abcd/Private 8 dict dup begin";
        assert_eq!(decrypt(&encrypt(plain, 55665), 55665, 0), plain);
    }
}
