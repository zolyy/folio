//! Export PDF → Image, with the settings of Acrobat Pro's dialog
//! (docs/acrobat/render.md): JPEG (quality, baseline / optimised /
//! progressive), PNG (filter), TIFF (none, LZW, ZIP, CCITT Group 4), colour
//! space (automatic, RGB, grayscale, CMYK, monochrome) and resolution.
//! TIFF files are written here; JPEG and PNG use pure-Rust encoders.

use folio_core::{Error, Result};
use tiny_skia::Pixmap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    /// Grayscale when the page has no colour, RGB otherwise.
    Auto,
    Rgb,
    Gray,
    Cmyk,
    /// 1 bit per pixel.
    Mono,
}

/// Acrobat's JPEG quality steps (**Folio choice** of the numeric values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Maximum,
    High,
    Medium,
    Low,
    Minimum,
}

impl Quality {
    fn value(self) -> u8 {
        match self {
            Quality::Maximum => 100,
            Quality::High => 85,
            Quality::Medium => 70,
            Quality::Low => 50,
            Quality::Minimum => 30,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JpegMode {
    /// Baseline (Standard).
    Baseline,
    /// Baseline (Optimized): optimised Huffman tables.
    Optimized,
    /// Progressive, with 3, 4 or 5 scans.
    Progressive(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PngFilter {
    None,
    Sub,
    Up,
    Average,
    Paeth,
    Adaptive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TiffCompression {
    None,
    Lzw,
    Zip,
    /// CCITT Group 4, monochrome only.
    CcittG4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png(PngFilter),
    Jpeg(Quality, JpegMode),
    Tiff(TiffCompression),
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Png(_) => "png",
            Format::Jpeg(..) => "jpg",
            Format::Tiff(_) => "tif",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub format: Format,
    pub color: ColorMode,
    /// Written into the file's resolution fields.
    pub dpi: f64,
}

/// Pixels flattened onto white, in the requested colour model.
enum Planes {
    Rgb(Vec<u8>),
    Gray(Vec<u8>),
    Cmyk(Vec<u8>),
    /// One byte per pixel, 0 = black, 1 = white.
    Mono(Vec<u8>),
}

fn flatten(pm: &Pixmap) -> Vec<[u8; 3]> {
    pm.pixels()
        .iter()
        .map(|p| {
            // Premultiplied over white: c + (255 - a).
            let a = 255 - u16::from(p.alpha());
            let ch = |c: u8| (u16::from(c) + a).min(255) as u8;
            [ch(p.red()), ch(p.green()), ch(p.blue())]
        })
        .collect()
}

fn luma(c: [u8; 3]) -> u8 {
    ((u32::from(c[0]) * 299 + u32::from(c[1]) * 587 + u32::from(c[2]) * 114 + 500) / 1000) as u8
}

/// The colour model "Determine automatically" picks: grayscale when every
/// pixel is (nearly) neutral.
pub fn auto_mode(pm: &Pixmap) -> ColorMode {
    let colourful = flatten(pm).iter().any(|c| {
        let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
        mx - mn > 3
    });
    if colourful {
        ColorMode::Rgb
    } else {
        ColorMode::Gray
    }
}

fn planes(pm: &Pixmap, mode: ColorMode) -> Planes {
    let px = flatten(pm);
    match mode {
        ColorMode::Auto => planes(pm, auto_mode(pm)),
        ColorMode::Rgb => Planes::Rgb(px.iter().flatten().copied().collect()),
        ColorMode::Gray => Planes::Gray(px.iter().map(|&c| luma(c)).collect()),
        ColorMode::Mono => Planes::Mono(px.iter().map(|&c| u8::from(luma(c) >= 128)).collect()),
        ColorMode::Cmyk => Planes::Cmyk(
            px.iter()
                .flat_map(|c| {
                    // The simple inverse of the display conversion (no ICC).
                    let k = 255 - c[0].max(c[1]).max(c[2]);
                    let d = 255 - u16::from(k);
                    let comp = |v: u8| {
                        ((d - u16::from(v).min(d)) * 255)
                            .checked_div(d)
                            .unwrap_or(0) as u8
                    };
                    [comp(c[0]), comp(c[1]), comp(c[2]), k]
                })
                .collect(),
        ),
    }
}

/// Encode a rendered page.
pub fn encode(pm: &Pixmap, s: &Settings) -> Result<Vec<u8>> {
    let (w, h) = (pm.width(), pm.height());
    let dpi = if s.dpi.is_finite() && s.dpi > 0.0 {
        s.dpi
    } else {
        72.0
    };
    let p = planes(pm, s.color);
    match s.format {
        Format::Png(filter) => png(&p, w, h, dpi, filter),
        Format::Jpeg(q, mode) => jpeg(&p, w, h, dpi, q, mode),
        Format::Tiff(c) => tiff(&p, w, h, dpi, c),
    }
}

fn pack_bits(mono: &[u8], w: u32, h: u32) -> Vec<u8> {
    let row = (w as usize).div_ceil(8);
    let mut out = vec![0u8; row * h as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            if mono.get(y * w as usize + x).copied().unwrap_or(1) != 0
                && let Some(b) = out.get_mut(y * row + x / 8)
            {
                *b |= 0x80 >> (x % 8);
            }
        }
    }
    out
}

fn png(p: &Planes, w: u32, h: u32, dpi: f64, filter: PngFilter) -> Result<Vec<u8>> {
    let (color, depth, data) = match p {
        Planes::Rgb(d) => (png::ColorType::Rgb, png::BitDepth::Eight, d.clone()),
        Planes::Gray(d) => (png::ColorType::Grayscale, png::BitDepth::Eight, d.clone()),
        Planes::Mono(d) => (
            png::ColorType::Grayscale,
            png::BitDepth::One,
            pack_bits(d, w, h),
        ),
        Planes::Cmyk(_) => {
            return Err(Error::Invalid(
                "PNG cannot hold CMYK; choose RGB, grayscale or monochrome".into(),
            ));
        }
    };
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(color);
        enc.set_depth(depth);
        enc.set_filter(match filter {
            PngFilter::None => png::Filter::NoFilter,
            PngFilter::Sub => png::Filter::Sub,
            PngFilter::Up => png::Filter::Up,
            PngFilter::Average => png::Filter::Avg,
            PngFilter::Paeth => png::Filter::Paeth,
            PngFilter::Adaptive => png::Filter::Adaptive,
        });
        let ppm = (dpi / 0.0254).round() as u32;
        enc.set_pixel_dims(Some(png::PixelDimensions {
            xppu: ppm,
            yppu: ppm,
            unit: png::Unit::Meter,
        }));
        let mut wr = enc
            .write_header()
            .map_err(|e| Error::Invalid(format!("PNG: {e}")))?;
        wr.write_image_data(&data)
            .map_err(|e| Error::Invalid(format!("PNG: {e}")))?;
    }
    Ok(out)
}

fn jpeg(p: &Planes, w: u32, h: u32, dpi: f64, q: Quality, mode: JpegMode) -> Result<Vec<u8>> {
    let (Ok(w16), Ok(h16)) = (u16::try_from(w), u16::try_from(h)) else {
        return Err(Error::Limit(
            "JPEG images are limited to 65535 pixels a side; lower the resolution".into(),
        ));
    };
    let gray;
    let (data, ct): (&[u8], jpeg_encoder::ColorType) = match p {
        Planes::Rgb(d) => (d, jpeg_encoder::ColorType::Rgb),
        Planes::Gray(d) => (d, jpeg_encoder::ColorType::Luma),
        Planes::Cmyk(d) => (d, jpeg_encoder::ColorType::Cmyk),
        Planes::Mono(d) => {
            // JPEG has no 1-bit mode: monochrome is written as black-and-white gray.
            gray = d
                .iter()
                .map(|&b| if b != 0 { 255 } else { 0 })
                .collect::<Vec<u8>>();
            (&gray, jpeg_encoder::ColorType::Luma)
        }
    };
    let mut out = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut out, q.value());
    let d = dpi.round().clamp(1.0, 65_535.0) as u16;
    enc.set_density(jpeg_encoder::Density::Inch { x: d, y: d });
    match mode {
        JpegMode::Baseline => {}
        JpegMode::Optimized => enc.set_optimized_huffman_tables(true),
        JpegMode::Progressive(scans) => {
            enc.set_progressive(true);
            enc.set_progressive_scans(scans.clamp(2, 64));
        }
    }
    enc.encode(data, w16, h16, ct)
        .map_err(|e| Error::Invalid(format!("JPEG: {e}")))?;
    Ok(out)
}

/// TIFF LZW (MSB-first codes, 9–12 bits, code width grows one code early).
pub fn lzw_encode(data: &[u8]) -> Vec<u8> {
    use std::collections::HashMap;
    let mut out = Vec::with_capacity(data.len() / 2 + 16);
    let mut bitbuf: u32 = 0;
    let mut nbits: u32 = 0;
    let mut put = |code: u32, width: u32, out: &mut Vec<u8>| {
        bitbuf = (bitbuf << width) | code;
        nbits += width;
        while nbits >= 8 {
            out.push((bitbuf >> (nbits - 8)) as u8);
            nbits -= 8;
            bitbuf &= (1 << nbits) - 1;
        }
    };
    let mut dict: HashMap<(u32, u8), u32> = HashMap::new();
    let mut next = 258u32;
    let mut width = 9u32;
    put(256, width, &mut out);
    let mut iter = data.iter();
    let Some(&first) = iter.next() else {
        put(257, width, &mut out);
        if nbits > 0 {
            out.push((bitbuf << (8 - nbits)) as u8);
        }
        return out;
    };
    let mut cur = u32::from(first);
    for &b in iter {
        if let Some(&code) = dict.get(&(cur, b)) {
            cur = code;
            continue;
        }
        put(cur, width, &mut out);
        dict.insert((cur, b), next);
        next += 1;
        // Early change: widen when the next code would need it.
        if next + 1 > (1 << width) && width < 12 {
            width += 1;
        }
        if next >= 4094 {
            put(256, width, &mut out);
            dict.clear();
            next = 258;
            width = 9;
        }
        cur = u32::from(b);
    }
    put(cur, width, &mut out);
    next += 1;
    if next + 1 > (1 << width) && width < 12 {
        width += 1;
    }
    put(257, width, &mut out);
    if nbits > 0 {
        out.push((bitbuf << (8 - nbits)) as u8);
    }
    out
}

fn tiff(p: &Planes, w: u32, h: u32, dpi: f64, c: TiffCompression) -> Result<Vec<u8>> {
    // (samples per pixel, bits per sample, photometric, raw strip)
    let (spp, bps, photometric, raw): (u16, u16, u16, Vec<u8>) = match p {
        Planes::Rgb(d) => (3, 8, 2, d.clone()),
        Planes::Gray(d) => (1, 8, 1, d.clone()),
        Planes::Cmyk(d) => (4, 8, 5, d.clone()),
        Planes::Mono(d) => (1, 1, 1, pack_bits(d, w, h)),
    };
    let (code, strip) = match c {
        TiffCompression::None => (1u16, raw),
        TiffCompression::Lzw => (5, lzw_encode(&raw)),
        TiffCompression::Zip => (8, folio_core::filters::flate_encode(&raw)),
        TiffCompression::CcittG4 => {
            let Planes::Mono(d) = p else {
                return Err(Error::Invalid(
                    "CCITT Group 4 compresses monochrome images only; choose monochrome".into(),
                ));
            };
            let mut enc = fax::encoder::Encoder::new(fax::VecWriter::new());
            for row in d.chunks(w.max(1) as usize).take(h as usize) {
                let pels = row.iter().map(|&v| {
                    if v != 0 {
                        fax::Color::White
                    } else {
                        fax::Color::Black
                    }
                });
                enc.encode_line(pels, w)
                    .map_err(|_| Error::Invalid("CCITT encoding failed".into()))?;
            }
            let data = enc
                .finish()
                .map_err(|_| Error::Invalid("CCITT encoding failed".into()))?
                .finish();
            (4, data)
        }
    };
    // CCITT and "WhiteIsZero"-free bilevel: 0 = black with photometric 1.
    let mut entries: Vec<(u16, u16, u32, u32)> = Vec::new(); // tag, type, count, value
    let mut extra: Vec<u8> = Vec::new();
    let header_len = 8u32;
    let n_tags = 13u32 + u32::from(spp == 4);
    let ifd_len = 2 + n_tags * 12 + 4;
    let extra_base = header_len + ifd_len;
    let add_extra = |bytes: &[u8], extra: &mut Vec<u8>| -> u32 {
        let off = extra_base + extra.len() as u32;
        extra.extend_from_slice(bytes);
        if extra.len() % 2 == 1 {
            extra.push(0);
        }
        off
    };
    let bps_value = if spp > 2 {
        let v: Vec<u8> = (0..spp).flat_map(|_| bps.to_le_bytes()).collect();
        add_extra(&v, &mut extra)
    } else {
        u32::from(bps)
    };
    let res = (dpi * 100.0).round().clamp(1.0, f64::from(u32::MAX)) as u32;
    let mut rational = res.to_le_bytes().to_vec();
    rational.extend_from_slice(&100u32.to_le_bytes());
    let xres = add_extra(&rational, &mut extra);
    let yres = add_extra(&rational, &mut extra);
    let strip_off = extra_base + extra.len() as u32;
    let strip_len = u32::try_from(strip.len())
        .map_err(|_| Error::Limit("the TIFF image is larger than 4 GB".into()))?;
    entries.push((254, 4, 1, 0)); // NewSubfileType
    entries.push((256, 4, 1, w)); // ImageWidth
    entries.push((257, 4, 1, h)); // ImageLength
    entries.push((258, 3, u32::from(spp), bps_value)); // BitsPerSample
    entries.push((259, 3, 1, u32::from(code))); // Compression
    entries.push((262, 3, 1, u32::from(photometric))); // Photometric
    entries.push((273, 4, 1, strip_off)); // StripOffsets
    entries.push((277, 3, 1, u32::from(spp))); // SamplesPerPixel
    entries.push((278, 4, 1, h)); // RowsPerStrip
    entries.push((279, 4, 1, strip_len)); // StripByteCounts
    entries.push((282, 5, 1, xres)); // XResolution
    entries.push((283, 5, 1, yres)); // YResolution
    entries.push((296, 3, 1, 2)); // ResolutionUnit: inch
    if spp == 4 {
        entries.push((332, 3, 1, 1)); // InkSet: CMYK
    }
    entries.sort_by_key(|e| e.0);
    let mut out = b"II*\0".to_vec();
    out.extend_from_slice(&header_len.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (tag, ty, count, value) in &entries {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&ty.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        if *ty == 3 && *count == 1 {
            out.extend_from_slice(&(*value as u16).to_le_bytes());
            out.extend_from_slice(&[0, 0]);
        } else {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&extra);
    out.extend_from_slice(&strip);
    Ok(out)
}

/// Margins around the non-white content of a rendered page, in pixels:
/// `[top, bottom, left, right]`, or `None` for a blank page. Used by
/// Crop Pages → "Remove white margins".
pub fn content_margins(pm: &Pixmap) -> Option<[u32; 4]> {
    let (w, h) = (pm.width(), pm.height());
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for (i, c) in flatten(pm).iter().enumerate() {
        if c.iter().any(|&v| v < 250) {
            let (x, y) = (
                (i % w.max(1) as usize) as u32,
                (i / w.max(1) as usize) as u32,
            );
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
    }
    (x0 < x1 && y0 < y1).then(|| [y0, h - y1, x0, w - x1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lzw_roundtrip_through_the_pdf_decoder() {
        // TIFF LZW and PDF LZW (EarlyChange 1) are the same code.
        for data in [
            b"".to_vec(),
            b"-----A---B".to_vec(),
            (0..20_000u32)
                .map(|i| (i * 7 % 251) as u8)
                .collect::<Vec<u8>>(),
            vec![0u8; 50_000],
        ] {
            let enc = lzw_encode(&data);
            let dec = folio_core::filters::lzw_decode(&enc, true).unwrap();
            assert_eq!(dec, data);
        }
    }
}
