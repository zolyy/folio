//! PDF functions (ISO 32000-2 §7.10): sampled (type 0), exponential (2),
//! stitching (3) and PostScript calculator (4). Used by Separation/DeviceN
//! colour spaces and, later, shadings. Evaluation never panics and is bounded.

use folio_core::filters::decode_stream;
use folio_core::lexer::Token;
use folio_core::parser::Parser;
use folio_core::{Dict, Document, Object};

const MAX_DEPTH: usize = 8;
const MAX_SAMPLES: usize = 16 * 1024 * 1024;
const MAX_PS_OPS: usize = 4096;
const MAX_PS_STEPS: usize = 100_000;
const MAX_PS_STACK: usize = 128;

#[derive(Clone, Debug)]
pub enum Function {
    Sampled {
        domain: Vec<f64>,
        range: Vec<f64>,
        size: Vec<usize>,
        encode: Vec<f64>,
        decode: Vec<f64>,
        samples: Vec<f64>,
        outputs: usize,
    },
    Exponential {
        domain: Vec<f64>,
        c0: Vec<f64>,
        c1: Vec<f64>,
        n: f64,
    },
    Stitching {
        domain: Vec<f64>,
        functions: Vec<Function>,
        bounds: Vec<f64>,
        encode: Vec<f64>,
    },
    PostScript {
        domain: Vec<f64>,
        range: Vec<f64>,
        program: Vec<Ps>,
    },
    /// An array of 1-output functions, one per output component.
    Array(Vec<Function>),
}

/// A PostScript calculator instruction. `If`/`IfElse` hold the indices of
/// their blocks' instruction ranges.
#[derive(Clone, Debug, PartialEq)]
pub enum Ps {
    Num(f64),
    Bool(bool),
    Op(Vec<u8>),
    /// Skip this many instructions when the condition is false.
    If {
        len: usize,
    },
    IfElse {
        then_len: usize,
        else_len: usize,
    },
}

fn nums(doc: &Document, d: &Dict, key: &[u8]) -> Vec<f64> {
    match doc.resolve_key(d, key) {
        Some(Object::Array(a)) => a
            .iter()
            .filter_map(|x| doc.resolve(x).as_f64())
            .take(4096)
            .collect(),
        _ => Vec::new(),
    }
}

impl Function {
    pub fn parse(doc: &Document, o: &Object) -> Option<Function> {
        Self::parse_depth(doc, o, 0)
    }

    fn parse_depth(doc: &Document, o: &Object, depth: usize) -> Option<Function> {
        if depth > MAX_DEPTH {
            return None;
        }
        let o = doc.resolve(o);
        if let Object::Array(items) = o {
            let fs: Vec<Function> = items
                .iter()
                .take(32)
                .filter_map(|f| Self::parse_depth(doc, f, depth + 1))
                .collect();
            return (!fs.is_empty()).then_some(Function::Array(fs));
        }
        let d = o.as_dict()?;
        let domain = nums(doc, d, b"Domain");
        match d.get_int(b"FunctionType")? {
            0 => {
                let s = o.as_stream()?;
                let range = nums(doc, d, b"Range");
                let size: Vec<usize> = nums(doc, d, b"Size")
                    .iter()
                    .map(|v| (*v as usize).clamp(1, 1 << 16))
                    .collect();
                let bps = d.get_int(b"BitsPerSample").unwrap_or(8);
                if !matches!(bps, 1 | 2 | 4 | 8 | 12 | 16 | 24 | 32) || size.is_empty() {
                    return None;
                }
                let outputs = range.len() / 2;
                if outputs == 0 {
                    return None;
                }
                let count = size
                    .iter()
                    .try_fold(outputs, |acc, &s| acc.checked_mul(s))
                    .filter(|&c| c <= MAX_SAMPLES)?;
                let data = decode_stream(s).ok()?;
                let max = ((1u64 << bps) - 1) as f64;
                let mut samples = Vec::with_capacity(count);
                let mut bit = 0usize;
                for _ in 0..count {
                    let mut v: u64 = 0;
                    for _ in 0..bps {
                        let byte = data.get(bit / 8).copied().unwrap_or(0);
                        let b = (byte >> (7 - (bit % 8))) & 1;
                        v = (v << 1) | u64::from(b);
                        bit += 1;
                    }
                    samples.push(v as f64 / max);
                }
                let mut encode = nums(doc, d, b"Encode");
                if encode.len() < size.len() * 2 {
                    encode = size
                        .iter()
                        .flat_map(|&s| [0.0, (s.saturating_sub(1)) as f64])
                        .collect();
                }
                let mut decode = nums(doc, d, b"Decode");
                if decode.len() < outputs * 2 {
                    decode = range.clone();
                }
                Some(Function::Sampled {
                    domain,
                    range,
                    size,
                    encode,
                    decode,
                    samples,
                    outputs,
                })
            }
            2 => {
                let c0 = {
                    let v = nums(doc, d, b"C0");
                    if v.is_empty() { vec![0.0] } else { v }
                };
                let c1 = {
                    let v = nums(doc, d, b"C1");
                    if v.is_empty() { vec![1.0] } else { v }
                };
                Some(Function::Exponential {
                    domain,
                    c0,
                    c1,
                    n: doc
                        .resolve_key(d, b"N")
                        .and_then(Object::as_f64)
                        .unwrap_or(1.0),
                })
            }
            3 => {
                let functions: Vec<Function> = match doc.resolve_key(d, b"Functions") {
                    Some(Object::Array(a)) => a
                        .iter()
                        .take(256)
                        .filter_map(|f| Self::parse_depth(doc, f, depth + 1))
                        .collect(),
                    _ => Vec::new(),
                };
                if functions.is_empty() {
                    return None;
                }
                Some(Function::Stitching {
                    domain,
                    bounds: nums(doc, d, b"Bounds"),
                    encode: nums(doc, d, b"Encode"),
                    functions,
                })
            }
            4 => {
                let s = o.as_stream()?;
                let data = decode_stream(s).ok()?;
                Some(Function::PostScript {
                    domain,
                    range: nums(doc, d, b"Range"),
                    program: parse_ps(&data)?,
                })
            }
            _ => None,
        }
    }

    /// Evaluate. Inputs are clipped to the domain, outputs to the range.
    pub fn eval(&self, input: &[f64]) -> Vec<f64> {
        self.eval_depth(input, 0)
    }

    fn eval_depth(&self, input: &[f64], depth: usize) -> Vec<f64> {
        if depth > MAX_DEPTH {
            return Vec::new();
        }
        match self {
            Function::Array(fs) => fs
                .iter()
                .filter_map(|f| f.eval_depth(input, depth + 1).first().copied())
                .collect(),
            Function::Exponential { domain, c0, c1, n } => {
                let x = clip(input.first().copied().unwrap_or(0.0), domain, 0);
                let xn = x.powf(*n);
                let xn = if xn.is_finite() { xn } else { 0.0 };
                c0.iter()
                    .zip(c1.iter())
                    .map(|(a, b)| a + xn * (b - a))
                    .collect()
            }
            Function::Stitching {
                domain,
                functions,
                bounds,
                encode,
            } => {
                let x = clip(input.first().copied().unwrap_or(0.0), domain, 0);
                let d0 = domain.first().copied().unwrap_or(0.0);
                let d1 = domain.get(1).copied().unwrap_or(1.0);
                let k = bounds.iter().take_while(|&&b| x >= b).count();
                let k = k.min(functions.len().saturating_sub(1));
                let lo = if k == 0 {
                    d0
                } else {
                    bounds.get(k - 1).copied().unwrap_or(d0)
                };
                let hi = bounds.get(k).copied().unwrap_or(d1);
                let e0 = encode.get(2 * k).copied().unwrap_or(0.0);
                let e1 = encode.get(2 * k + 1).copied().unwrap_or(1.0);
                let t = interpolate(x, lo, hi, e0, e1);
                functions
                    .get(k)
                    .map(|f| f.eval_depth(&[t], depth + 1))
                    .unwrap_or_default()
            }
            Function::Sampled {
                domain,
                range,
                size,
                encode,
                decode,
                samples,
                outputs,
            } => {
                // Nearest sample in every dimension but the first, which is
                // interpolated linearly (smooth 1-input tint transforms).
                let mut index_lo = 0usize;
                let mut index_hi = 0usize;
                let mut frac = 0.0;
                let mut stride = *outputs;
                for (i, &sz) in size.iter().enumerate() {
                    let x = clip(input.get(i).copied().unwrap_or(0.0), domain, i);
                    let e = interpolate(
                        x,
                        domain.get(2 * i).copied().unwrap_or(0.0),
                        domain.get(2 * i + 1).copied().unwrap_or(1.0),
                        encode.get(2 * i).copied().unwrap_or(0.0),
                        encode.get(2 * i + 1).copied().unwrap_or((sz - 1) as f64),
                    )
                    .clamp(0.0, (sz - 1) as f64);
                    let (lo, hi) = if i == 0 {
                        frac = e - e.floor();
                        (e.floor() as usize, (e.ceil() as usize).min(sz - 1))
                    } else {
                        let r = e.round() as usize;
                        (r, r)
                    };
                    index_lo += lo * stride;
                    index_hi += hi * stride;
                    stride = stride.saturating_mul(sz);
                }
                (0..*outputs)
                    .map(|j| {
                        let a = samples.get(index_lo + j).copied().unwrap_or(0.0);
                        let b = samples.get(index_hi + j).copied().unwrap_or(a);
                        let s = a + (b - a) * frac;
                        let v = interpolate(
                            s,
                            0.0,
                            1.0,
                            decode.get(2 * j).copied().unwrap_or(0.0),
                            decode.get(2 * j + 1).copied().unwrap_or(1.0),
                        );
                        clip(v, range, j)
                    })
                    .collect()
            }
            Function::PostScript {
                domain,
                range,
                program,
            } => {
                let mut stack: Vec<f64> = input
                    .iter()
                    .enumerate()
                    .map(|(i, &x)| clip(x, domain, i))
                    .collect();
                run_ps(program, &mut stack);
                let n = range.len() / 2;
                let start = stack.len().saturating_sub(n);
                stack
                    .get(start..)
                    .unwrap_or_default()
                    .iter()
                    .enumerate()
                    .map(|(i, &v)| clip(v, range, i))
                    .collect()
            }
        }
    }
}

fn clip(x: f64, bounds: &[f64], i: usize) -> f64 {
    let x = if x.is_finite() { x } else { 0.0 };
    match (bounds.get(2 * i), bounds.get(2 * i + 1)) {
        (Some(&lo), Some(&hi)) if lo <= hi => x.clamp(lo, hi),
        _ => x,
    }
}

fn interpolate(x: f64, x0: f64, x1: f64, y0: f64, y1: f64) -> f64 {
    if (x1 - x0).abs() < f64::EPSILON {
        y0
    } else {
        y0 + (x - x0) * (y1 - y0) / (x1 - x0)
    }
}

/// Parse `{ ... }` into a flat instruction list.
pub fn parse_ps(data: &[u8]) -> Option<Vec<Ps>> {
    let mut p = Parser::content(data);
    // Expect the outer brace.
    loop {
        match p.lex.token().ok()?? {
            Token::Keyword(k) if k == b"{" => break,
            _ => continue,
        }
    }
    let mut out = Vec::new();
    parse_block(&mut p, &mut out, 0)?;
    Some(out)
}

fn parse_block(p: &mut Parser<'_>, out: &mut Vec<Ps>, depth: usize) -> Option<()> {
    if depth > 32 {
        return None;
    }
    loop {
        if out.len() > MAX_PS_OPS {
            return None;
        }
        match p.lex.token().ok()?? {
            Token::Keyword(k) if k == b"}" => return Some(()),
            Token::Keyword(k) if k == b"{" => {
                // A block: followed by `if`, or by another block and `ifelse`.
                let mut first = Vec::new();
                parse_block(p, &mut first, depth + 1)?;
                match p.lex.token().ok()?? {
                    Token::Keyword(k) if k == b"if" => {
                        out.push(Ps::If { len: first.len() });
                        out.extend(first);
                    }
                    Token::Keyword(k) if k == b"{" => {
                        let mut second = Vec::new();
                        parse_block(p, &mut second, depth + 1)?;
                        match p.lex.token().ok()?? {
                            Token::Keyword(k) if k == b"ifelse" => {
                                out.push(Ps::IfElse {
                                    then_len: first.len(),
                                    else_len: second.len(),
                                });
                                out.extend(first);
                                out.extend(second);
                            }
                            _ => return None,
                        }
                    }
                    _ => return None,
                }
            }
            Token::Keyword(k) => out.push(match k.as_slice() {
                b"true" => Ps::Bool(true),
                b"false" => Ps::Bool(false),
                _ => Ps::Op(k),
            }),
            Token::Obj(Object::Int(i)) => out.push(Ps::Num(i as f64)),
            Token::Obj(Object::Real(r)) => out.push(Ps::Num(r)),
            Token::Obj(Object::Bool(b)) => out.push(Ps::Bool(b)),
            _ => return None,
        }
    }
}

/// Booleans live on the numeric stack as 1.0 / 0.0.
fn run_ps(prog: &[Ps], stack: &mut Vec<f64>) {
    let mut steps = 0usize;
    exec(prog, stack, &mut steps);
}

fn exec(prog: &[Ps], st: &mut Vec<f64>, steps: &mut usize) {
    let mut i = 0usize;
    while let Some(ins) = prog.get(i) {
        *steps += 1;
        if *steps > MAX_PS_STEPS || st.len() > MAX_PS_STACK {
            return;
        }
        i += 1;
        match ins {
            Ps::Num(n) => st.push(*n),
            Ps::Bool(b) => st.push(if *b { 1.0 } else { 0.0 }),
            Ps::If { len } => {
                let c = st.pop().unwrap_or(0.0) != 0.0;
                let body = prog.get(i..i + len).unwrap_or_default();
                if c {
                    exec(body, st, steps);
                }
                i += len;
            }
            Ps::IfElse { then_len, else_len } => {
                let c = st.pop().unwrap_or(0.0) != 0.0;
                let t = prog.get(i..i + then_len).unwrap_or_default();
                let e = prog
                    .get(i + then_len..i + then_len + else_len)
                    .unwrap_or_default();
                exec(if c { t } else { e }, st, steps);
                i += then_len + else_len;
            }
            Ps::Op(op) => ps_op(op, st),
        }
    }
}

fn ps_op(op: &[u8], st: &mut Vec<f64>) {
    let pop = |st: &mut Vec<f64>| st.pop().unwrap_or(0.0);
    let b = |v: bool| if v { 1.0 } else { 0.0 };
    match op {
        b"add" => {
            let (y, x) = (pop(st), pop(st));
            st.push(x + y);
        }
        b"sub" => {
            let (y, x) = (pop(st), pop(st));
            st.push(x - y);
        }
        b"mul" => {
            let (y, x) = (pop(st), pop(st));
            st.push(x * y);
        }
        b"div" => {
            let (y, x) = (pop(st), pop(st));
            st.push(if y == 0.0 { 0.0 } else { x / y });
        }
        b"idiv" => {
            let (y, x) = (pop(st) as i64, pop(st) as i64);
            st.push(x.checked_div(y).unwrap_or(0) as f64);
        }
        b"mod" => {
            let (y, x) = (pop(st) as i64, pop(st) as i64);
            st.push(x.checked_rem(y).unwrap_or(0) as f64);
        }
        b"neg" => {
            let x = pop(st);
            st.push(-x);
        }
        b"abs" => {
            let x = pop(st);
            st.push(x.abs());
        }
        b"ceiling" => {
            let x = pop(st);
            st.push(x.ceil());
        }
        b"floor" => {
            let x = pop(st);
            st.push(x.floor());
        }
        b"round" => {
            let x = pop(st);
            st.push((x + 0.5).floor());
        }
        b"truncate" | b"cvi" => {
            let x = pop(st);
            st.push(x.trunc());
        }
        b"cvr" => {}
        b"sqrt" => {
            let x = pop(st);
            st.push(x.max(0.0).sqrt());
        }
        b"sin" => {
            let x = pop(st);
            st.push(x.to_radians().sin());
        }
        b"cos" => {
            let x = pop(st);
            st.push(x.to_radians().cos());
        }
        b"atan" => {
            let (den, num) = (pop(st), pop(st));
            let a = num.atan2(den).to_degrees();
            st.push(if a < 0.0 { a + 360.0 } else { a });
        }
        b"exp" => {
            let (e, base) = (pop(st), pop(st));
            let v = base.powf(e);
            st.push(if v.is_finite() { v } else { 0.0 });
        }
        b"ln" => {
            let x = pop(st);
            st.push(if x > 0.0 { x.ln() } else { 0.0 });
        }
        b"log" => {
            let x = pop(st);
            st.push(if x > 0.0 { x.log10() } else { 0.0 });
        }
        b"eq" => {
            let (y, x) = (pop(st), pop(st));
            st.push(b(x == y));
        }
        b"ne" => {
            let (y, x) = (pop(st), pop(st));
            st.push(b(x != y));
        }
        b"gt" => {
            let (y, x) = (pop(st), pop(st));
            st.push(b(x > y));
        }
        b"ge" => {
            let (y, x) = (pop(st), pop(st));
            st.push(b(x >= y));
        }
        b"lt" => {
            let (y, x) = (pop(st), pop(st));
            st.push(b(x < y));
        }
        b"le" => {
            let (y, x) = (pop(st), pop(st));
            st.push(b(x <= y));
        }
        b"and" => {
            let (y, x) = (pop(st) as i64, pop(st) as i64);
            st.push((x & y) as f64);
        }
        b"or" => {
            let (y, x) = (pop(st) as i64, pop(st) as i64);
            st.push((x | y) as f64);
        }
        b"xor" => {
            let (y, x) = (pop(st) as i64, pop(st) as i64);
            st.push((x ^ y) as f64);
        }
        b"not" => {
            let x = pop(st);
            st.push(if x == 0.0 || x == 1.0 {
                b(x == 0.0)
            } else {
                !(x as i64) as f64
            });
        }
        b"bitshift" => {
            let (s, x) = (pop(st) as i64, pop(st) as i64);
            let s = s.clamp(-63, 63);
            st.push(if s >= 0 { x << s } else { x >> -s } as f64);
        }
        b"pop" => {
            st.pop();
        }
        b"dup" => {
            let x = st.last().copied().unwrap_or(0.0);
            st.push(x);
        }
        b"exch" => {
            let (y, x) = (pop(st), pop(st));
            st.push(y);
            st.push(x);
        }
        b"copy" => {
            let n = (pop(st) as usize).min(st.len()).min(MAX_PS_STACK);
            let start = st.len() - n;
            let copy: Vec<f64> = st.get(start..).unwrap_or_default().to_vec();
            st.extend(copy);
        }
        b"index" => {
            let n = pop(st) as usize;
            let v = st
                .len()
                .checked_sub(n + 1)
                .and_then(|i| st.get(i))
                .copied()
                .unwrap_or(0.0);
            st.push(v);
        }
        b"roll" => {
            let (j, n) = (pop(st) as i64, pop(st) as i64);
            let n = (n.max(0) as usize).min(st.len());
            if n > 0 {
                let start = st.len() - n;
                if let Some(s) = st.get_mut(start..) {
                    let k = j.rem_euclid(n as i64) as usize;
                    s.rotate_right(k);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postscript_calculator() {
        let prog = parse_ps(b"{ dup 0.5 gt { 1 exch sub } if 2 mul }").unwrap();
        let mut s = vec![0.75];
        run_ps(&prog, &mut s);
        assert!((s[0] - 0.5).abs() < 1e-9);
        let prog = parse_ps(b"{ 0 eq { 1 } { 2 } ifelse 3 1 roll }").unwrap();
        let mut s = vec![7.0, 8.0, 0.0];
        run_ps(&prog, &mut s);
        assert_eq!(s, vec![1.0, 7.0, 8.0]);
    }

    #[test]
    fn exponential_and_stitching() {
        let e = Function::Exponential {
            domain: vec![0.0, 1.0],
            c0: vec![1.0, 0.0],
            c1: vec![0.0, 1.0],
            n: 1.0,
        };
        assert_eq!(e.eval(&[0.25]), vec![0.75, 0.25]);
        let s = Function::Stitching {
            domain: vec![0.0, 1.0],
            functions: vec![e.clone(), e],
            bounds: vec![0.5],
            encode: vec![0.0, 1.0, 1.0, 0.0],
        };
        // Second half runs the function backwards.
        assert_eq!(s.eval(&[1.0]), vec![1.0, 0.0]);
    }

    #[test]
    fn hostile_programs_are_bounded() {
        let deep = "{".repeat(100);
        assert!(parse_ps(deep.as_bytes()).is_none());
        let prog = parse_ps(b"{ 1 1000 copy 1000 copy 1000 copy }").unwrap();
        let mut s = vec![];
        run_ps(&prog, &mut s);
        assert!(s.len() <= 2 * MAX_PS_STACK + 2);
    }
}
