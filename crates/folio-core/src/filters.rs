//! Stream filters (ISO 32000-2 §7.4): decoding for the general-purpose
//! filters, plus Flate encoding for the writer. Image-only filters (DCT, JPX,
//! JBIG2, CCITT) are left encoded; images are decoded by the renderer.

use crate::error::{Error, Result};
use crate::object::{Dict, Object, Stream};

/// Decoded data larger than this is refused (a "zip bomb" guard).
pub const MAX_DECODED: usize = 512 * 1024 * 1024;

/// Decode a stream through all of its filters.
pub fn decode_stream(stream: &Stream) -> Result<Vec<u8>> {
    let filters = names_of(stream.dict.get(b"Filter"));
    let parms = parms_of(stream.dict.get(b"DecodeParms"), filters.len());
    let mut data = stream.data.clone();
    for (i, f) in filters.iter().enumerate() {
        let p = parms.get(i).and_then(|p| p.as_ref());
        data = decode(f, &data, p)?;
    }
    Ok(data)
}

/// Whether every filter of this stream can be decoded here.
pub fn can_decode(stream: &Stream) -> bool {
    names_of(stream.dict.get(b"Filter"))
        .iter()
        .all(|f| GENERAL.contains(&f.as_slice()))
}

const GENERAL: &[&[u8]] = &[
    b"FlateDecode",
    b"Fl",
    b"LZWDecode",
    b"LZW",
    b"ASCIIHexDecode",
    b"AHx",
    b"ASCII85Decode",
    b"A85",
    b"RunLengthDecode",
    b"RL",
];

fn names_of(o: Option<&Object>) -> Vec<Vec<u8>> {
    match o {
        Some(Object::Name(n)) => vec![n.clone()],
        Some(Object::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_name().map(<[u8]>::to_vec))
            .collect(),
        _ => Vec::new(),
    }
}

fn parms_of(o: Option<&Object>, n: usize) -> Vec<Option<Dict>> {
    match o {
        Some(Object::Dict(d)) => vec![Some(d.clone())],
        Some(Object::Array(a)) => a.iter().map(|x| x.as_dict().cloned()).collect(),
        _ => vec![None; n],
    }
}

/// Decode `data` with one filter.
pub fn decode(filter: &[u8], data: &[u8], parms: Option<&Dict>) -> Result<Vec<u8>> {
    match filter {
        b"FlateDecode" | b"Fl" => predict(flate_decode(data)?, parms),
        b"LZWDecode" | b"LZW" => {
            let early = parms.and_then(|p| p.get_int(b"EarlyChange")).unwrap_or(1) != 0;
            predict(lzw_decode(data, early)?, parms)
        }
        b"ASCIIHexDecode" | b"AHx" => Ok(ascii_hex_decode(data)),
        b"ASCII85Decode" | b"A85" => ascii85_decode(data),
        b"RunLengthDecode" | b"RL" => Ok(run_length_decode(data)),
        other => Err(Error::Unsupported(format!(
            "stream filter /{}",
            String::from_utf8_lossy(other)
        ))),
    }
}

pub fn flate_decode(data: &[u8]) -> Result<Vec<u8>> {
    use miniz_oxide::inflate::{TINFLStatus, decompress_to_vec_zlib_with_limit};
    match decompress_to_vec_zlib_with_limit(data, MAX_DECODED) {
        Ok(v) => Ok(v),
        Err(e) if e.status == TINFLStatus::HasMoreOutput => Err(Error::Limit(format!(
            "a stream decodes to more than {MAX_DECODED} bytes"
        ))),
        Err(e) => {
            // Many real files have truncated or checksum-broken streams;
            // keep what decoded, like other readers do.
            if !e.output.is_empty() {
                return Ok(e.output);
            }
            miniz_oxide::inflate::decompress_to_vec_with_limit(data, MAX_DECODED)
                .map_err(|_| Error::Invalid("a Flate stream is corrupt".into()))
        }
    }
}

pub fn flate_encode(data: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(data, 6)
}

fn predict(data: Vec<u8>, parms: Option<&Dict>) -> Result<Vec<u8>> {
    let Some(p) = parms else { return Ok(data) };
    let predictor = p.get_int(b"Predictor").unwrap_or(1);
    if predictor < 2 {
        return Ok(data);
    }
    let colors = p.get_int(b"Colors").unwrap_or(1).clamp(1, 32) as usize;
    let bpc = p.get_int(b"BitsPerComponent").unwrap_or(8).clamp(1, 16) as usize;
    let columns = p.get_int(b"Columns").unwrap_or(1).clamp(1, 1 << 24) as usize;
    let bits_per_pixel = colors * bpc;
    let bpp = bits_per_pixel.div_ceil(8).max(1);
    let row_len = (bits_per_pixel * columns).div_ceil(8);
    if predictor == 2 {
        if bpc != 8 {
            return Err(Error::Unsupported(
                "TIFF predictor with other than 8 bits per component".into(),
            ));
        }
        let mut out = data;
        for row in out.chunks_mut(row_len) {
            for i in bpp..row.len() {
                let left = row.get(i - bpp).copied().unwrap_or(0);
                if let Some(b) = row.get_mut(i) {
                    *b = b.wrapping_add(left);
                }
            }
        }
        return Ok(out);
    }
    // PNG predictors: every row starts with its own filter-type byte.
    let mut out = Vec::with_capacity(data.len());
    let mut prev = vec![0u8; row_len];
    for chunk in data.chunks(row_len + 1) {
        let Some((&ft, row)) = chunk.split_first() else {
            break;
        };
        let mut cur = row.to_vec();
        for i in 0..cur.len() {
            let a = if i >= bpp {
                cur.get(i - bpp).copied().unwrap_or(0)
            } else {
                0
            };
            let b = prev.get(i).copied().unwrap_or(0);
            let c = if i >= bpp {
                prev.get(i - bpp).copied().unwrap_or(0)
            } else {
                0
            };
            let add = match ft {
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                4 => paeth(a, b, c),
                _ => 0,
            };
            if let Some(x) = cur.get_mut(i) {
                *x = x.wrapping_add(add);
            }
        }
        out.extend_from_slice(&cur);
        cur.resize(row_len, 0);
        prev = cur;
    }
    Ok(out)
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let pa = (p - i16::from(a)).abs();
    let pb = (p - i16::from(b)).abs();
    let pc = (p - i16::from(c)).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

pub fn lzw_decode(data: &[u8], early_change: bool) -> Result<Vec<u8>> {
    let mut table: Vec<Vec<u8>> = (0..=255u8).map(|b| vec![b]).collect();
    table.push(Vec::new()); // 256 clear
    table.push(Vec::new()); // 257 end of data
    let mut out = Vec::new();
    let mut prev: Option<Vec<u8>> = None;
    let mut code_len = 9u32;
    let mut bitbuf: u32 = 0;
    let mut nbits: u32 = 0;
    let mut bytes = data.iter();
    loop {
        while nbits < code_len {
            let Some(&b) = bytes.next() else {
                return Ok(out);
            };
            bitbuf = (bitbuf << 8) | u32::from(b);
            nbits += 8;
        }
        let code = ((bitbuf >> (nbits - code_len)) & ((1 << code_len) - 1)) as usize;
        nbits -= code_len;
        bitbuf &= (1u32 << nbits).wrapping_sub(1);
        match code {
            256 => {
                table.truncate(258);
                code_len = 9;
                prev = None;
                continue;
            }
            257 => return Ok(out),
            _ => {}
        }
        let entry = match (table.get(code), &prev) {
            (Some(e), _) if !(256..=257).contains(&code) => e.clone(),
            (None, Some(p)) if code == table.len() => {
                let mut e = p.clone();
                e.push(p.first().copied().unwrap_or(0));
                e
            }
            _ => return Ok(out), // corrupt: keep what we have
        };
        out.extend_from_slice(&entry);
        if out.len() > MAX_DECODED {
            return Err(Error::Limit("LZW stream decodes too large".into()));
        }
        if let Some(p) = prev
            && table.len() < 4096
        {
            let mut n = p;
            n.push(entry.first().copied().unwrap_or(0));
            table.push(n);
        }
        prev = Some(entry);
        let size = table.len() + usize::from(early_change);
        code_len = if size >= 2048 {
            12
        } else if size >= 1024 {
            11
        } else if size >= 512 {
            10
        } else {
            9
        };
    }
}

pub fn ascii_hex_decode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2);
    let mut high: Option<u8> = None;
    for &b in data {
        if b == b'>' {
            break;
        }
        let v = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => continue,
        };
        match high.take() {
            Some(h) => out.push(h << 4 | v),
            None => high = Some(v),
        }
    }
    if let Some(h) = high {
        out.push(h << 4);
    }
    out
}

pub fn ascii85_decode(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() * 4 / 5);
    let mut group = [0u8; 5];
    let mut n = 0usize;
    let body = data.strip_prefix(b"<~").unwrap_or(data);
    for &b in body {
        match b {
            b'~' => break,
            b'z' if n == 0 => out.extend_from_slice(&[0, 0, 0, 0]),
            b'!'..=b'u' => {
                if let Some(slot) = group.get_mut(n) {
                    *slot = b - b'!';
                }
                n += 1;
                if n == 5 {
                    out.extend_from_slice(&a85_group(&group)?);
                    n = 0;
                }
            }
            b if crate::lexer::is_ws(b) => {}
            _ => return Err(Error::Invalid("ASCII85 stream has an invalid byte".into())),
        }
    }
    if n > 1 {
        for slot in group.iter_mut().skip(n) {
            *slot = b'u' - b'!';
        }
        let full = a85_group(&group)?;
        out.extend_from_slice(full.get(..n - 1).unwrap_or_default());
    }
    Ok(out)
}

fn a85_group(g: &[u8; 5]) -> Result<[u8; 4]> {
    let v = g.iter().fold(0u64, |acc, &d| acc * 85 + u64::from(d));
    let v = u32::try_from(v).map_err(|_| Error::Invalid("ASCII85 group overflows".into()))?;
    Ok(v.to_be_bytes())
}

pub fn run_length_decode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while let Some(&len) = data.get(i) {
        i += 1;
        match len {
            128 => break,
            0..=127 => {
                let n = usize::from(len) + 1;
                let end = (i + n).min(data.len());
                out.extend_from_slice(data.get(i..end).unwrap_or_default());
                i = end;
            }
            _ => {
                let Some(&b) = data.get(i) else { break };
                i += 1;
                out.extend(std::iter::repeat_n(b, 257 - usize::from(len)));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flate_roundtrip() {
        let data = b"hello hello hello hello".repeat(10);
        assert_eq!(flate_decode(&flate_encode(&data)).unwrap(), data);
    }

    #[test]
    fn lzw_spec_example() {
        // ISO 32000-2 §7.4.4.2: "-----A---B" encodes to these bytes.
        let enc = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        assert_eq!(lzw_decode(&enc, true).unwrap(), b"-----A---B");
    }

    #[test]
    fn ascii_hex() {
        assert_eq!(ascii_hex_decode(b"48 65 6C6C 6F>"), b"Hello");
        assert_eq!(ascii_hex_decode(b"7>"), vec![0x70]);
    }

    #[test]
    fn ascii85() {
        // "Man " is the classic example: 9jqo^
        assert_eq!(ascii85_decode(b"9jqo^~>").unwrap(), b"Man ");
        assert_eq!(ascii85_decode(b"z~>").unwrap(), vec![0, 0, 0, 0]);
        assert_eq!(ascii85_decode(b"9jqo~>").unwrap(), b"Man");
        assert!(ascii85_decode(b"vvvvv~>").is_err());
    }

    #[test]
    fn run_length() {
        assert_eq!(
            run_length_decode(&[2, b'a', b'b', b'c', 254, b'z', 128]),
            b"abczzz"
        );
    }

    #[test]
    fn png_up_predictor() {
        let mut parms = Dict::new();
        parms.set("Predictor", Object::Int(12));
        parms.set("Columns", Object::Int(3));
        let data = vec![0, 1, 2, 3, 2, 1, 1, 1];
        assert_eq!(predict(data, Some(&parms)).unwrap(), vec![1, 2, 3, 2, 3, 4]);
    }

    #[test]
    fn unknown_filter_is_unsupported() {
        assert!(matches!(
            decode(b"DCTDecode", b"", None),
            Err(Error::Unsupported(_))
        ));
    }
}
