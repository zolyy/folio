//! Stage 3e: Export PDF → Image. Every file is read back with an
//! independent decoder (png, zune-jpeg, tiff, hayro-ccitt).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use folio_render::export::{
    ColorMode, Format, JpegMode, PngFilter, Quality, Settings, TiffCompression, auto_mode, encode,
};
use folio_render::{Options, render_page, samples};
use tiny_skia::Pixmap;

/// The stage-3a vector page at 36 dpi (306 × 396): colour content.
fn colour_page() -> Pixmap {
    let doc = samples::vector().unwrap();
    let p = doc.pages().unwrap()[0];
    render_page(
        &doc,
        p,
        &Options {
            dpi: 36.0,
            ..Options::default()
        },
    )
    .unwrap()
    .pixmap
}

/// A gray-only page: black rectangle on white.
fn gray_page() -> Pixmap {
    let mut doc = folio_core::Document::new();
    doc.add_page(
        [100.0, 100.0],
        b"0 g 10 10 30 30 re f",
        folio_core::Dict::new(),
    )
    .unwrap();
    let p = doc.pages().unwrap()[0];
    render_page(&doc, p, &Options::default()).unwrap().pixmap
}

fn settings(format: Format, color: ColorMode) -> Settings {
    Settings {
        format,
        color,
        dpi: 36.0,
    }
}

fn png_info(bytes: &[u8]) -> (png::OutputInfo, Vec<u8>, Option<png::PixelDimensions>) {
    let dec = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut rd = dec.read_info().unwrap();
    let dims = rd.info().pixel_dims;
    let mut buf = vec![0; rd.output_buffer_size().unwrap()];
    let info = rd.next_frame(&mut buf).unwrap();
    (info, buf, dims)
}

#[test]
fn png_colour_spaces_filters_and_resolution() {
    let pm = colour_page();
    for filter in [
        PngFilter::None,
        PngFilter::Sub,
        PngFilter::Up,
        PngFilter::Average,
        PngFilter::Paeth,
        PngFilter::Adaptive,
    ] {
        let (info, buf, dims) =
            png_info(&encode(&pm, &settings(Format::Png(filter), ColorMode::Rgb)).unwrap());
        assert_eq!((info.width, info.height), (306, 396));
        assert_eq!(info.color_type, png::ColorType::Rgb);
        // Red rectangle of the sample: (172, 670) at 72 dpi, (86, 335) at 36 dpi.
        let i = (335 * 306 + 86) * 3;
        assert_eq!(&buf[i..i + 3], &[255, 0, 0], "{filter:?}");
        let d = dims.unwrap();
        assert_eq!(d.xppu, (36.0f64 / 0.0254).round() as u32);
    }
    let (info, ..) = png_info(
        &encode(
            &pm,
            &settings(Format::Png(PngFilter::Adaptive), ColorMode::Gray),
        )
        .unwrap(),
    );
    assert_eq!(info.color_type, png::ColorType::Grayscale);
    let (info, ..) = png_info(
        &encode(
            &pm,
            &settings(Format::Png(PngFilter::Adaptive), ColorMode::Mono),
        )
        .unwrap(),
    );
    assert_eq!(info.bit_depth, png::BitDepth::One);
    let e = encode(
        &pm,
        &settings(Format::Png(PngFilter::Adaptive), ColorMode::Cmyk),
    )
    .unwrap_err();
    assert!(e.to_string().contains("CMYK"), "{e}");
}

#[test]
fn determine_automatically_picks_gray_for_pages_without_colour() {
    assert_eq!(auto_mode(&gray_page()), ColorMode::Gray);
    assert_eq!(auto_mode(&colour_page()), ColorMode::Rgb);
    let (info, ..) = png_info(
        &encode(
            &gray_page(),
            &settings(Format::Png(PngFilter::Adaptive), ColorMode::Auto),
        )
        .unwrap(),
    );
    assert_eq!(info.color_type, png::ColorType::Grayscale);
}

#[test]
fn jpeg_quality_modes_and_colour_spaces() {
    let pm = colour_page();
    let mut sizes = Vec::new();
    for q in [
        Quality::Maximum,
        Quality::High,
        Quality::Medium,
        Quality::Low,
        Quality::Minimum,
    ] {
        let s = settings(Format::Jpeg(q, JpegMode::Baseline), ColorMode::Rgb);
        sizes.push(encode(&pm, &s).unwrap().len());
    }
    assert!(
        sizes.windows(2).all(|w| w[0] >= w[1]),
        "lower quality must not grow: {sizes:?}"
    );
    for (mode, marker) in [
        (JpegMode::Baseline, 0xC0u8),
        (JpegMode::Optimized, 0xC0),
        (JpegMode::Progressive(4), 0xC2),
    ] {
        let bytes = encode(
            &pm,
            &settings(Format::Jpeg(Quality::High, mode), ColorMode::Rgb),
        )
        .unwrap();
        assert!(
            bytes.windows(2).any(|w| w == [0xFF, marker]),
            "{mode:?}: frame marker"
        );
        let mut dec = jpeg_decoder::Decoder::new(&bytes[..]);
        let px = dec.decode().unwrap();
        let info = dec.info().unwrap();
        assert_eq!((info.width, info.height), (306, 396));
        let i = (335 * 306 + 86) * 3;
        assert!(
            px[i] > 220 && px[i + 1] < 40 && px[i + 2] < 40,
            "{mode:?}: {:?}",
            &px[i..i + 3]
        );
    }
    for (mode, fmt) in [
        (ColorMode::Cmyk, jpeg_decoder::PixelFormat::CMYK32),
        (ColorMode::Gray, jpeg_decoder::PixelFormat::L8),
    ] {
        let bytes = encode(
            &pm,
            &settings(Format::Jpeg(Quality::High, JpegMode::Baseline), mode),
        )
        .unwrap();
        let mut dec = jpeg_decoder::Decoder::new(&bytes[..]);
        dec.read_info().unwrap();
        assert_eq!(dec.info().unwrap().pixel_format, fmt);
    }
}

#[test]
fn tiff_none_lzw_zip_in_rgb_gray_and_cmyk() {
    use tiff::decoder::{Decoder, DecodingResult};
    let pm = colour_page();
    for c in [
        TiffCompression::None,
        TiffCompression::Lzw,
        TiffCompression::Zip,
    ] {
        for (mode, spp) in [
            (ColorMode::Rgb, 3usize),
            (ColorMode::Gray, 1),
            (ColorMode::Cmyk, 4),
        ] {
            let bytes = encode(&pm, &settings(Format::Tiff(c), mode)).unwrap();
            let mut dec = Decoder::new(std::io::Cursor::new(bytes)).unwrap();
            assert_eq!(dec.dimensions().unwrap(), (306, 396), "{c:?} {mode:?}");
            let DecodingResult::U8(px) = dec.read_image().unwrap() else {
                panic!("8-bit samples expected");
            };
            assert_eq!(px.len(), 306 * 396 * spp, "{c:?} {mode:?}");
            let i = (335 * 306 + 86) * spp;
            match mode {
                ColorMode::Rgb => assert_eq!(&px[i..i + 3], &[255, 0, 0]),
                ColorMode::Cmyk => assert_eq!(&px[i..i + 4], &[0, 255, 255, 0]),
                _ => assert!(px[i] < 128),
            }
        }
    }
}

#[test]
fn tiff_ccitt_group4_for_monochrome() {
    let pm = gray_page();
    let bytes = encode(
        &pm,
        &settings(Format::Tiff(TiffCompression::CcittG4), ColorMode::Mono),
    )
    .unwrap();
    // Find the strip through the StripOffsets/StripByteCounts tags.
    let u32_at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as usize;
    let u16_at = |o: usize| u16::from_le_bytes(bytes[o..o + 2].try_into().unwrap());
    let ifd = u32_at(4);
    let n = u16_at(ifd) as usize;
    let tag = |t: u16| {
        (0..n)
            .map(|k| ifd + 2 + 12 * k)
            .find(|&e| u16_at(e) == t)
            .map(|e| {
                if u16_at(e + 2) == 3 {
                    u16_at(e + 8) as usize
                } else {
                    u32_at(e + 8)
                }
            })
            .unwrap()
    };
    assert_eq!(tag(259), 4, "compression = CCITT T.6");
    let (off, len) = (tag(273), tag(279));
    struct Rows(Vec<Vec<bool>>);
    impl hayro_ccitt::Decoder for Rows {
        fn push_pixels(&mut self, white: bool, count: u32) {
            let row = self.0.last_mut().unwrap();
            row.extend(std::iter::repeat_n(!white, count as usize));
        }
        fn next_line(&mut self) {
            self.0.push(Vec::new());
        }
    }
    let settings = hayro_ccitt::DecodeSettings {
        columns: 100,
        rows: 100,
        end_of_block: true,
        end_of_line: false,
        rows_are_byte_aligned: false,
        encoding: hayro_ccitt::EncodingMode::Group4,
        invert_black: false,
    };
    let mut rows = Rows(vec![Vec::new()]);
    let mut ctx = hayro_ccitt::DecoderContext::new(settings);
    hayro_ccitt::decode(&bytes[off..off + len], &mut rows, &mut ctx).unwrap();
    // The black square spans x 10..40 from the top row 60 (100 - 40).
    assert!(rows.0[70][20], "inside the square is black");
    assert!(!rows.0[70][60], "outside is white");
    let e = encode(&colour_page(), &settings_g4_rgb()).unwrap_err();
    assert!(e.to_string().contains("monochrome"), "{e}");
}

fn settings_g4_rgb() -> Settings {
    settings(Format::Tiff(TiffCompression::CcittG4), ColorMode::Rgb)
}
