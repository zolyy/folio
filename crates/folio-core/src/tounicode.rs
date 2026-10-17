//! ToUnicode CMaps (ISO 32000-2 §9.10.3): character codes to Unicode text.
//! Shared by text extraction, search and selection.

use std::collections::HashMap;

use crate::lexer::Token;
use crate::object::Object;
use crate::parser::Parser;

/// ToUnicode CMap: `bfchar` and `bfrange` sections (§9.10.3).
pub fn parse(data: &[u8]) -> HashMap<u32, String> {
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
