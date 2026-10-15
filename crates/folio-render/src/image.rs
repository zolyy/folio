//! Images (ISO 32000-2 §8.9): sample decoding through every filter, colour
//! conversion, Decode arrays, stencil masks, colour-key masks, explicit
//! masks and soft masks. The result is an RGBA pixmap in image space.
//!
//! JPEG, JPEG 2000, JBIG2 and CCITT data go through pure-Rust decoders
//! (zune-jpeg, hayro-*). Each call into them sits behind `catch_unwind`, so
//! a bug in a decoder becomes "image not drawn", never a crash.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

use folio_core::filters;
use folio_core::{Dict, Document, Object};
use tiny_skia::{IntSize, Pixmap};

use crate::color::ColorSpace;

/// Largest image (in pixels) Folio decodes; larger ones are reported, not drawn.
pub const MAX_IMAGE_PIXELS: u64 = 100_000_000;

/// Decoded samples before colour conversion.
struct Samples {
    width: u32,
    height: u32,
    comps: usize,
    bpc: u32,
    data: Vec<u8>,
    /// JPEG 2000 alpha channel (SMaskInData).
    alpha: Option<Vec<u8>>,
    /// Colour space the codec reported, used when the dictionary has none.
    native: Option<ColorSpace>,
}

/// Look up `full` or its inline-image abbreviation.
fn key<'a>(d: &'a Dict, full: &[u8], short: &[u8]) -> Option<&'a Object> {
    d.get(full).or_else(|| d.get(short))
}

fn filter_name(n: &[u8]) -> &[u8] {
    match n {
        b"AHx" => b"ASCIIHexDecode",
        b"A85" => b"ASCII85Decode",
        b"LZW" => b"LZWDecode",
        b"Fl" => b"FlateDecode",
        b"RL" => b"RunLengthDecode",
        b"CCF" => b"CCITTFaxDecode",
        b"DCT" => b"DCTDecode",
        other => other,
    }
}

/// Decode an image dictionary and its (still encoded) data into an RGBA
/// pixmap. `fill` is the current fill colour, used by stencil masks.
pub fn decode(
    doc: &Document,
    dict: &Dict,
    data: &[u8],
    res: Option<&Dict>,
    fill: [f32; 4],
) -> Result<Pixmap, String> {
    let is_mask = matches!(key(dict, b"ImageMask", b"IM"), Some(Object::Bool(true)));
    let s = samples(doc, dict, data, is_mask)?;
    let n_px = (s.width as usize).saturating_mul(s.height as usize);
    let decode_arr: Vec<f64> = key(dict, b"Decode", b"D")
        .map(|o| doc.resolve(o))
        .and_then(Object::as_array)
        .map(|a| a.iter().filter_map(|x| doc.resolve(x).as_f64()).collect())
        .unwrap_or_default();

    let mut rgba = vec![0u8; n_px.saturating_mul(4)];
    if is_mask {
        let inverted = decode_arr.first().copied().unwrap_or(0.0) > 0.5;
        let a = fill[3];
        for (i, px) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let bit = sample(&s, i, 0) != 0;
            // Sample 0 paints by default; Decode [1 0] swaps that.
            if bit == inverted {
                px.copy_from_slice(&premul(fill[0], fill[1], fill[2], a));
            }
        }
        return pixmap(rgba, s.width, s.height);
    }

    let cs = match key(dict, b"ColorSpace", b"CS") {
        Some(o) => ColorSpace::parse(doc, o, res).ok_or("unknown image colour space")?,
        None => s.native.clone().unwrap_or(match s.comps {
            1 => ColorSpace::Gray,
            4 => ColorSpace::Cmyk,
            _ => ColorSpace::Rgb,
        }),
    };
    let comps = cs.components().min(s.comps.max(1));
    let max = ((1u64 << s.bpc.min(16)) - 1) as f64;
    let indexed = matches!(cs, ColorSpace::Indexed { .. });
    let range = |c: usize| -> (f64, f64) {
        match (decode_arr.get(2 * c), decode_arr.get(2 * c + 1)) {
            (Some(&a), Some(&b)) => (a, b),
            _ if indexed => (0.0, max),
            _ => (0.0, 1.0),
        }
    };
    // Colour-key masking: a list of [min max] sample ranges.
    let key_ranges: Option<Vec<(u32, u32)>> = match dict.get(b"Mask").map(|o| doc.resolve(o)) {
        Some(Object::Array(a)) => Some(
            a.chunks(2)
                .map(|p| {
                    let v = |i: usize| {
                        p.get(i)
                            .and_then(|x| doc.resolve(x).as_int())
                            .unwrap_or(0)
                            .clamp(0, i64::from(u32::MAX)) as u32
                    };
                    (v(0), v(1))
                })
                .collect(),
        ),
        _ => None,
    };

    let mut cache: HashMap<Vec<u32>, [u8; 3]> = HashMap::new();
    let mut raw = vec![0u32; comps];
    let mut vals = vec![0.0f64; comps];
    for (i, px) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        for c in 0..comps {
            let r = sample(&s, i, c);
            if let Some(slot) = raw.get_mut(c) {
                *slot = r;
            }
            let (lo, hi) = range(c);
            if let Some(slot) = vals.get_mut(c) {
                *slot = lo + f64::from(r) * (hi - lo) / max.max(1.0);
            }
        }
        if let Some(k) = &key_ranges
            && k.len() >= comps
            && raw
                .iter()
                .zip(k.iter())
                .all(|(&v, &(lo, hi))| v >= lo && v <= hi)
        {
            continue; // masked out: stays transparent
        }
        let rgb = match cache.get(&raw) {
            Some(c) => *c,
            None => {
                let c = cs
                    .to_rgb(&vals)
                    .map(|[r, g, b]| [to8(r), to8(g), to8(b)])
                    .unwrap_or([255, 255, 255]);
                if cache.len() < 65_536 {
                    cache.insert(raw.clone(), c);
                }
                c
            }
        };
        px.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }

    // Alpha from a soft mask, an explicit mask, or the JPEG 2000 alpha channel.
    let alpha: Option<Vec<u8>> = if let Some(sm) = dict
        .get(b"SMask")
        .map(|o| doc.resolve(o))
        .and_then(Object::as_stream)
    {
        alpha_from(doc, &sm.dict, &sm.data, false, s.width, s.height)
    } else if let Some(m) = dict
        .get(b"Mask")
        .map(|o| doc.resolve(o))
        .and_then(Object::as_stream)
    {
        alpha_from(doc, &m.dict, &m.data, true, s.width, s.height)
    } else {
        s.alpha.clone()
    };
    if let Some(a) = alpha {
        for (px, &al) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(a.iter()) {
            if px[3] != 0 {
                px[3] = al;
            }
        }
    }
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = u16::from(px[3]);
        for c in px.iter_mut().take(3) {
            *c = ((u16::from(*c) * a + 127) / 255) as u8;
        }
    }
    pixmap(rgba, s.width, s.height)
}

fn to8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

fn premul(r: f32, g: f32, b: f32, a: f32) -> [u8; 4] {
    let a = a.clamp(0.0, 1.0);
    [to8(r * a), to8(g * a), to8(b * a), to8(a)]
}

fn pixmap(rgba: Vec<u8>, w: u32, h: u32) -> Result<Pixmap, String> {
    let size = IntSize::from_wh(w, h).ok_or("empty image")?;
    Pixmap::from_vec(rgba, size).ok_or_else(|| "image too large".into())
}

/// The `c`-th component of pixel `i`. Rows are padded to whole bytes.
fn sample(s: &Samples, i: usize, c: usize) -> u32 {
    let w = s.width as usize;
    let (x, y) = (i % w.max(1), i / w.max(1));
    let row_bits = w * s.comps * s.bpc as usize;
    let row_bytes = row_bits.div_ceil(8);
    let bit = y * row_bytes * 8 + (x * s.comps + c) * s.bpc as usize;
    match s.bpc {
        8 => u32::from(s.data.get(bit / 8).copied().unwrap_or(0)),
        16 => {
            let hi = s.data.get(bit / 8).copied().unwrap_or(0);
            let lo = s.data.get(bit / 8 + 1).copied().unwrap_or(0);
            u32::from(u16::from_be_bytes([hi, lo]))
        }
        n => {
            let byte = s.data.get(bit / 8).copied().unwrap_or(0);
            let shift = 8 - (bit % 8) as u32 - n;
            (u32::from(byte) >> shift) & ((1 << n) - 1)
        }
    }
}

/// A mask image (soft: gray values; explicit: stencil, 0 = painted) as an
/// alpha channel, resampled to `w` × `h` by nearest neighbour.
fn alpha_from(
    doc: &Document,
    md: &Dict,
    data: &[u8],
    stencil: bool,
    w: u32,
    h: u32,
) -> Option<Vec<u8>> {
    let s = samples(doc, md, data, stencil).ok()?;
    let decode_inverted = md
        .get(b"Decode")
        .and_then(Object::as_array)
        .and_then(|a| a.first())
        .and_then(Object::as_f64)
        .is_some_and(|v| v > 0.5);
    let max = ((1u64 << s.bpc.min(16)) - 1) as f64;
    let mut out = Vec::with_capacity((w as usize).saturating_mul(h as usize));
    for y in 0..h {
        let sy = (u64::from(y) * u64::from(s.height) / u64::from(h.max(1))) as usize;
        for x in 0..w {
            let sx = (u64::from(x) * u64::from(s.width) / u64::from(w.max(1))) as usize;
            let v = sample(&s, sy * s.width as usize + sx, 0);
            let a = if stencil {
                if (v != 0) == decode_inverted { 255 } else { 0 }
            } else {
                let f = f64::from(v) / max.max(1.0);
                let f = if decode_inverted { 1.0 - f } else { f };
                (f * 255.0 + 0.5) as u8
            };
            out.push(a);
        }
    }
    Some(out)
}

/// Run the filters and the image codec; return the raw samples.
fn samples(doc: &Document, dict: &Dict, data: &[u8], is_mask: bool) -> Result<Samples, String> {
    let int = |full: &[u8], short: &[u8]| {
        key(dict, full, short)
            .map(|o| doc.resolve(o))
            .and_then(Object::as_int)
    };
    let width = int(b"Width", b"W").unwrap_or(0);
    let height = int(b"Height", b"H").unwrap_or(0);
    if width <= 0 || height <= 0 || width > 1 << 20 || height > 1 << 20 {
        return Err("bad image size".into());
    }
    let (width, height) = (width as u32, height as u32);
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err("image larger than 100 megapixels".into());
    }
    let filters: Vec<Vec<u8>> = match key(dict, b"Filter", b"F").map(|o| doc.resolve(o)) {
        Some(Object::Name(n)) => vec![filter_name(n).to_vec()],
        Some(Object::Array(a)) => a
            .iter()
            .filter_map(|x| doc.resolve(x).as_name().map(|n| filter_name(n).to_vec()))
            .collect(),
        _ => Vec::new(),
    };
    let parms: Vec<Option<Dict>> = match key(dict, b"DecodeParms", b"DP").map(|o| doc.resolve(o)) {
        Some(Object::Dict(d)) => vec![Some(d.clone())],
        Some(Object::Array(a)) => a
            .iter()
            .map(|x| doc.resolve(x).as_dict().cloned())
            .collect(),
        _ => Vec::new(),
    };
    let comps_for = |cs: Option<&Object>| -> usize {
        if is_mask {
            return 1;
        }
        cs.and_then(|o| ColorSpace::parse(doc, o, None))
            .map(|c| c.components())
            .unwrap_or(1)
    };
    let mut buf = data.to_vec();
    for (i, f) in filters.iter().enumerate() {
        let p = parms.get(i).and_then(|p| p.as_ref());
        match f.as_slice() {
            b"DCTDecode" => return guard(|| jpeg(&buf, width, height)),
            b"JPXDecode" => return guard(|| jpx(&buf, is_mask)),
            b"JBIG2Decode" => {
                let globals = p
                    .and_then(|p| p.get(b"JBIG2Globals"))
                    .map(|o| doc.resolve(o))
                    .and_then(Object::as_stream)
                    .and_then(|s| filters::decode_stream(s).ok());
                return guard(|| jbig2(&buf, globals.as_deref()));
            }
            b"CCITTFaxDecode" => return guard(|| ccitt(&buf, p, width, height)),
            other => {
                buf = filters::decode(other, &buf, p).map_err(|e| e.to_string())?;
            }
        }
    }
    let bpc = int(b"BitsPerComponent", b"BPC").unwrap_or(if is_mask { 1 } else { 8 });
    let bpc = if is_mask { 1 } else { bpc };
    if !matches!(bpc, 1 | 2 | 4 | 8 | 16) {
        return Err(format!("{bpc} bits per component"));
    }
    Ok(Samples {
        width,
        height,
        comps: comps_for(key(dict, b"ColorSpace", b"CS")),
        bpc: bpc as u32,
        data: buf,
        alpha: None,
        native: None,
    })
}

/// Run a third-party decoder; a panic inside it becomes an error.
fn guard(f: impl FnOnce() -> Result<Samples, String>) -> Result<Samples, String> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| Err("the image decoder failed".into()))
}

fn jpeg(data: &[u8], w: u32, h: u32) -> Result<Samples, String> {
    use zune_jpeg::JpegDecoder;
    use zune_jpeg::zune_core::colorspace::ColorSpace as Zcs;
    use zune_jpeg::zune_core::options::DecoderOptions;
    let opts = DecoderOptions::default()
        .set_max_width((w as usize).max(1) * 2)
        .set_max_height((h as usize).max(1) * 2);
    let mut dec = JpegDecoder::new_with_options(data, opts);
    dec.decode_headers().map_err(|e| format!("JPEG: {e:?}"))?;
    let input = dec.get_input_colorspace();
    let opts = DecoderOptions::default()
        .set_max_width((w as usize).max(1) * 2)
        .set_max_height((h as usize).max(1) * 2)
        .jpeg_set_out_colorspace(match input {
            Some(Zcs::Luma) => Zcs::Luma,
            Some(Zcs::CMYK | Zcs::YCCK) => Zcs::CMYK,
            _ => Zcs::RGB,
        });
    let mut dec = JpegDecoder::new_with_options(data, opts);
    let pixels = dec.decode().map_err(|e| format!("JPEG: {e:?}"))?;
    let info = dec.info().ok_or("JPEG without a header")?;
    let comps = match dec.get_output_colorspace() {
        Some(Zcs::Luma) => 1,
        Some(Zcs::CMYK) => 4,
        _ => 3,
    };
    let (width, height) = (u32::from(info.width), u32::from(info.height));
    let mut data = pixels;
    if comps == 4 {
        // Adobe CMYK JPEGs store inverted values (APP14 transform).
        data.iter_mut().for_each(|b| *b = 255 - *b);
    }
    Ok(Samples {
        width,
        height,
        comps,
        bpc: 8,
        data,
        alpha: None,
        native: Some(match comps {
            1 => ColorSpace::Gray,
            4 => ColorSpace::Cmyk,
            _ => ColorSpace::Rgb,
        }),
    })
}

fn jpx(data: &[u8], is_mask: bool) -> Result<Samples, String> {
    use hayro_jpeg2000::{DecodeSettings, DecoderContext, Image};
    let img = Image::new(data, &DecodeSettings::default()).map_err(|e| format!("JPX: {e:?}"))?;
    let (width, height) = (img.width(), img.height());
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err("JPX image larger than 100 megapixels".into());
    }
    let channels = usize::from(img.color_space().num_channels());
    let has_alpha = img.has_alpha();
    let mut ctx = DecoderContext::default();
    let decoded = img.decode(&mut ctx).map_err(|e| format!("JPX: {e:?}"))?;
    if decoded.components().is_empty() {
        return Err("JPX image without components".into());
    }
    let all = decoded.data_u8();
    let stride = channels + usize::from(has_alpha);
    let (data, alpha) = if has_alpha && !is_mask {
        let mut d = Vec::with_capacity(all.len());
        let mut a = Vec::with_capacity(all.len() / stride.max(1));
        for px in all.chunks(stride.max(1)) {
            d.extend_from_slice(px.get(..channels).unwrap_or(px));
            a.push(px.get(channels).copied().unwrap_or(255));
        }
        (d, Some(a))
    } else {
        (all, None)
    };
    Ok(Samples {
        width,
        height,
        comps: channels,
        bpc: 8,
        data,
        alpha,
        native: Some(match channels {
            1 => ColorSpace::Gray,
            4 => ColorSpace::Cmyk,
            _ => ColorSpace::Rgb,
        }),
    })
}

/// Collects bilevel output as 1-bit rows, 0 = black (PDF's convention).
struct Bits {
    width: usize,
    data: Vec<u8>,
    x: usize,
    row: Vec<u8>,
    rows: usize,
    max_rows: usize,
}

impl Bits {
    fn new(width: usize, max_rows: usize) -> Self {
        Bits {
            width,
            data: Vec::new(),
            x: 0,
            row: vec![0xFF; width.div_ceil(8)],
            rows: 0,
            max_rows,
        }
    }
    fn push(&mut self, black: bool) {
        if self.x < self.width
            && black
            && let Some(b) = self.row.get_mut(self.x / 8)
        {
            *b &= !(0x80 >> (self.x % 8));
        }
        self.x += 1;
    }
    fn line(&mut self) {
        if self.rows < self.max_rows {
            self.data.extend_from_slice(&self.row);
            self.rows += 1;
        }
        self.row.iter_mut().for_each(|b| *b = 0xFF);
        self.x = 0;
    }
}

impl hayro_jbig2::Decoder for Bits {
    fn push_pixel(&mut self, black: bool) {
        self.push(black);
    }
    fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
        for _ in 0..(chunk_count as usize).saturating_mul(8).min(self.width + 8) {
            self.push(black);
        }
    }
    fn next_line(&mut self) {
        self.line();
    }
}

struct FaxBits {
    bits: Bits,
    black_is_1: bool,
}

impl hayro_ccitt::Decoder for FaxBits {
    fn push_pixels(&mut self, white: bool, count: u32) {
        for _ in 0..(count as usize).min(self.bits.width + 1) {
            // Stored as 0 = black unless BlackIs1 says the opposite.
            let black = !white;
            self.bits.push(if self.black_is_1 { !black } else { black });
        }
    }
    fn next_line(&mut self) {
        self.bits.line();
    }
}

fn jbig2(data: &[u8], globals: Option<&[u8]>) -> Result<Samples, String> {
    let img =
        hayro_jbig2::Image::new_embedded(data, globals).map_err(|e| format!("JBIG2: {e:?}"))?;
    let (width, height) = (img.width(), img.height());
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err("JBIG2 image larger than 100 megapixels".into());
    }
    let mut bits = Bits::new(width as usize, height as usize);
    img.decode(&mut bits).map_err(|e| format!("JBIG2: {e:?}"))?;
    Ok(Samples {
        width,
        height,
        comps: 1,
        bpc: 1,
        data: bits.data,
        alpha: None,
        native: Some(ColorSpace::Gray),
    })
}

fn ccitt(data: &[u8], p: Option<&Dict>, w: u32, h: u32) -> Result<Samples, String> {
    use hayro_ccitt::{DecodeSettings, DecoderContext, EncodingMode};
    let get = |k: &[u8]| p.and_then(|p| p.get(k));
    let int = |k: &[u8], d: i64| get(k).and_then(Object::as_int).unwrap_or(d);
    let flag = |k: &[u8]| matches!(get(k), Some(Object::Bool(true)));
    let k = int(b"K", 0);
    let columns = int(b"Columns", 1728).clamp(1, 1 << 20) as u32;
    let rows = match int(b"Rows", 0) {
        r if r > 0 => (r as u32).min(1 << 20),
        _ => h,
    };
    let settings = DecodeSettings {
        columns,
        rows,
        end_of_block: !matches!(get(b"EndOfBlock"), Some(Object::Bool(false))),
        end_of_line: flag(b"EndOfLine"),
        rows_are_byte_aligned: flag(b"EncodedByteAlign"),
        encoding: match k {
            k if k < 0 => EncodingMode::Group4,
            0 => EncodingMode::Group3_1D,
            k => EncodingMode::Group3_2D {
                k: u32::try_from(k).unwrap_or(1),
            },
        },
        invert_black: false,
    };
    let mut ctx = DecoderContext::new(settings);
    let mut out = FaxBits {
        bits: Bits::new(columns as usize, rows as usize),
        black_is_1: flag(b"BlackIs1"),
    };
    // A truncated stream still yields the rows decoded so far.
    let _ = hayro_ccitt::decode(data, &mut out, &mut ctx);
    let mut bits = out.bits;
    let row_bytes = (columns as usize).div_ceil(8);
    bits.data.resize(row_bytes * rows as usize, 0xFF);
    let _ = w;
    Ok(Samples {
        width: columns,
        height: rows,
        comps: 1,
        bpc: 1,
        data: bits.data,
        alpha: None,
        native: Some(ColorSpace::Gray),
    })
}
