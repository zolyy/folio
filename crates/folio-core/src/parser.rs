//! Builds objects from tokens: arrays, dictionaries, references, and indirect
//! objects with their streams (ISO 32000-2 §7.3.8, §7.3.10).

use crate::error::{Error, Result};
use crate::lexer::{Lexer, Token, is_ws};
use crate::object::{Dict, ObjRef, Object, Stream};

/// Maximum nesting of arrays and dictionaries. Hostile files nest thousands deep.
pub const MAX_DEPTH: usize = 100;

pub struct Parser<'a> {
    pub lex: Lexer<'a>,
    /// Whether `n g R` is read as a reference. Off for content streams.
    allow_refs: bool,
}

impl<'a> Parser<'a> {
    pub fn new(data: &'a [u8], pos: usize) -> Self {
        Parser {
            lex: Lexer::new(data, pos),
            allow_refs: true,
        }
    }

    /// A parser for content streams and CMaps, where `R` is never a reference.
    pub fn content(data: &'a [u8]) -> Self {
        Parser {
            lex: Lexer::new(data, 0),
            allow_refs: false,
        }
    }

    /// Parse one direct object.
    pub fn object(&mut self) -> Result<Object> {
        let start = self.lex.pos;
        match self.lex.token()? {
            Some(t) => self.from_token(t, 0, start),
            None => Err(Error::parse(
                start,
                "unexpected end of data, expected an object",
            )),
        }
    }

    /// Finish an object whose first token has already been read.
    pub fn from_token(&mut self, tok: Token, depth: usize, start: usize) -> Result<Object> {
        if depth > MAX_DEPTH {
            return Err(Error::Limit(format!(
                "objects nested more than {MAX_DEPTH} levels deep at byte {start}"
            )));
        }
        match tok {
            Token::Obj(Object::Int(n)) if self.allow_refs && n >= 0 => Ok(self.maybe_ref(n)),
            Token::Obj(o) => Ok(o),
            Token::ArrayStart => {
                let mut items = Vec::new();
                loop {
                    let at = self.lex.pos;
                    match self.lex.token()? {
                        None | Some(Token::ArrayEnd) => break,
                        // Stray keywords inside arrays are skipped, not fatal.
                        Some(Token::Keyword(_)) => continue,
                        Some(Token::DictEnd) => continue,
                        Some(t) => items.push(self.from_token(t, depth + 1, at)?),
                    }
                }
                Ok(Object::Array(items))
            }
            Token::DictStart => {
                let mut dict = Dict::new();
                loop {
                    let at = self.lex.pos;
                    match self.lex.token()? {
                        None | Some(Token::DictEnd) => break,
                        Some(Token::Obj(Object::Name(key))) => {
                            let vat = self.lex.pos;
                            match self.lex.token()? {
                                None | Some(Token::DictEnd) => {
                                    dict.set(key, Object::Null);
                                    break;
                                }
                                Some(Token::Keyword(k)) if k == b"endobj" || k == b"stream" => {
                                    // Missing `>>`: rewind so the caller sees the keyword.
                                    self.lex.pos = vat;
                                    dict.set(key, Object::Null);
                                    break;
                                }
                                Some(t) => {
                                    let v = self.from_token(t, depth + 1, vat)?;
                                    dict.set(key, v);
                                }
                            }
                        }
                        Some(Token::Keyword(k)) if k == b"endobj" || k == b"stream" => {
                            self.lex.pos = at;
                            break;
                        }
                        // Junk where a key belongs is skipped.
                        Some(_) => continue,
                    }
                }
                Ok(Object::Dict(dict))
            }
            Token::Keyword(k) => Err(Error::parse(
                start,
                format!(
                    "expected an object, found keyword `{}`",
                    String::from_utf8_lossy(&k)
                ),
            )),
            Token::ArrayEnd | Token::DictEnd => Err(Error::parse(start, "unexpected `]` or `>>`")),
        }
    }

    fn maybe_ref(&mut self, num: i64) -> Object {
        let save = self.lex.pos;
        let ok = (|| -> Option<ObjRef> {
            let Ok(Some(Token::Obj(Object::Int(generation)))) = self.lex.token() else {
                return None;
            };
            let Ok(Some(Token::Keyword(k))) = self.lex.token() else {
                return None;
            };
            if k != b"R" || !(0..=i64::from(u16::MAX)).contains(&generation) {
                return None;
            }
            let num = u32::try_from(num).ok()?;
            Some(ObjRef::new(num, generation as u16))
        })();
        match ok {
            Some(r) => Object::Ref(r),
            None => {
                self.lex.pos = save;
                Object::Int(num)
            }
        }
    }
}

/// Find `needle` in `hay` starting at `from`.
pub fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= hay.len() {
        return None;
    }
    hay.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// Find the last `needle` in `hay`.
pub fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).rposition(|w| w == needle)
}

/// Parse `num gen obj ... endobj` at `offset`, including stream data.
pub fn parse_indirect(data: &[u8], offset: usize) -> Result<(ObjRef, Object)> {
    let mut p = Parser::new(data, offset);
    let num = match p.lex.token()? {
        Some(Token::Obj(Object::Int(n))) => {
            u32::try_from(n).map_err(|_| Error::parse(offset, "object number out of range"))?
        }
        _ => return Err(Error::parse(offset, "expected an object number")),
    };
    let generation = match p.lex.token()? {
        Some(Token::Obj(Object::Int(g))) => u16::try_from(g).unwrap_or(0),
        _ => return Err(Error::parse(offset, "expected a generation number")),
    };
    match p.lex.token()? {
        Some(Token::Keyword(k)) if k == b"obj" => {}
        _ => return Err(Error::parse(offset, "expected `obj`")),
    }
    let obj = p.object()?;
    let id = ObjRef::new(num, generation);

    let Object::Dict(dict) = obj else {
        return Ok((id, obj));
    };
    let after_dict = p.lex.pos;
    match p.lex.token() {
        Ok(Some(Token::Keyword(k))) if k == b"stream" => {}
        _ => {
            return Ok((id, Object::Dict(dict)));
        }
    }
    let _ = after_dict;
    // `stream` is followed by CRLF or LF (a lone CR is tolerated).
    let mut start = p.lex.pos;
    if data.get(start) == Some(&b'\r') {
        start += 1;
    }
    if data.get(start) == Some(&b'\n') {
        start += 1;
    }
    let declared = dict
        .get_int(b"Length")
        .and_then(|l| usize::try_from(l).ok());
    let mut end = None;
    if let Some(len) = declared
        && let Some(e) = start.checked_add(len)
        && e <= data.len()
    {
        let mut q = e;
        while data.get(q).copied().is_some_and(is_ws) {
            q += 1;
        }
        if data
            .get(q..)
            .is_some_and(|rest| rest.starts_with(b"endstream"))
        {
            end = Some(e);
        }
    }
    let end = match end {
        Some(e) => e,
        None => {
            // Length missing, indirect or wrong: search for `endstream`.
            let e = find(data, b"endstream", start)
                .ok_or_else(|| Error::parse(start, "stream has no `endstream`"))?;
            let mut e2 = e;
            if e2 > start && data.get(e2 - 1) == Some(&b'\n') {
                e2 -= 1;
            }
            if e2 > start && data.get(e2 - 1) == Some(&b'\r') {
                e2 -= 1;
            }
            e2
        }
    };
    let bytes = data.get(start..end).unwrap_or_default().to_vec();
    Ok((id, Object::Stream(Stream { dict, data: bytes })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dict_with_refs_and_nested_arrays() {
        let mut p = Parser::new(b"<< /Kids [3 0 R 4 0 R] /Count 2 /N [1 [2]] >>", 0);
        let o = p.object().unwrap();
        let d = o.as_dict().unwrap();
        assert_eq!(
            d.get(b"Kids").unwrap().as_array().unwrap()[1],
            Object::Ref(ObjRef::new(4, 0))
        );
        assert_eq!(d.get_int(b"Count"), Some(2));
    }

    #[test]
    fn two_ints_are_not_a_ref() {
        let mut p = Parser::new(b"[1 2 3]", 0);
        assert_eq!(
            p.object().unwrap(),
            Object::Array(vec![Object::Int(1), Object::Int(2), Object::Int(3)])
        );
    }

    #[test]
    fn deep_nesting_hits_the_limit_instead_of_the_stack() {
        let src = "[".repeat(100_000);
        let mut p = Parser::new(src.as_bytes(), 0);
        assert!(matches!(p.object(), Err(Error::Limit(_))));
    }

    #[test]
    fn stream_with_wrong_length_is_recovered() {
        let src = b"5 0 obj << /Length 999 >> stream\nhello\nendstream endobj";
        let (id, o) = parse_indirect(src, 0).unwrap();
        assert_eq!(id, ObjRef::new(5, 0));
        assert_eq!(o.as_stream().unwrap().data, b"hello");
    }

    #[test]
    fn stream_with_correct_length_keeps_trailing_newlines() {
        let src = b"1 0 obj << /Length 3 >> stream\r\na\n\n\nendstream endobj";
        let (_, o) = parse_indirect(src, 0).unwrap();
        assert_eq!(o.as_stream().unwrap().data, b"a\n\n");
    }
}
