//! Character encodings for simple fonts (ISO 32000-2 Annex D) and glyph
//! names ↔ Unicode for the Latin repertoire PDFs use most. Written from the
//! specification's tables; no font or glyph data is included.

/// Named base encodings of simple fonts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseEncoding {
    Standard,
    WinAnsi,
    MacRoman,
}

impl BaseEncoding {
    pub fn from_name(n: &[u8]) -> Option<Self> {
        match n {
            b"StandardEncoding" => Some(Self::Standard),
            b"WinAnsiEncoding" => Some(Self::WinAnsi),
            b"MacRomanEncoding" => Some(Self::MacRoman),
            _ => None,
        }
    }

    /// The glyph name a code maps to.
    pub fn glyph_name(self, code: u8) -> Option<&'static str> {
        match self {
            Self::Standard => standard_name(code),
            Self::WinAnsi => char_name(win_ansi_char(code)?),
            Self::MacRoman => char_name(mac_roman_char(code)?),
        }
    }
}

/// WinAnsiEncoding (Windows code page 1252 as used by PDF).
pub fn win_ansi_char(code: u8) -> Option<char> {
    const HIGH: [char; 32] = [
        '€', '\0', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\0', 'Ž', '\0', '\0',
        '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\0', 'ž', 'Ÿ',
    ];
    match code {
        0x20..=0x7E | 0xA0..=0xFF => Some(char::from(code)),
        0x80..=0x9F => HIGH
            .get(usize::from(code - 0x80))
            .copied()
            .filter(|c| *c != '\0'),
        _ => None,
    }
}

/// MacRomanEncoding.
pub fn mac_roman_char(code: u8) -> Option<char> {
    const HIGH: [char; 128] = [
        'Ä', 'Å', 'Ç', 'É', 'Ñ', 'Ö', 'Ü', 'á', 'à', 'â', 'ä', 'ã', 'å', 'ç', 'é', 'è', 'ê', 'ë',
        'í', 'ì', 'î', 'ï', 'ñ', 'ó', 'ò', 'ô', 'ö', 'õ', 'ú', 'ù', 'û', 'ü', '†', '°', '¢', '£',
        '§', '•', '¶', 'ß', '®', '©', '™', '´', '¨', '≠', 'Æ', 'Ø', '∞', '±', '≤', '≥', '¥', 'µ',
        '∂', '∑', '∏', 'π', '∫', 'ª', 'º', 'Ω', 'æ', 'ø', '¿', '¡', '¬', '√', 'ƒ', '≈', '∆', '«',
        '»', '…', '\u{a0}', 'À', 'Ã', 'Õ', 'Œ', 'œ', '–', '—', '“', '”', '‘', '’', '÷', '◊', 'ÿ',
        'Ÿ', '⁄', '¤', '‹', '›', 'ﬁ', 'ﬂ', '‡', '·', '‚', '„', '‰', 'Â', 'Ê', 'Á', 'Ë', 'È', 'Í',
        'Î', 'Ï', 'Ì', 'Ó', 'Ô', '\u{f8ff}', 'Ò', 'Ú', 'Û', 'Ù', 'ı', 'ˆ', '˜', '¯', '˘', '˙', '˚',
        '¸', '˝', '˛', 'ˇ',
    ];
    match code {
        0x20..=0x7E => Some(char::from(code)),
        0x80..=0xFF => HIGH.get(usize::from(code - 0x80)).copied(),
        _ => None,
    }
}

/// StandardEncoding: ASCII with typographic quotes, and its own upper half.
fn standard_name(code: u8) -> Option<&'static str> {
    match code {
        0x27 => Some("quoteright"),
        0x60 => Some("quoteleft"),
        0x20..=0x7E => char_name(char::from(code)),
        _ => {
            const HIGH: [(u8, &str); 54] = [
                (0xA1, "exclamdown"),
                (0xA2, "cent"),
                (0xA3, "sterling"),
                (0xA4, "fraction"),
                (0xA5, "yen"),
                (0xA6, "florin"),
                (0xA7, "section"),
                (0xA8, "currency"),
                (0xA9, "quotesingle"),
                (0xAA, "quotedblleft"),
                (0xAB, "guillemotleft"),
                (0xAC, "guilsinglleft"),
                (0xAD, "guilsinglright"),
                (0xAE, "fi"),
                (0xAF, "fl"),
                (0xB1, "endash"),
                (0xB2, "dagger"),
                (0xB3, "daggerdbl"),
                (0xB4, "periodcentered"),
                (0xB6, "paragraph"),
                (0xB7, "bullet"),
                (0xB8, "quotesinglbase"),
                (0xB9, "quotedblbase"),
                (0xBA, "quotedblright"),
                (0xBB, "guillemotright"),
                (0xBC, "ellipsis"),
                (0xBD, "perthousand"),
                (0xBF, "questiondown"),
                (0xC1, "grave"),
                (0xC2, "acute"),
                (0xC3, "circumflex"),
                (0xC4, "tilde"),
                (0xC5, "macron"),
                (0xC6, "breve"),
                (0xC7, "dotaccent"),
                (0xC8, "dieresis"),
                (0xCA, "ring"),
                (0xCB, "cedilla"),
                (0xCD, "hungarumlaut"),
                (0xCE, "ogonek"),
                (0xCF, "caron"),
                (0xD0, "emdash"),
                (0xE1, "AE"),
                (0xE3, "ordfeminine"),
                (0xE8, "Lslash"),
                (0xE9, "Oslash"),
                (0xEA, "OE"),
                (0xEB, "ordmasculine"),
                (0xF1, "ae"),
                (0xF5, "dotlessi"),
                (0xF8, "lslash"),
                (0xF9, "oslash"),
                (0xFA, "oe"),
                (0xFB, "germandbls"),
            ];
            HIGH.iter().find(|(c, _)| *c == code).map(|(_, n)| *n)
        }
    }
}

/// (Unicode, glyph name) pairs of the Adobe Glyph List for the characters
/// of the three base encodings plus Romanian and Hungarian letters.
const NAMES: &[(char, &str)] = &[
    (' ', "space"),
    ('!', "exclam"),
    ('"', "quotedbl"),
    ('#', "numbersign"),
    ('$', "dollar"),
    ('%', "percent"),
    ('&', "ampersand"),
    ('\'', "quotesingle"),
    ('(', "parenleft"),
    (')', "parenright"),
    ('*', "asterisk"),
    ('+', "plus"),
    (',', "comma"),
    ('-', "hyphen"),
    ('.', "period"),
    ('/', "slash"),
    ('0', "zero"),
    ('1', "one"),
    ('2', "two"),
    ('3', "three"),
    ('4', "four"),
    ('5', "five"),
    ('6', "six"),
    ('7', "seven"),
    ('8', "eight"),
    ('9', "nine"),
    (':', "colon"),
    (';', "semicolon"),
    ('<', "less"),
    ('=', "equal"),
    ('>', "greater"),
    ('?', "question"),
    ('@', "at"),
    ('[', "bracketleft"),
    ('\\', "backslash"),
    (']', "bracketright"),
    ('^', "asciicircum"),
    ('_', "underscore"),
    ('`', "grave"),
    ('{', "braceleft"),
    ('|', "bar"),
    ('}', "braceright"),
    ('~', "asciitilde"),
    ('\u{a0}', "space"),
    ('¡', "exclamdown"),
    ('¢', "cent"),
    ('£', "sterling"),
    ('¤', "currency"),
    ('¥', "yen"),
    ('¦', "brokenbar"),
    ('§', "section"),
    ('¨', "dieresis"),
    ('©', "copyright"),
    ('ª', "ordfeminine"),
    ('«', "guillemotleft"),
    ('¬', "logicalnot"),
    ('\u{ad}', "hyphen"),
    ('®', "registered"),
    ('¯', "macron"),
    ('°', "degree"),
    ('±', "plusminus"),
    ('²', "twosuperior"),
    ('³', "threesuperior"),
    ('´', "acute"),
    ('µ', "mu"),
    ('¶', "paragraph"),
    ('·', "periodcentered"),
    ('¸', "cedilla"),
    ('¹', "onesuperior"),
    ('º', "ordmasculine"),
    ('»', "guillemotright"),
    ('¼', "onequarter"),
    ('½', "onehalf"),
    ('¾', "threequarters"),
    ('¿', "questiondown"),
    ('À', "Agrave"),
    ('Á', "Aacute"),
    ('Â', "Acircumflex"),
    ('Ã', "Atilde"),
    ('Ä', "Adieresis"),
    ('Å', "Aring"),
    ('Æ', "AE"),
    ('Ç', "Ccedilla"),
    ('È', "Egrave"),
    ('É', "Eacute"),
    ('Ê', "Ecircumflex"),
    ('Ë', "Edieresis"),
    ('Ì', "Igrave"),
    ('Í', "Iacute"),
    ('Î', "Icircumflex"),
    ('Ï', "Idieresis"),
    ('Ð', "Eth"),
    ('Ñ', "Ntilde"),
    ('Ò', "Ograve"),
    ('Ó', "Oacute"),
    ('Ô', "Ocircumflex"),
    ('Õ', "Otilde"),
    ('Ö', "Odieresis"),
    ('×', "multiply"),
    ('Ø', "Oslash"),
    ('Ù', "Ugrave"),
    ('Ú', "Uacute"),
    ('Û', "Ucircumflex"),
    ('Ü', "Udieresis"),
    ('Ý', "Yacute"),
    ('Þ', "Thorn"),
    ('ß', "germandbls"),
    ('à', "agrave"),
    ('á', "aacute"),
    ('â', "acircumflex"),
    ('ã', "atilde"),
    ('ä', "adieresis"),
    ('å', "aring"),
    ('æ', "ae"),
    ('ç', "ccedilla"),
    ('è', "egrave"),
    ('é', "eacute"),
    ('ê', "ecircumflex"),
    ('ë', "edieresis"),
    ('ì', "igrave"),
    ('í', "iacute"),
    ('î', "icircumflex"),
    ('ï', "idieresis"),
    ('ð', "eth"),
    ('ñ', "ntilde"),
    ('ò', "ograve"),
    ('ó', "oacute"),
    ('ô', "ocircumflex"),
    ('õ', "otilde"),
    ('ö', "odieresis"),
    ('÷', "divide"),
    ('ø', "oslash"),
    ('ù', "ugrave"),
    ('ú', "uacute"),
    ('û', "ucircumflex"),
    ('ü', "udieresis"),
    ('ý', "yacute"),
    ('þ', "thorn"),
    ('ÿ', "ydieresis"),
    ('Ă', "Abreve"),
    ('ă', "abreve"),
    ('Ő', "Ohungarumlaut"),
    ('ő', "ohungarumlaut"),
    ('Ű', "Uhungarumlaut"),
    ('ű', "uhungarumlaut"),
    ('Œ', "OE"),
    ('œ', "oe"),
    ('Š', "Scaron"),
    ('š', "scaron"),
    ('Ș', "Scommaaccent"),
    ('ș', "scommaaccent"),
    ('Ş', "Scedilla"),
    ('ş', "scedilla"),
    ('Ț', "Tcommaaccent"),
    ('ț', "tcommaaccent"),
    ('Ţ', "Tcedilla"),
    ('ţ', "tcedilla"),
    ('Ÿ', "Ydieresis"),
    ('Ž', "Zcaron"),
    ('ž', "zcaron"),
    ('Ł', "Lslash"),
    ('ł', "lslash"),
    ('ı', "dotlessi"),
    ('ƒ', "florin"),
    ('ˆ', "circumflex"),
    ('ˇ', "caron"),
    ('˘', "breve"),
    ('˙', "dotaccent"),
    ('˚', "ring"),
    ('˛', "ogonek"),
    ('˜', "tilde"),
    ('˝', "hungarumlaut"),
    ('–', "endash"),
    ('—', "emdash"),
    ('‘', "quoteleft"),
    ('’', "quoteright"),
    ('‚', "quotesinglbase"),
    ('“', "quotedblleft"),
    ('”', "quotedblright"),
    ('„', "quotedblbase"),
    ('†', "dagger"),
    ('‡', "daggerdbl"),
    ('•', "bullet"),
    ('…', "ellipsis"),
    ('‰', "perthousand"),
    ('‹', "guilsinglleft"),
    ('›', "guilsinglright"),
    ('⁄', "fraction"),
    ('€', "Euro"),
    ('™', "trademark"),
    ('−', "minus"),
    ('ﬁ', "fi"),
    ('ﬂ', "fl"),
    ('≠', "notequal"),
    ('≤', "lessequal"),
    ('≥', "greaterequal"),
    ('∞', "infinity"),
    ('∂', "partialdiff"),
    ('∑', "summation"),
    ('∏', "product"),
    ('π', "pi"),
    ('∫', "integral"),
    ('Ω', "Omega"),
    ('√', "radical"),
    ('≈', "approxequal"),
    ('∆', "Delta"),
    ('◊', "lozenge"),
    ('\u{f8ff}', "apple"),
];

/// The glyph name of a character, if it has a standard one.
pub fn char_name(c: char) -> Option<&'static str> {
    if c.is_ascii_alphabetic() {
        // Letters are their own names.
        return ASCII_LETTERS
            .iter()
            .find(|n| n.starts_with(c) && n.len() == 1)
            .copied();
    }
    NAMES.iter().find(|(ch, _)| *ch == c).map(|(_, n)| *n)
}

const ASCII_LETTERS: [&str; 52] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S",
    "T", "U", "V", "W", "X", "Y", "Z", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l",
    "m", "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z",
];

/// The character a glyph name stands for: `uniXXXX`, `uXXXX[XX]`, single
/// letters, names from the list above, and `name.suffix` variants.
pub fn name_char(name: &str) -> Option<char> {
    let base = name.split('.').next().unwrap_or(name);
    if let Some(hex) = base.strip_prefix("uni").filter(|h| h.len() == 4) {
        return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
    }
    if let Some(hex) = base
        .strip_prefix('u')
        .filter(|h| (4..=6).contains(&h.len()) && h.chars().all(|c| c.is_ascii_hexdigit()))
    {
        return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
    }
    let mut chars = base.chars();
    if let (Some(c), None) = (chars.next(), chars.next())
        && c.is_ascii_alphabetic()
    {
        return Some(c);
    }
    NAMES
        .iter()
        // Prefer the first (canonical) character for names used twice.
        .find(|(_, n)| *n == base)
        .map(|(c, _)| *c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodings_and_names() {
        assert_eq!(BaseEncoding::WinAnsi.glyph_name(0x80), Some("Euro"));
        assert_eq!(BaseEncoding::Standard.glyph_name(0x27), Some("quoteright"));
        assert_eq!(BaseEncoding::MacRoman.glyph_name(0x8A), Some("adieresis"));
        assert_eq!(BaseEncoding::WinAnsi.glyph_name(b'A'), Some("A"));
        assert_eq!(name_char("scommaaccent"), Some('ș'));
        assert_eq!(name_char("uni0103"), Some('ă'));
        assert_eq!(name_char("a.sc"), Some('a'));
        assert_eq!(name_char("space"), Some(' '));
        assert_eq!(char_name('€'), Some("Euro"));
    }
}
