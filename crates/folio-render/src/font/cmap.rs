//! CMaps for composite fonts (ISO 32000-2 §9.7.5): splitting a string into
//! character codes by the codespace ranges, and mapping codes to CIDs.

use folio_core::lexer::Token;
use folio_core::object::Object;
use folio_core::parser::Parser;

#[derive(Clone, Debug, Default)]
pub struct CMap {
    /// (low, high) byte sequences of equal length.
    codespace: Vec<(Vec<u8>, Vec<u8>)>,
    /// (low code, high code, first CID, code length).
    ranges: Vec<(u32, u32, u32, usize)>,
    /// Identity-H / Identity-V: 2-byte codes, CID = code.
    pub identity: bool,
    /// Written vertically (`-V`); drawn horizontally until vertical
    /// writing is supported.
    pub vertical: bool,
}

impl CMap {
    pub fn identity(vertical: bool) -> Self {
        CMap {
            identity: true,
            vertical,
            ..CMap::default()
        }
    }

    pub fn parse(data: &[u8]) -> CMap {
        let mut cm = CMap::default();
        let mut p = Parser::content(data);
        let mut mode = 0u8; // 1 codespace, 2 cidrange, 3 cidchar
        let mut items: Vec<Object> = Vec::new();
        let code = |b: &[u8]| b.iter().take(4).fold(0u32, |a, &x| (a << 8) | u32::from(x));
        loop {
            let at = p.lex.pos;
            let Ok(Some(tok)) = p.lex.token() else { break };
            match tok {
                Token::Keyword(k) => match k.as_slice() {
                    b"begincodespacerange" => {
                        mode = 1;
                        items.clear();
                    }
                    b"begincidrange" => {
                        mode = 2;
                        items.clear();
                    }
                    b"begincidchar" => {
                        mode = 3;
                        items.clear();
                    }
                    b"endcodespacerange" => {
                        for pair in items.chunks(2) {
                            if let (Some(Object::Str(lo)), Some(Object::Str(hi))) =
                                (pair.first(), pair.get(1))
                                && lo.len() == hi.len()
                                && (1..=4).contains(&lo.len())
                                && cm.codespace.len() < 256
                            {
                                cm.codespace.push((lo.clone(), hi.clone()));
                            }
                        }
                        mode = 0;
                    }
                    b"endcidrange" => {
                        for t in items.chunks(3) {
                            if let (Some(Object::Str(lo)), Some(Object::Str(hi)), Some(cid)) =
                                (t.first(), t.get(1), t.get(2).and_then(Object::as_int))
                                && cm.ranges.len() < 100_000
                            {
                                cm.ranges.push((
                                    code(lo),
                                    code(hi),
                                    u32::try_from(cid).unwrap_or(0),
                                    lo.len(),
                                ));
                            }
                        }
                        mode = 0;
                    }
                    b"endcidchar" => {
                        for t in items.chunks(2) {
                            if let (Some(Object::Str(c)), Some(cid)) =
                                (t.first(), t.get(1).and_then(Object::as_int))
                                && cm.ranges.len() < 100_000
                            {
                                let v = code(c);
                                cm.ranges
                                    .push((v, v, u32::try_from(cid).unwrap_or(0), c.len()));
                            }
                        }
                        mode = 0;
                    }
                    b"usecmap" => {
                        // Only the identity CMaps are known without CMap resources.
                        if let Some(Object::Name(n)) = items.last()
                            && n.starts_with(b"Identity")
                        {
                            cm.identity = cm.ranges.is_empty();
                        }
                    }
                    _ => {}
                },
                t => {
                    if let Ok(o) = p.from_token(t, 0, at)
                        && (mode != 0 || matches!(o, Object::Name(_)))
                        && items.len() < 300_000
                    {
                        items.push(o);
                    }
                }
            }
        }
        if cm.codespace.is_empty() {
            cm.codespace.push((vec![0, 0], vec![0xFF, 0xFF]));
        }
        cm
    }

    /// Split a string into (code, length) pairs.
    pub fn codes(&self, bytes: &[u8]) -> Vec<(u32, usize)> {
        let mut out = Vec::with_capacity(bytes.len() / 2 + 1);
        let mut i = 0usize;
        while i < bytes.len() {
            let len = if self.identity {
                2
            } else {
                (1..=4)
                    .find(|&n| {
                        bytes.get(i..i + n).is_some_and(|prefix| {
                            self.codespace.iter().any(|(lo, hi)| {
                                lo.len() == n
                                    && prefix
                                        .iter()
                                        .zip(lo.iter().zip(hi.iter()))
                                        .all(|(b, (l, h))| b >= l && b <= h)
                            })
                        })
                    })
                    .unwrap_or_else(|| {
                        self.codespace
                            .iter()
                            .map(|(lo, _)| lo.len())
                            .min()
                            .unwrap_or(1)
                    })
            };
            let end = (i + len).min(bytes.len());
            let code = bytes
                .get(i..end)
                .unwrap_or_default()
                .iter()
                .fold(0u32, |a, &x| (a << 8) | u32::from(x));
            out.push((code, len));
            i = end.max(i + 1);
        }
        out
    }

    pub fn cid(&self, code: u32, len: usize) -> u32 {
        if self.identity {
            return code;
        }
        self.ranges
            .iter()
            .rev()
            .find(|(lo, hi, _, l)| *l == len && code >= *lo && code <= *hi)
            .map(|(lo, _, cid, _)| cid.saturating_add(code - lo))
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_width_codespace() {
        let cm = CMap::parse(
            b"2 begincodespacerange <00> <80> <8140> <9FFC> endcodespacerange \
              1 begincidrange <20> <7E> 1 endcidrange \
              1 begincidchar <8140> 633 endcidchar",
        );
        let codes = cm.codes(&[0x41, 0x81, 0x40, 0x42]);
        assert_eq!(codes, vec![(0x41, 1), (0x8140, 2), (0x42, 1)]);
        assert_eq!(cm.cid(0x41, 1), 34);
        assert_eq!(cm.cid(0x8140, 2), 633);
    }
}
