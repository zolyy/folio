//! Generated sample documents for tests and the stage-1 anti-crash gate.
//! Everything is built by code (AGENTS.md §1): no downloaded files.

use folio_core::document::rect;
use folio_core::filters::flate_encode;
use folio_core::text_string::encode_text;
use folio_core::{Dict, Document, ObjRef, Object, Result, Stream};

fn stream(dict: Dict, data: &[u8], compress: bool) -> Object {
    let mut dict = dict;
    let data = if compress {
        dict.set("Filter", Object::name("FlateDecode"));
        flate_encode(data)
    } else {
        data.to_vec()
    };
    Object::Stream(Stream { dict, data })
}

fn page(doc: &mut Document, size: [f64; 2], content: &[u8], res: Dict) -> ObjRef {
    let c = doc.add(stream(Dict::new(), content, true));
    let mut p = Dict::new();
    p.set("Type", Object::name("Page"));
    p.set("MediaBox", rect([0.0, 0.0, size[0], size[1]]));
    p.set("Resources", res.into());
    p.set("Contents", c.into());
    doc.add(p.into())
}

/// Three pages exercising the text paths, plus bookmarks (one through a
/// named destination), page labels `i, ii, A-1` and document properties.
///
/// Page 1: Helvetica/WinAnsi, kerning in `TJ`, a second line.
/// Page 2: a Type0 font with a ToUnicode CMap and `/W` widths.
/// Page 3: /Differences glyph names, an inline image, a Form XObject.
pub fn rich() -> Result<Document> {
    let mut doc = Document::new();

    let mut helv = Dict::new();
    helv.set("Type", Object::name("Font"));
    helv.set("Subtype", Object::name("Type1"));
    helv.set("BaseFont", Object::name("Helvetica"));
    helv.set("Encoding", Object::name("WinAnsiEncoding"));
    let helv = doc.add(helv.into());

    let mut f1 = Dict::new();
    f1.set("F1", helv.into());
    let mut res1 = Dict::new();
    res1.set("Font", f1.clone().into());
    let p1 = page(
        &mut doc,
        [612.0, 792.0],
        b"BT /F1 12 Tf 72 720 Td (Hello Folio) Tj 0 -16 Td [(W) 120 (orld) -600 (again)] TJ ET",
        res1,
    );

    // Type0 font, Identity-H, with ToUnicode.
    let cmap = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
        1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
        2 beginbfchar <0001> <0218> <0002> <0074> endbfchar\n\
        1 beginbfrange <0003> <0005> <0061> endbfrange\n\
        endcmap CMapName currentdict /CMap defineresource pop end end";
    let tu = doc.add(stream(Dict::new(), cmap, true));
    let mut cid = Dict::new();
    cid.set("Type", Object::name("Font"));
    cid.set("Subtype", Object::name("CIDFontType2"));
    cid.set("BaseFont", Object::name("FolioSans"));
    cid.set("DW", Object::Int(1000));
    cid.set(
        "W",
        Object::Array(vec![
            Object::Int(1),
            Object::Array(vec![Object::Int(600), Object::Int(500)]),
            Object::Int(3),
            Object::Int(5),
            Object::Int(500),
        ]),
    );
    let cid = doc.add(cid.into());
    let mut t0 = Dict::new();
    t0.set("Type", Object::name("Font"));
    t0.set("Subtype", Object::name("Type0"));
    t0.set("BaseFont", Object::name("FolioSans"));
    t0.set("Encoding", Object::name("Identity-H"));
    t0.set("DescendantFonts", Object::Array(vec![cid.into()]));
    t0.set("ToUnicode", tu.into());
    let t0 = doc.add(t0.into());
    let mut f2 = Dict::new();
    f2.set("F2", t0.into());
    let mut res2 = Dict::new();
    res2.set("Font", f2.into());
    let p2 = page(
        &mut doc,
        [842.0, 595.0],
        b"BT /F2 14 Tf 72 500 Td <00010002000300040005> Tj ET",
        res2,
    );

    // Differences + inline image + form XObject.
    let mut enc = Dict::new();
    enc.set("Type", Object::name("Encoding"));
    enc.set(
        "Differences",
        Object::Array(vec![
            Object::Int(65),
            Object::name("scommaaccent"),
            Object::name("tcommaaccent"),
            Object::name("uni0103"),
        ]),
    );
    let mut diff = Dict::new();
    diff.set("Type", Object::name("Font"));
    diff.set("Subtype", Object::name("Type1"));
    diff.set("BaseFont", Object::name("Times-Roman"));
    diff.set("Encoding", enc.into());
    let diff = doc.add(diff.into());
    let mut form_res = Dict::new();
    form_res.set("Font", f1.into());
    let mut fd = Dict::new();
    fd.set("Type", Object::name("XObject"));
    fd.set("Subtype", Object::name("Form"));
    fd.set("BBox", rect([0.0, 0.0, 300.0, 100.0]));
    fd.set(
        "Matrix",
        Object::Array(vec![
            Object::Int(1),
            Object::Int(0),
            Object::Int(0),
            Object::Int(1),
            Object::Int(72),
            Object::Int(300),
        ]),
    );
    fd.set("Resources", form_res.into());
    let form = doc.add(stream(
        fd,
        b"BT /F1 10 Tf 0 0 Td (Inside form) Tj ET",
        false,
    ));
    let mut f3 = Dict::new();
    f3.set("F3", diff.into());
    let mut xo = Dict::new();
    xo.set("Fm1", form.into());
    let mut res3 = Dict::new();
    res3.set("Font", f3.into());
    res3.set("XObject", xo.into());
    let p3 = page(
        &mut doc,
        [612.0, 792.0],
        b"BT /F3 12 Tf 72 700 Td (ABC) Tj ET\nq 10 0 0 10 72 600 cm BI /W 2 /H 1 /CS /G /BPC 8 ID \x00\xff EI Q\n/Fm1 Do",
        res3,
    );

    doc.set_pages(&[p1, p2, p3])?;

    // Bookmarks: "Chapter 1" (explicit) and "Chapter 2" (named destination).
    let root = doc.add(Object::Null);
    let b1 = doc.add(Object::Null);
    let b2 = doc.add(Object::Null);
    let mut d1 = Dict::new();
    d1.set("Title", Object::Str(encode_text("Chapter 1")));
    d1.set("Parent", root.into());
    d1.set("Next", b2.into());
    d1.set("Dest", Object::Array(vec![p1.into(), Object::name("Fit")]));
    let mut d2 = Dict::new();
    d2.set("Title", Object::Str(encode_text("Capitolul 2 — ș")));
    d2.set("Parent", root.into());
    d2.set("Prev", b1.into());
    d2.set("Dest", Object::Str(b"chap2".to_vec()));
    let mut r = Dict::new();
    r.set("Type", Object::name("Outlines"));
    r.set("First", b1.into());
    r.set("Last", b2.into());
    r.set("Count", Object::Int(2));
    doc.objects.insert(root.num, r.into());
    doc.objects.insert(b1.num, d1.into());
    doc.objects.insert(b2.num, d2.into());

    let mut dests = Dict::new();
    dests.set(
        "Names",
        Object::Array(vec![
            Object::Str(b"chap2".to_vec()),
            Object::Array(vec![p3.into(), Object::name("Fit")]),
        ]),
    );
    let dests = doc.add(dests.into());
    let mut names = Dict::new();
    names.set("Dests", dests.into());

    let mut lab0 = Dict::new();
    lab0.set("S", Object::name("r"));
    let mut lab2 = Dict::new();
    lab2.set("S", Object::name("D"));
    lab2.set("P", Object::Str(b"A-".to_vec()));
    let mut labels = Dict::new();
    labels.set(
        "Nums",
        Object::Array(vec![
            Object::Int(0),
            lab0.into(),
            Object::Int(2),
            lab2.into(),
        ]),
    );

    let cat = doc.catalog_mut()?;
    cat.set("Outlines", root.into());
    cat.set("Names", names.into());
    cat.set("PageLabels", labels.into());

    let info = doc.info_mut()?;
    info.set("Title", Object::Str(b"Rich sample".to_vec()));
    info.set("Author", Object::Str(encode_text("Gáll Zoltán")));
    info.set("Producer", Object::Str(b"Folio samples".to_vec()));
    info.set(
        "CreationDate",
        Object::Str(b"D:20261013120000+03'00'".to_vec()),
    );
    Ok(doc)
}

/// A plain document with `n` pages showing "Page 1", "Page 2", ...
pub fn numbered(n: usize) -> Result<Document> {
    let mut doc = Document::new();
    for i in 1..=n {
        doc.add_text_page(&format!("Page {i}"))?;
    }
    Ok(doc)
}
