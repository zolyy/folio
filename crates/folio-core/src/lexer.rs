//! Tokeniser for PDF syntax (ISO 32000-2 §7.2), shared by file parsing,
//! content streams and CMaps. Lenient: odd bytes become keywords, never panics.

use crate::error::{Error, Result};
use crate::object::Object;

/// PDF white-space characters (Table 1).
pub fn is_ws(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}

/// PDF delimiter characters (Table 2).
pub fn is_delim(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

pub fn is_regular(b: u8) -> bool {
    !is_ws(b) && !is_delim(b)
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// A complete direct primitive: number, name, string, boolean or null.
    Obj(Object),
    /// Anything else made of regular characters: `obj`, `R`, `Tj`, `stream`...
    Keyword(Vec<u8>),
    ArrayStart,
    ArrayEnd,
    DictStart,
    DictEnd,
}

pub struct Lexer<'a> {
    data: &'a [u8],
    pub pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(data: &'a [u8], pos: usize) -> Self {
        Lexer { data, pos }
    }

    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    pub fn at_end(&self) -> bool {
        self.pos >= self.data.len()
    }

    pub fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    /// Skip white space and comments.
    pub fn skip_ws(&mut self) {
        loop {
            while self.peek().is_some_and(is_ws) {
                self.pos += 1;
            }
            if self.peek() == Some(b'%') {
                while let Some(b) = self.peek() {
                    if b == b'\n' || b == b'\r' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                return;
            }
        }
    }

    /// The next token, or `None` at end of input.
    pub fn token(&mut self) -> Result<Option<Token>> {
        self.skip_ws();
        let Some(b) = self.peek() else {
            return Ok(None);
        };
        let next = self.data.get(self.pos + 1).copied();
        let tok = match b {
            b'[' => {
                self.pos += 1;
                Token::ArrayStart
            }
            b']' => {
                self.pos += 1;
                Token::ArrayEnd
            }
            b'<' if next == Some(b'<') => {
                self.pos += 2;
                Token::DictStart
            }
            b'>' if next == Some(b'>') => {
                self.pos += 2;
                Token::DictEnd
            }
            b'<' => Token::Obj(Object::Str(self.hex_string()?)),
            b'(' => Token::Obj(Object::Str(self.literal_string()?)),
            b'/' => Token::Obj(Object::Name(self.name())),
            b'{' | b'}' | b')' | b'>' => {
                self.pos += 1;
                Token::Keyword(vec![b])
            }
            _ => {
                let start = self.pos;
                while self.peek().is_some_and(is_regular) {
                    self.pos += 1;
                }
                let word = self.data.get(start..self.pos).unwrap_or_default();
                classify(word)
            }
        };
        Ok(Some(tok))
    }

    fn literal_string(&mut self) -> Result<Vec<u8>> {
        let start = self.pos;
        self.pos += 1; // '('
        let mut depth = 1usize;
        let mut out = Vec::new();
        loop {
            let Some(b) = self.peek() else {
                return Err(Error::parse(start, "unterminated literal string"));
            };
            self.pos += 1;
            match b {
                b'\\' => {
                    let Some(e) = self.peek() else { continue };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'\r' => {
                            if self.peek() == Some(b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        b'0'..=b'7' => {
                            let mut v = u32::from(e - b'0');
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(d @ b'0'..=b'7') => {
                                        v = v * 8 + u32::from(d - b'0');
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push((v & 0xff) as u8);
                        }
                        other => out.push(other),
                    }
                }
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(out);
                    }
                    out.push(b);
                }
                _ => out.push(b),
            }
        }
    }

    fn hex_string(&mut self) -> Result<Vec<u8>> {
        let start = self.pos;
        self.pos += 1; // '<'
        let mut out = Vec::new();
        let mut high: Option<u8> = None;
        loop {
            let Some(b) = self.peek() else {
                return Err(Error::parse(start, "unterminated hex string"));
            };
            self.pos += 1;
            if b == b'>' {
                break;
            }
            let Some(v) = hex_val(b) else { continue };
            match high.take() {
                Some(h) => out.push(h << 4 | v),
                None => high = Some(v),
            }
        }
        if let Some(h) = high {
            out.push(h << 4);
        }
        Ok(out)
    }

    fn name(&mut self) -> Vec<u8> {
        self.pos += 1; // '/'
        let mut out = Vec::new();
        while let Some(b) = self.peek() {
            if !is_regular(b) {
                break;
            }
            self.pos += 1;
            if b == b'#' {
                let h = self.data.get(self.pos).copied().and_then(hex_val);
                let l = self.data.get(self.pos + 1).copied().and_then(hex_val);
                if let (Some(h), Some(l)) = (h, l) {
                    out.push(h << 4 | l);
                    self.pos += 2;
                    continue;
                }
            }
            out.push(b);
        }
        out
    }

    /// After a `BI ... ID` inline-image header, skip the image data up to and
    /// including the `EI` keyword. Returns false if no `EI` was found.
    pub fn skip_inline_image_data(&mut self) -> bool {
        // One white-space byte separates ID from the data.
        self.pos += 1;
        let mut i = self.pos;
        while i + 1 < self.data.len() {
            let before_ok = i == 0 || self.data.get(i - 1).copied().is_some_and(is_ws);
            if before_ok
                && self.data.get(i) == Some(&b'E')
                && self.data.get(i + 1) == Some(&b'I')
                && self.data.get(i + 2).is_none_or(|&b| !is_regular(b))
            {
                self.pos = i + 2;
                return true;
            }
            i += 1;
        }
        self.pos = self.data.len();
        false
    }
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn classify(word: &[u8]) -> Token {
    match word {
        b"true" => return Token::Obj(Object::Bool(true)),
        b"false" => return Token::Obj(Object::Bool(false)),
        b"null" => return Token::Obj(Object::Null),
        _ => {}
    }
    if let Some(n) = parse_number(word) {
        return Token::Obj(n);
    }
    Token::Keyword(word.to_vec())
}

/// Numbers per §7.3.3, leniently: `+5`, `-.5`, `4.`, and stray extra signs
/// (`--5`, written by some producers) are accepted.
fn parse_number(word: &[u8]) -> Option<Object> {
    if word.is_empty()
        || !word
            .iter()
            .all(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.'))
        || !word.iter().any(u8::is_ascii_digit)
    {
        return None;
    }
    let negative = word.first() == Some(&b'-');
    let body: Vec<u8> = word
        .iter()
        .copied()
        .skip_while(|b| matches!(b, b'+' | b'-'))
        .filter(|b| !matches!(b, b'+' | b'-'))
        .collect();
    let text = std::str::from_utf8(&body).ok()?;
    if body.contains(&b'.') {
        // Keep only the first decimal point.
        let mut seen = false;
        let cleaned: String = text
            .chars()
            .filter(|&c| {
                if c == '.' {
                    if seen {
                        return false;
                    }
                    seen = true;
                }
                true
            })
            .collect();
        let v: f64 = if cleaned == "." {
            0.0
        } else {
            cleaned.parse().ok()?
        };
        Some(Object::Real(if negative { -v } else { v }))
    } else {
        match text.parse::<i64>() {
            Ok(v) => Some(Object::Int(if negative { -v } else { v })),
            Err(_) => {
                let v: f64 = text.parse().ok()?;
                Some(Object::Real(if negative { -v } else { v }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(src: &[u8]) -> Vec<Token> {
        let mut l = Lexer::new(src, 0);
        let mut v = Vec::new();
        while let Some(t) = l.token().unwrap() {
            v.push(t);
        }
        v
    }

    #[test]
    fn numbers() {
        assert_eq!(
            tokens(b"12 -7 +3 4. -.5 --2 0.25"),
            vec![
                Token::Obj(Object::Int(12)),
                Token::Obj(Object::Int(-7)),
                Token::Obj(Object::Int(3)),
                Token::Obj(Object::Real(4.0)),
                Token::Obj(Object::Real(-0.5)),
                Token::Obj(Object::Int(-2)),
                Token::Obj(Object::Real(0.25)),
            ]
        );
    }

    #[test]
    fn literal_string_escapes() {
        assert_eq!(
            tokens(b"(a\\(b\\)c\\n\\101(x)\\\nz)"),
            vec![Token::Obj(Object::Str(b"a(b)c\nA(x)z".to_vec()))]
        );
    }

    #[test]
    fn hex_string_odd_length_and_spaces() {
        assert_eq!(
            tokens(b"<48 65 6c6C 6>"),
            vec![Token::Obj(Object::Str(vec![0x48, 0x65, 0x6c, 0x6c, 0x60]))]
        );
    }

    #[test]
    fn names_with_escapes() {
        assert_eq!(
            tokens(b"/A#20B/Type"),
            vec![
                Token::Obj(Object::Name(b"A B".to_vec())),
                Token::Obj(Object::Name(b"Type".to_vec())),
            ]
        );
    }

    #[test]
    fn comments_and_keywords() {
        assert_eq!(
            tokens(b"% hi\nBT ET"),
            vec![
                Token::Keyword(b"BT".to_vec()),
                Token::Keyword(b"ET".to_vec())
            ]
        );
    }

    #[test]
    fn unterminated_string_is_an_error_not_a_panic() {
        let mut l = Lexer::new(b"(abc", 0);
        assert!(l.token().is_err());
    }
}
