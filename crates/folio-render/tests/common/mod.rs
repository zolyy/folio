//! Image test documents, built by code with test-only encoders.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]

use folio_core::{Dict, Document, Object, Stream};

pub fn image_stream(dict: &[(&str, Object)], data: Vec<u8>) -> Object {
    let mut d = Dict::new();
    d.set("Type", Object::name("XObject"));
    d.set("Subtype", Object::name("Image"));
    for (k, v) in dict {
        d.set(*k, v.clone());
    }
    Object::Stream(Stream { dict: d, data })
}

pub fn int(v: i64) -> Object {
    Object::Int(v)
}

pub fn name(v: &str) -> Object {
    Object::name(v)
}

pub fn jpeg(rgb: &[u8], w: u16, h: u16, gray: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let enc = jpeg_encoder::Encoder::new(&mut out, 95);
    let ct = if gray {
        jpeg_encoder::ColorType::Luma
    } else {
        jpeg_encoder::ColorType::Rgb
    };
    enc.encode(rgb, w, h, ct).unwrap();
    out
}

/// CCITT Group 4 data for rows given as `true` = black.
pub fn g4(rows: &[Vec<bool>]) -> Vec<u8> {
    let width = rows[0].len() as u32;
    let mut enc = fax::encoder::Encoder::new(fax::VecWriter::new());
    for r in rows {
        let pels = r.iter().map(|&b| {
            if b {
                fax::Color::Black
            } else {
                fax::Color::White
            }
        });
        enc.encode_line(pels, width).unwrap();
    }
    enc.finish().unwrap().finish()
}

/// One page with every image kind at a known place (see tests/images.rs).
pub fn image_sample() -> Document {
    let mut doc = Document::new();
    let mut xo = Dict::new();
    let mut add = |doc: &mut Document, key: &str, o: Object| {
        let r = doc.add(o);
        xo.set(key, r.into());
    };
    let rgb8 = [
        ("Width", int(2)),
        ("Height", int(2)),
        ("ColorSpace", name("DeviceRGB")),
        ("BitsPerComponent", int(8)),
    ];
    add(
        &mut doc,
        "Raw",
        image_stream(&rgb8, vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]),
    );
    add(
        &mut doc,
        "Bits",
        image_stream(
            &[
                ("Width", int(8)),
                ("Height", int(1)),
                ("ColorSpace", name("DeviceGray")),
                ("BitsPerComponent", int(1)),
                ("Decode", Object::Array(vec![int(1), int(0)])),
            ],
            vec![0b1010_1010],
        ),
    );
    add(
        &mut doc,
        "Idx",
        image_stream(
            &[
                ("Width", int(2)),
                ("Height", int(1)),
                (
                    "ColorSpace",
                    Object::Array(vec![
                        name("Indexed"),
                        name("DeviceRGB"),
                        int(1),
                        Object::Str(vec![255, 0, 0, 0, 0, 255]),
                    ]),
                ),
                ("BitsPerComponent", int(4)),
            ],
            vec![0x01],
        ),
    );
    add(
        &mut doc,
        "G16",
        image_stream(
            &[
                ("Width", int(1)),
                ("Height", int(1)),
                ("ColorSpace", name("DeviceGray")),
                ("BitsPerComponent", int(16)),
            ],
            vec![0x80, 0x00],
        ),
    );
    add(
        &mut doc,
        "Stencil",
        image_stream(
            &[
                ("Width", int(2)),
                ("Height", int(2)),
                ("ImageMask", Object::Bool(true)),
            ],
            vec![0b0100_0000, 0b1000_0000],
        ),
    );
    let solid: Vec<u8> = (0..16 * 16).flat_map(|_| [200u8, 50, 50]).collect();
    add(
        &mut doc,
        "Jpg",
        image_stream(
            &[
                ("Width", int(16)),
                ("Height", int(16)),
                ("ColorSpace", name("DeviceRGB")),
                ("BitsPerComponent", int(8)),
                ("Filter", name("DCTDecode")),
            ],
            jpeg(&solid, 16, 16, false),
        ),
    );
    add(
        &mut doc,
        "JpgGray",
        image_stream(
            &[
                ("Width", int(8)),
                ("Height", int(8)),
                ("ColorSpace", name("DeviceGray")),
                ("BitsPerComponent", int(8)),
                ("Filter", name("DCTDecode")),
            ],
            jpeg(&[100u8; 64], 8, 8, true),
        ),
    );
    let smask = doc.add(image_stream(
        &[
            ("Width", int(1)),
            ("Height", int(1)),
            ("ColorSpace", name("DeviceGray")),
            ("BitsPerComponent", int(8)),
        ],
        vec![128],
    ));
    add(
        &mut doc,
        "Soft",
        image_stream(
            &[
                ("Width", int(1)),
                ("Height", int(1)),
                ("ColorSpace", name("DeviceRGB")),
                ("BitsPerComponent", int(8)),
                ("SMask", smask.into()),
            ],
            vec![0, 0, 255],
        ),
    );
    add(
        &mut doc,
        "Key",
        image_stream(
            &[
                ("Width", int(2)),
                ("Height", int(1)),
                ("ColorSpace", name("DeviceRGB")),
                ("BitsPerComponent", int(8)),
                (
                    "Mask",
                    Object::Array(vec![int(0), int(0), int(255), int(255), int(0), int(0)]),
                ),
            ],
            vec![255, 0, 0, 0, 255, 0],
        ),
    );
    let mut row0 = vec![true; 8];
    row0.extend(vec![false; 8]);
    let mut parms = Dict::new();
    parms.set("K", int(-1));
    parms.set("Columns", int(16));
    parms.set("Rows", int(2));
    add(
        &mut doc,
        "Fax",
        image_stream(
            &[
                ("Width", int(16)),
                ("Height", int(2)),
                ("ColorSpace", name("DeviceGray")),
                ("BitsPerComponent", int(1)),
                ("Filter", name("CCITTFaxDecode")),
                ("DecodeParms", parms.into()),
            ],
            g4(&[row0, vec![false; 16]]),
        ),
    );

    let mut res = Dict::new();
    res.set("XObject", xo.into());
    let mut content = b"\
q 100 0 0 100 50 600 cm /Raw Do Q
q 80 0 0 10 200 600 cm /Bits Do Q
q 100 0 0 50 300 600 cm /Idx Do Q
q 50 0 0 50 420 600 cm /G16 Do Q
q 0 1 0 rg 100 0 0 100 50 400 cm /Stencil Do Q
q 100 0 0 100 200 400 cm /Jpg Do Q
q 50 0 0 50 320 400 cm /JpgGray Do Q
q 50 0 0 50 400 450 cm /Soft Do Q
q 100 0 0 50 470 450 cm /Key Do Q
q 160 0 0 20 50 300 cm /Fax Do Q
q 100 0 0 50 300 300 cm BI /W 2 /H 1 /CS /RGB /BPC 8 ID "
        .to_vec();
    content.extend_from_slice(&[255, 0, 255, 0, 255, 255]);
    content.extend_from_slice(
        b" EI Q\nq 50 0 0 50 420 300 cm BI /W 1 /H 1 /CS /G /BPC 8 /F /AHx ID 80> EI Q\n",
    );
    doc.add_page([612.0, 792.0], &content, res).unwrap();
    doc
}
