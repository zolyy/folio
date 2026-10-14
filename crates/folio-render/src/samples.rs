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
