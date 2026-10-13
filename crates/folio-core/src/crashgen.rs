//! Inputs for the anti-crash gate (`cargo xtask crash`, PLAN.md §4).
//!
//! Valid sample files that exercise the parser's paths, and a deterministic
//! mutator that damages them in the ways real-world and hostile files are
//! damaged. Every stage adds its own samples and runs its own operations on
//! the mutants; nothing may panic, hang, or allocate without bound.

use crate::document::Document;
use crate::filters::flate_encode;
use crate::object::{Dict, Object};

/// Small, fast, deterministic PRNG (xorshift64*). Same seed, same mutants.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform-ish in `0..n` (0 when `n` is 0).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }
}

/// Valid PDFs covering classic xref tables, xref streams with PNG
/// predictors, object streams, incremental updates and Flate content.
pub fn core_samples() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut doc = Document::new();
    for text in [
        "Hello Folio",
        "Second page\nwith two lines",
        "(parens) \\ back",
    ] {
        if doc.add_text_page(text).is_ok() {}
    }
    if let Ok(b) = doc.to_bytes() {
        out.push(incremental_update(&b));
        out.push(b);
    }
    out.push(xref_stream_sample());
    out
}

/// Append an incremental update that changes the document title.
fn incremental_update(base: &[u8]) -> Vec<u8> {
    let Ok(doc) = Document::from_bytes(base) else {
        return base.to_vec();
    };
    let Ok(root) = doc.catalog_ref() else {
        return base.to_vec();
    };
    let num = doc.max_num() + 1;
    let prev = crate::parser::rfind(base, b"startxref")
        .and_then(|p| {
            let mut l = crate::lexer::Lexer::new(base, p + 9);
            l.token().ok().flatten()
        })
        .and_then(|t| match t {
            crate::lexer::Token::Obj(Object::Int(n)) => Some(n),
            _ => None,
        })
        .unwrap_or(0);
    let mut out = base.to_vec();
    let off = out.len();
    out.extend_from_slice(format!("{num} 0 obj\n<< /Title (Updated) >>\nendobj\n").as_bytes());
    let xref = out.len();
    out.extend_from_slice(
        format!(
            "xref\n{num} 1\n{off:010} 00000 n\r\ntrailer\n<< /Size {} /Root {} 0 R /Info {num} 0 R /Prev {prev} >>\nstartxref\n{xref}\n%%EOF\n",
            num + 1,
            root.num
        )
        .as_bytes(),
    );
    out
}

/// A PDF 1.5-style file: objects 1–3 live in an object stream, and the
/// cross-reference stream uses the PNG Up predictor.
pub fn xref_stream_sample() -> Vec<u8> {
    let content = b"BT /F1 18 Tf 72 700 Td (Packed objects) Tj ET";
    let objs: [&[u8]; 3] = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /Resources << /Font << /F1 7 0 R >> >> >>",
    ];
    let mut header = String::new();
    let mut body = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        header.push_str(&format!("{} {} ", i + 1, body.len()));
        body.extend_from_slice(o);
        body.push(b'\n');
    }
    let first = header.len();
    let mut packed = header.into_bytes();
    packed.extend_from_slice(&body);
    let packed = flate_encode(&packed);

    let mut out = b"%PDF-1.5\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = [0usize; 8];
    let c = flate_encode(content);
    let mut push = |out: &mut Vec<u8>, n: usize, bytes: Vec<u8>| {
        if let Some(slot) = offsets.get_mut(n) {
            *slot = out.len();
        }
        out.extend_from_slice(&bytes);
    };
    let mut o4 = format!(
        "4 0 obj\n<< /Length {} /Filter /FlateDecode >>\nstream\n",
        c.len()
    )
    .into_bytes();
    o4.extend_from_slice(&c);
    o4.extend_from_slice(b"\nendstream\nendobj\n");
    push(&mut out, 4, o4);
    let mut o5 = format!(
        "5 0 obj\n<< /Type /ObjStm /N 3 /First {first} /Length {} /Filter /FlateDecode >>\nstream\n",
        packed.len()
    )
    .into_bytes();
    o5.extend_from_slice(&packed);
    o5.extend_from_slice(b"\nendstream\nendobj\n");
    push(&mut out, 5, o5);
    push(
        &mut out,
        7,
        b"7 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n".to_vec(),
    );
    let xref_at = out.len();
    // Rows: type (1 byte), field 2 (4 bytes), field 3 (2 bytes).
    let rows: Vec<[u8; 7]> = (0..8u32)
        .map(|n| {
            let (t, f2, f3): (u8, u32, u16) = match n {
                0 => (0, 0, 65535),
                1..=3 => (2, 5, (n - 1) as u16),
                6 => (1, xref_at as u32, 0),
                _ => (1, offsets.get(n as usize).copied().unwrap_or(0) as u32, 0),
            };
            let mut r = [0u8; 7];
            r[0] = t;
            r[1..5].copy_from_slice(&f2.to_be_bytes());
            r[5..7].copy_from_slice(&f3.to_be_bytes());
            r
        })
        .collect();
    // PNG "Up" predictor: each row stores its difference from the row above.
    let mut predicted = Vec::new();
    let mut prev = [0u8; 7];
    for r in &rows {
        predicted.push(2u8);
        for (a, b) in r.iter().zip(prev.iter()) {
            predicted.push(a.wrapping_sub(*b));
        }
        prev = *r;
    }
    let xs = flate_encode(&predicted);
    let mut o6 = format!(
        "6 0 obj\n<< /Type /XRef /Size 8 /W [1 4 2] /Root 1 0 R /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 7 >> /Length {} >>\nstream\n",
        xs.len()
    )
    .into_bytes();
    o6.extend_from_slice(&xs);
    o6.extend_from_slice(b"\nendstream\nendobj\n");
    out.extend_from_slice(&o6);
    out.extend_from_slice(format!("startxref\n{xref_at}\n%%EOF\n").as_bytes());
    out
}

/// Hand-written hostile inputs that once crashed (or could crash) readers.
pub fn nasty_inputs() -> Vec<Vec<u8>> {
    let mut v: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"%PDF-1.7".to_vec(),
        b"%PDF-1.7\nstartxref\n999999999999\n%%EOF".to_vec(),
        b"%PDF-1.7\nstartxref\n-5\n%%EOF".to_vec(),
        b"%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 1 0 R >> endobj\ntrailer << /Root 1 0 R >>"
            .to_vec(),
        b"%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj 2 0 obj << /Type /Pages /Kids [2 0 R 2 0 R] /Count 2 >> endobj trailer << /Root 1 0 R >>".to_vec(),
        b"xref\n0 4294967295\ntrailer <<>>\nstartxref\n0\n%%EOF".to_vec(),
        b"1 0 obj << /Length 99999999999 >> stream\nabc".to_vec(),
    ];
    let mut deep = b"%PDF-1.7\n1 0 obj ".to_vec();
    deep.extend(std::iter::repeat_n(b'[', 200_000));
    v.push(deep);
    let mut dicts = b"%PDF-1.7\n1 0 obj ".to_vec();
    dicts.extend(std::iter::repeat_n(b"<</A ".as_slice(), 50_000).flatten());
    v.push(dicts);
    v
}

/// Damage `input` in one of several ways.
pub fn mutate(input: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut d = input.to_vec();
    let rounds = 1 + rng.below(4);
    for _ in 0..rounds {
        let len = d.len();
        match rng.below(9) {
            // Flip random bytes.
            0 => {
                for _ in 0..1 + rng.below(8) {
                    let i = rng.below(len);
                    if let Some(b) = d.get_mut(i) {
                        *b ^= 1 << rng.below(8);
                    }
                }
            }
            // Truncate.
            1 => d.truncate(rng.below(len + 1)),
            // Delete a span.
            2 => {
                let a = rng.below(len);
                let b = (a + rng.below(64)).min(len);
                d.drain(a..b);
            }
            // Duplicate a span somewhere else.
            3 => {
                let a = rng.below(len);
                let b = (a + rng.below(256)).min(len);
                let chunk: Vec<u8> = d.get(a..b).unwrap_or_default().to_vec();
                let at = rng.below(d.len() + 1);
                d.splice(at..at, chunk);
            }
            // Replace a number with an extreme one.
            4 => {
                let extremes: [&[u8]; 6] = [
                    b"0",
                    b"-1",
                    b"4294967295",
                    b"99999999999999999999",
                    b"-2147483648",
                    b"1e308",
                ];
                if let Some(pos) = (0..8)
                    .map(|_| rng.below(len))
                    .find(|&i| d.get(i).is_some_and(u8::is_ascii_digit))
                {
                    let mut end = pos;
                    while d.get(end).is_some_and(u8::is_ascii_digit) {
                        end += 1;
                    }
                    let pick = extremes
                        .get(rng.below(extremes.len()))
                        .copied()
                        .unwrap_or(b"0");
                    d.splice(pos..end, pick.iter().copied());
                }
            }
            // Insert PDF syntax at random.
            5 => {
                let tokens: [&[u8]; 12] = [
                    b"<<",
                    b">>",
                    b"[",
                    b"]",
                    b"(",
                    b")",
                    b" 0 R ",
                    b"stream\n",
                    b"endstream",
                    b"endobj",
                    b" obj ",
                    b"/Kids",
                ];
                let t = tokens.get(rng.below(tokens.len())).copied().unwrap_or(b"[");
                let at = rng.below(len + 1);
                d.splice(at..at, t.iter().copied());
            }
            // Swap two spans.
            6 if len > 8 => {
                let a = rng.below(len / 2);
                let b = len / 2 + rng.below(len / 2);
                let n = rng.below(32).min(len / 2 - a).min(len - b);
                for i in 0..n {
                    d.swap(a + i, b + i);
                }
            }
            // Zero a span (destroys compressed data and offsets).
            7 => {
                let a = rng.below(len);
                let b = (a + rng.below(128)).min(len);
                if let Some(s) = d.get_mut(a..b) {
                    s.fill(b'0');
                }
            }
            // Random bytes.
            _ => {
                let at = rng.below(len + 1);
                let n = rng.below(32);
                let junk: Vec<u8> = (0..n).map(|_| rng.next_u64() as u8).collect();
                d.splice(at..at, junk);
            }
        }
    }
    d
}

/// Everything the core offers, run on one input. Errors are fine; only a
/// panic (or the caller's timeout) is a failure.
pub fn exercise_core(bytes: &[u8]) {
    let Ok(mut doc) = Document::from_bytes(bytes) else {
        return;
    };
    let pages = doc.pages().unwrap_or_default();
    for p in pages.iter().take(64) {
        let _ = doc.media_box(*p);
        let _ = doc.page_attr(*p, b"Resources");
        if let Some(c) = doc
            .get(*p)
            .and_then(Object::as_dict)
            .and_then(|d| d.get(b"Contents"))
            && let Object::Stream(s) = doc.resolve(c)
        {
            let _ = crate::filters::decode_stream(s);
        }
    }
    let _ = doc.info().map(Dict::len);
    if let Ok(out) = doc.to_bytes() {
        // What we write must always read back.
        let _ = Document::from_bytes(&out);
    }
    if doc.materialize_pages().is_ok() {
        let _ = doc.set_pages(&pages);
    }
    doc.prune();
    let _ = doc.to_bytes();
}
