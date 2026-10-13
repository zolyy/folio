//! Text strings (ISO 32000-2 §7.9.2): UTF-16BE with BOM, UTF-8 with BOM, or
//! PDFDocEncoding.

/// Decode a PDF text string (document title, bookmark, field value...).
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest
            .chunks(2)
            .map(|c| {
                let hi = c.first().copied().unwrap_or(0);
                let lo = c.get(1).copied().unwrap_or(0);
                u16::from_be_bytes([hi, lo])
            })
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    bytes.iter().map(|&b| pdf_doc_char(b)).collect()
}

/// Encode a text string: plain bytes when the text is ASCII, otherwise
/// UTF-16BE with a byte-order mark (readable by every PDF reader).
pub fn encode_text(s: &str) -> Vec<u8> {
    if s.is_ascii() {
        return s.as_bytes().to_vec();
    }
    let mut out = vec![0xFE, 0xFF];
    for u in s.encode_utf16() {
        out.extend_from_slice(&u.to_be_bytes());
    }
    out
}

/// PDFDocEncoding (Annex D). Matches Latin-1 except 0x18–0x1F and 0x80–0x9F.
fn pdf_doc_char(b: u8) -> char {
    const LOW: [char; 8] = ['˘', 'ˇ', 'ˆ', '˙', '˝', '˛', '˚', '˜'];
    const HIGH: [char; 32] = [
        '•', '†', '‡', '…', '—', '–', 'ƒ', '⁄', '‹', '›', '−', '‰', '„', '“', '”', '‘', '’', '‚',
        '™', 'ﬁ', 'ﬂ', 'Ł', 'Œ', 'Š', 'Ÿ', 'Ž', 'ı', 'ł', 'œ', 'š', 'ž', '\u{FFFD}',
    ];
    match b {
        0x18..=0x1F => LOW
            .get(usize::from(b - 0x18))
            .copied()
            .unwrap_or('\u{FFFD}'),
        0x80..=0x9F => HIGH
            .get(usize::from(b - 0x80))
            .copied()
            .unwrap_or('\u{FFFD}'),
        0xA0 => '€',
        _ => char::from(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_roundtrip() {
        let s = "Proiect înțeles — Árvíztűrő";
        assert_eq!(decode_text(&encode_text(s)), s);
    }

    #[test]
    fn ascii_stays_plain() {
        assert_eq!(encode_text("Title"), b"Title");
        assert_eq!(decode_text(b"Title"), "Title");
    }

    #[test]
    fn pdfdoc_specials() {
        assert_eq!(decode_text(&[0x84, 0xA0]), "—€");
    }
}
