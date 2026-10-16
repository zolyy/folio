//! Generated test documents for rendering (no downloaded files, AGENTS.md §1).
//! Every element sits at a known place so tests can check pixels.

use folio_core::document::rect;
use folio_core::{Dict, Document, Object, Result, Stream};

/// Page 1 (612 × 792): every stage-3a feature at a known position.
/// Page 2 (200 × 100, rotated 90°): a red band on the left of the unrotated page.
pub fn vector() -> Result<Document> {
    let mut doc = Document::new();

    // Separation colour with a type-2 tint transform: 1.0 = (0, 0.5, 0).
    let mut f = Dict::new();
    f.set("FunctionType", Object::Int(2));
    f.set("Domain", nums(&[0.0, 1.0]));
    f.set("C0", nums(&[1.0, 1.0, 1.0]));
    f.set("C1", nums(&[0.0, 0.5, 0.0]));
    f.set("N", Object::Int(1));
    let sep = Object::Array(vec![
        Object::name("Separation"),
        Object::name("Spot"),
        Object::name("DeviceRGB"),
        f.into(),
    ]);
    let mut cs = Dict::new();
    cs.set("CS0", sep);

    // A PostScript-calculator DeviceN colour: (a, b) -> (a, b, 0).
    let mut ps = Dict::new();
    ps.set("FunctionType", Object::Int(4));
    ps.set("Domain", nums(&[0.0, 1.0, 0.0, 1.0]));
    ps.set("Range", nums(&[0.0, 1.0, 0.0, 1.0, 0.0, 1.0]));
    let ps = doc.add(Object::Stream(Stream {
        dict: ps,
        data: b"{ 0 }".to_vec(),
    }));
    cs.set(
        "CS1",
        Object::Array(vec![
            Object::name("DeviceN"),
            Object::Array(vec![Object::name("A"), Object::name("B")]),
            Object::name("DeviceRGB"),
            ps.into(),
        ]),
    );

    let mut gs0 = Dict::new();
    gs0.set("ca", Object::Real(0.5));
    let mut ext = Dict::new();
    ext.set("GS0", gs0.into());

    let mut fd = Dict::new();
    fd.set("Type", Object::name("XObject"));
    fd.set("Subtype", Object::name("Form"));
    fd.set("BBox", rect([0.0, 0.0, 40.0, 40.0]));
    fd.set("Matrix", nums(&[1.0, 0.0, 0.0, 1.0, 520.0, 500.0]));
    let form = doc.add(Object::Stream(Stream {
        dict: fd,
        data: b"1 0 1 rg -10 -10 100 100 re f".to_vec(),
    }));
    let mut xo = Dict::new();
    xo.set("Fm0", form.into());

    let mut res = Dict::new();
    res.set("ColorSpace", cs.into());
    res.set("ExtGState", ext.into());
    res.set("XObject", xo.into());

    let content = b"\
1 0 0 rg 72 72 200 100 re f
0 0 1 RG 10 w 300 700 m 500 700 l S
0 g 300 300 200 200 re 350 350 100 100 re f*
q 72 400 100 100 re W n 0 1 0 rg 0 0 612 792 re f Q
0 1 1 0 k 72 600 50 50 re f
/CS0 cs 1 scn 400 600 50 50 re f
/CS1 cs 1 0.5 scn 470 600 50 50 re f
q /GS0 gs 0 0 1 rg 500 72 50 50 re f Q
/Fm0 Do
0 G 2 w [6 6] 0 d 72 250 m 272 250 l S
";
    doc.add_page([612.0, 792.0], content, res)?;

    let p2 = doc.add_page([200.0, 100.0], b"1 0 0 rg 0 0 50 100 re f", Dict::new())?;
    if let Some(d) = doc.get_mut(p2).and_then(Object::as_dict_mut) {
        d.set("Rotate", Object::Int(90));
    }
    Ok(doc)
}

/// The same document with every stream stored uncompressed, so mutations
/// reach the content-stream syntax.
pub fn vector_uncompressed() -> Result<Document> {
    let mut d = vector()?;
    for o in d.objects.values_mut() {
        if let Object::Stream(s) = o
            && s.dict.get_name(b"Filter") == Some(b"FlateDecode")
            && let Ok(data) = folio_core::filters::decode_stream(s)
        {
            s.data = data;
            s.dict.remove(b"Filter");
        }
    }
    Ok(d)
}

fn nums(v: &[f64]) -> Object {
    Object::Array(v.iter().map(|&x| folio_core::document::num(x)).collect())
}

// ---- Text sample: one tiny synthetic font of every kind ----------------
//
// Glyph "A" is the square (100,0)-(600,700) in a 1000-unit em, glyph "B"
// (TrueType only) the triangle (100,0)-(350,700)-(600,0). At 100 pt the
// square's centre is 35 pt right of and 35 pt above the text origin.

fn be16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn be32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}

/// A minimal TrueType font: .notdef, A (square), B (triangle); cmap 3,1.
pub fn tiny_truetype() -> Vec<u8> {
    let simple = |pts: &[(i16, i16)]| -> Vec<u8> {
        let mut g = Vec::new();
        be16(&mut g, 1); // one contour
        for v in [100i16, 0, 600, 700] {
            be16(&mut g, v as u16);
        }
        be16(&mut g, (pts.len() - 1) as u16);
        be16(&mut g, 0); // no instructions
        g.extend(std::iter::repeat_n(0x01u8, pts.len())); // on-curve, 16-bit deltas
        let (mut px, mut py) = (0i16, 0i16);
        let mut ys = Vec::new();
        for &(x, y) in pts {
            be16(&mut g, (x - px) as u16);
            be16(&mut ys, (y - py) as u16);
            px = x;
            py = y;
        }
        g.extend_from_slice(&ys);
        if g.len() % 2 == 1 {
            g.push(0);
        }
        g
    };
    let glyphs = [
        Vec::new(),
        simple(&[(100, 0), (100, 700), (600, 700), (600, 0)]),
        simple(&[(100, 0), (350, 700), (600, 0)]),
    ];
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for g in &glyphs {
        be16(&mut loca, (glyf.len() / 2) as u16);
        glyf.extend_from_slice(g);
    }
    be16(&mut loca, (glyf.len() / 2) as u16);

    let mut head = Vec::new();
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0);
    be32(&mut head, 0x5F0F_3CF5);
    be16(&mut head, 0);
    be16(&mut head, 1000);
    head.extend_from_slice(&[0; 16]);
    for v in [0i16, -200, 1000, 800] {
        be16(&mut head, v as u16);
    }
    be16(&mut head, 0);
    be16(&mut head, 8);
    be16(&mut head, 2);
    be16(&mut head, 0); // short loca
    be16(&mut head, 0);

    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    for v in [800i16, -200, 0] {
        be16(&mut hhea, v as u16);
    }
    be16(&mut hhea, 700);
    for v in [0i16, 0, 600, 1, 0, 0, 0, 0, 0, 0, 0] {
        be16(&mut hhea, v as u16);
    }
    be16(&mut hhea, 3);

    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0000_5000);
    be16(&mut maxp, 3);

    let mut hmtx = Vec::new();
    for _ in 0..3 {
        be16(&mut hmtx, 700);
        be16(&mut hmtx, 100);
    }

    let mut cmap = Vec::new();
    be16(&mut cmap, 0);
    be16(&mut cmap, 1);
    be16(&mut cmap, 3);
    be16(&mut cmap, 1);
    be32(&mut cmap, 12);
    // format 4: A..B -> glyphs 1..2
    let seg: [(u16, u16, u16); 2] = [(0x41, 0x42, 1u16.wrapping_sub(0x41)), (0xFFFF, 0xFFFF, 1)];
    be16(&mut cmap, 4);
    be16(&mut cmap, 16 + 8 * seg.len() as u16);
    be16(&mut cmap, 0);
    be16(&mut cmap, 2 * seg.len() as u16);
    be16(&mut cmap, 4);
    be16(&mut cmap, 1);
    be16(&mut cmap, 0);
    for s in &seg {
        be16(&mut cmap, s.1);
    }
    be16(&mut cmap, 0);
    for s in &seg {
        be16(&mut cmap, s.0);
    }
    for s in &seg {
        be16(&mut cmap, s.2);
    }
    for _ in &seg {
        be16(&mut cmap, 0);
    }

    let mut post = Vec::new();
    be32(&mut post, 0x0003_0000);
    post.extend_from_slice(&[0; 28]);

    let tables: [(&[u8; 4], Vec<u8>); 8] = [
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
        (b"post", post),
    ];
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    be16(&mut out, tables.len() as u16);
    be16(&mut out, 128);
    be16(&mut out, 3);
    be16(&mut out, 0);
    let mut offset = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, data) in &tables {
        out.extend_from_slice(*tag);
        be32(&mut out, 0);
        be32(&mut out, offset as u32);
        be32(&mut out, data.len() as u32);
        body.extend_from_slice(data);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        offset = 12 + 16 * tables.len() + body.len();
    }
    out.extend_from_slice(&body);
    out
}

fn type2_num(v: i32, out: &mut Vec<u8>) {
    match v {
        -107..=107 => out.push((v + 139) as u8),
        108..=1131 => {
            let w = v - 108;
            out.push((w / 256 + 247) as u8);
            out.push((w % 256) as u8);
        }
        -1131..=-108 => {
            let w = -v - 108;
            out.push((w / 256 + 251) as u8);
            out.push((w % 256) as u8);
        }
        _ => {
            out.push(28);
            out.extend_from_slice(&(v as i16).to_be_bytes());
        }
    }
}

fn cff_index(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    be16(&mut out, items.len() as u16);
    if items.is_empty() {
        return out;
    }
    out.push(4); // 4-byte offsets
    let mut off = 1u32;
    be32(&mut out, off);
    for it in items {
        off += it.len() as u32;
        be32(&mut out, off);
    }
    for it in items {
        out.extend_from_slice(it);
    }
    out
}

/// A minimal bare CFF font (as in FontFile3 /Type1C). With the ISOAdobe
/// charset, glyph 34 is "A".
pub fn tiny_cff() -> Vec<u8> {
    let mut glyphs: Vec<Vec<u8>> = (0..34).map(|_| vec![14u8]).collect();
    let mut a = Vec::new();
    for v in [700, 100, 0] {
        type2_num(v, &mut a);
    }
    a.push(21); // width + rmoveto
    for (dx, dy) in [(0, 700), (500, 0), (0, -700)] {
        type2_num(dx, &mut a);
        type2_num(dy, &mut a);
        a.push(5);
    }
    a.push(14);
    glyphs.push(a);
    let charstrings = cff_index(&glyphs);
    let private = vec![139u8, 20]; // defaultWidthX 0
    let int5 = |v: i32| {
        let mut b = vec![29u8];
        b.extend_from_slice(&v.to_be_bytes());
        b
    };
    // Fixed-size operands, so the offsets can be computed first.
    let header = [1u8, 0, 4, 4];
    let name = cff_index(&[b"FolioCFF".to_vec()]);
    let top_len = 5 + 1 + 5 + 5 + 1;
    let top_index_len = 2 + 1 + 8 + top_len;
    let strings = cff_index(&[]);
    let gsubrs = cff_index(&[]);
    let cs_off = header.len() + name.len() + top_index_len + strings.len() + gsubrs.len();
    let priv_off = cs_off + charstrings.len();
    let mut top = int5(cs_off as i32);
    top.push(17);
    top.extend(int5(private.len() as i32));
    top.extend(int5(priv_off as i32));
    top.push(18);
    let mut out = header.to_vec();
    out.extend(name);
    out.extend(cff_index(&[top]));
    out.extend(strings);
    out.extend(gsubrs);
    out.extend(charstrings);
    out.extend(private);
    out
}

fn type1_num(v: i32, out: &mut Vec<u8>) {
    match v {
        -107..=107 => out.push((v + 139) as u8),
        108..=1131 => {
            let w = v - 108;
            out.push((w / 256 + 247) as u8);
            out.push((w % 256) as u8);
        }
        -1131..=-108 => {
            let w = -v - 108;
            out.push((w / 256 + 251) as u8);
            out.push((w % 256) as u8);
        }
        _ => {
            out.push(255);
            out.extend_from_slice(&v.to_be_bytes());
        }
    }
}

/// A minimal Type 1 font program (PFA, eexec-encrypted) with glyph "A".
pub fn tiny_type1() -> Vec<u8> {
    use crate::font::type1::encrypt;
    let mut a = Vec::new();
    type1_num(0, &mut a);
    type1_num(700, &mut a);
    a.push(13); // hsbw
    type1_num(100, &mut a);
    type1_num(0, &mut a);
    a.push(21); // rmoveto
    for (dx, dy) in [(0, 700), (500, 0), (0, -700)] {
        type1_num(dx, &mut a);
        type1_num(dy, &mut a);
        a.push(5);
    }
    a.push(9);
    a.push(14);
    let notdef = vec![139u8, 139, 13, 14];
    let cs = |plain: &[u8]| {
        let mut p = vec![0u8; 4]; // lenIV 4
        p.extend_from_slice(plain);
        encrypt(&p, 4330)
    };
    let (a, n) = (cs(&a), cs(&notdef));
    // Laid out as the Type 1 specification shows, so strict readers accept it.
    let mut private = b"\0\0\0\0dup /Private 12 dict dup begin\n/RD{string currentfile exch readstring pop}executeonly def\n/ND{noaccess def}executeonly def\n/NP{noaccess put}executeonly def\n/BlueValues [] def\n/MinFeature {16 16} def\n/password 5839 def\n/lenIV 4 def\n/Subrs 0 array\n/CharStrings 2 dict dup begin\n".to_vec();
    private.extend_from_slice(format!("/.notdef {} RD ", n.len()).as_bytes());
    private.extend_from_slice(&n);
    private.extend_from_slice(format!(" ND\n/A {} RD ", a.len()).as_bytes());
    private.extend_from_slice(&a);
    private.extend_from_slice(
        b" ND\nend\nend\nreadonly put\nnoaccess put\ndup /FontName get exch definefont pop\nmark currentfile closefile\n",
    );
    let mut out = b"%!PS-AdobeFont-1.0: FolioT1 001.000\n12 dict begin\n/FontInfo 2 dict dup begin\n/version (001.000) readonly def\n/FullName (Folio T1) readonly def\nend readonly def\n/FontName /FolioT1 def\n/PaintType 0 def\n/FontType 1 def\n/FontMatrix [0.001 0 0 0.001 0 0] readonly def\n/Encoding StandardEncoding def\n/FontBBox {0 0 700 700} readonly def\ncurrentdict end\ncurrentfile eexec\n".to_vec();
    out.extend(encrypt(&private, 55665));
    out
}

fn font_file(doc: &mut Document, key: &str, data: Vec<u8>, subtype: Option<&str>) -> Dict {
    let mut sd = Dict::new();
    if let Some(s) = subtype {
        sd.set("Subtype", Object::name(s));
    }
    let file = doc.add(Object::Stream(Stream { dict: sd, data }));
    let mut desc = Dict::new();
    desc.set("Type", Object::name("FontDescriptor"));
    desc.set("FontName", Object::name("FolioTest"));
    desc.set("Flags", Object::Int(32));
    // The entries ISO 32000-2 Table 120 requires, so strict readers accept it.
    desc.set("FontBBox", rect([0.0, 0.0, 700.0, 700.0]));
    desc.set("ItalicAngle", Object::Int(0));
    desc.set("Ascent", Object::Int(700));
    desc.set("Descent", Object::Int(0));
    desc.set("CapHeight", Object::Int(700));
    desc.set("StemV", Object::Int(80));
    desc.set(key, file.into());
    desc
}

fn simple_font(
    doc: &mut Document,
    subtype: &str,
    desc: Dict,
    widths: &[f64],
) -> folio_core::ObjRef {
    let desc = doc.add(desc.into());
    let mut f = Dict::new();
    f.set("Type", Object::name("Font"));
    f.set("Subtype", Object::name(subtype));
    f.set("BaseFont", Object::name("FolioTest"));
    f.set("FirstChar", Object::Int(65));
    f.set("LastChar", Object::Int(64 + widths.len() as i64));
    f.set("Widths", nums(widths));
    f.set("Encoding", Object::name("WinAnsiEncoding"));
    f.set("FontDescriptor", desc.into());
    doc.add(f.into())
}

/// One page drawing glyph "A" with each font kind, plus render modes.
/// Origins (pt): TrueType "AB" (50,650); CFF (50,500); Type 1 (50,350);
/// Type 3 (50,200); stroke mode (300,650); clip mode (300,500);
/// Type 0 / Identity-H (300,350).
pub fn text() -> Result<Document> {
    let mut doc = Document::new();
    let d = font_file(&mut doc, "FontFile2", tiny_truetype(), None);
    let tt = simple_font(&mut doc, "TrueType", d, &[700.0, 700.0]);
    let d = font_file(&mut doc, "FontFile3", tiny_cff(), Some("Type1C"));
    let cff = simple_font(&mut doc, "Type1", d, &[700.0]);
    let d = font_file(&mut doc, "FontFile", tiny_type1(), None);
    let t1 = simple_font(&mut doc, "Type1", d, &[700.0]);

    let proc_a = doc.add(Object::Stream(Stream {
        dict: Dict::new(),
        data: b"700 0 0 0 700 700 d1 0 0 700 700 re f".to_vec(),
    }));
    let mut procs = Dict::new();
    procs.set("A", proc_a.into());
    let mut enc = Dict::new();
    enc.set(
        "Differences",
        Object::Array(vec![Object::Int(65), Object::name("A")]),
    );
    let mut t3 = Dict::new();
    t3.set("Type", Object::name("Font"));
    t3.set("Subtype", Object::name("Type3"));
    t3.set("FontBBox", rect([0.0, 0.0, 700.0, 700.0]));
    t3.set("FontMatrix", nums(&[0.001, 0.0, 0.0, 0.001, 0.0, 0.0]));
    t3.set("CharProcs", procs.into());
    t3.set("Encoding", enc.into());
    t3.set("FirstChar", Object::Int(65));
    t3.set("LastChar", Object::Int(65));
    t3.set("Widths", nums(&[700.0]));
    let t3 = doc.add(t3.into());

    let d = font_file(&mut doc, "FontFile2", tiny_truetype(), None);
    let d = doc.add(d.into());
    let mut cid = Dict::new();
    cid.set("Type", Object::name("Font"));
    cid.set("Subtype", Object::name("CIDFontType2"));
    cid.set("BaseFont", Object::name("FolioTest"));
    cid.set("FontDescriptor", d.into());
    cid.set("CIDToGIDMap", Object::name("Identity"));
    cid.set("DW", Object::Int(700));
    let cid = doc.add(cid.into());
    let mut t0 = Dict::new();
    t0.set("Type", Object::name("Font"));
    t0.set("Subtype", Object::name("Type0"));
    t0.set("BaseFont", Object::name("FolioTest"));
    t0.set("Encoding", Object::name("Identity-H"));
    t0.set("DescendantFonts", Object::Array(vec![cid.into()]));
    let t0 = doc.add(t0.into());

    let mut fonts = Dict::new();
    for (k, r) in [("TT", tt), ("CF", cff), ("T1", t1), ("T3", t3), ("T0", t0)] {
        fonts.set(k, r.into());
    }
    let mut res = Dict::new();
    res.set("Font", fonts.into());
    let content = b"\
BT /TT 100 Tf 50 650 Td (AB) Tj ET
BT /CF 100 Tf 50 500 Td (A) Tj ET
BT /T1 100 Tf 50 350 Td (A) Tj ET
BT /T3 100 Tf 50 200 Td (A) Tj ET
BT /TT 100 Tf 1 Tr 300 650 Td (A) Tj ET
q BT /TT 100 Tf 7 Tr 300 500 Td (A) Tj ET 1 0 0 rg 0 0 612 792 re f Q
BT /T0 100 Tf 0 Tr 300 350 Td <0001> Tj ET
";
    doc.add_page([612.0, 792.0], content, res)?;
    Ok(doc)
}
