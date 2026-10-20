//! Stage 4c engine: layers (optional content) and Replace Document Colors.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use folio_core::{Dict, Document, ObjRef, Object};
use folio_render::{Options, Rendered, ReplaceColors, oc, render_page};

/// 100 × 50 page: a red square in layer "Rosu" (on, locked) and a blue
/// square in layer "Albastru" (off by default).
fn layered() -> (Document, ObjRef, ObjRef) {
    let mut doc = Document::new();
    let ocg = |doc: &mut Document, name: &str| {
        let mut d = Dict::new();
        d.set("Type", Object::name("OCG"));
        d.set("Name", Object::Str(name.as_bytes().to_vec()));
        doc.add(d.into())
    };
    let red = ocg(&mut doc, "Rosu");
    let blue = ocg(&mut doc, "Albastru");
    let mut config = Dict::new();
    config.set("Order", Object::Array(vec![red.into(), blue.into()]));
    config.set("OFF", Object::Array(vec![blue.into()]));
    config.set("Locked", Object::Array(vec![red.into()]));
    let mut ocp = Dict::new();
    ocp.set("OCGs", Object::Array(vec![red.into(), blue.into()]));
    ocp.set("D", config.into());
    doc.catalog_mut().unwrap().set("OCProperties", ocp.into());
    let mut props = Dict::new();
    props.set("P1", red.into());
    props.set("P2", blue.into());
    let mut res = Dict::new();
    res.set("Properties", props.into());
    doc.add_page(
        [100.0, 50.0],
        b"/OC /P1 BDC 1 0 0 rg 10 10 30 30 re f EMC /OC /P2 BDC 0 0 1 rg 60 10 30 30 re f EMC",
        res,
    )
    .unwrap();
    (doc, red, blue)
}

fn render(doc: &Document, opts: Options) -> Rendered {
    let p = doc.pages().unwrap()[0];
    render_page(doc, p, &opts).unwrap()
}

fn rgb(r: &Rendered, x: u32, y: u32) -> [u8; 3] {
    let [a, b, c, _] = r.rgba(x, y).unwrap();
    [a, b, c]
}

#[test]
fn layers_panel_lists_order_default_state_and_locks() {
    let (doc, red, blue) = layered();
    let l = oc::layers(&doc);
    let names: Vec<&str> = l.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names, vec!["Rosu", "Albastru"]);
    assert_eq!(l[0].ocg, Some(red));
    assert!(l[0].on_by_default && l[0].locked);
    assert_eq!(l[1].ocg, Some(blue));
    assert!(!l[1].on_by_default && !l[1].locked);
}

#[test]
fn hidden_layers_are_not_drawn() {
    let (doc, red, _blue) = layered();
    // The document's default: red on, blue off.
    let r = render(&doc, Options::default());
    assert_eq!(rgb(&r, 25, 25), [255, 0, 0]);
    assert_eq!(rgb(&r, 75, 25), [255, 255, 255]);
    // Hide red only: blue becomes visible.
    let r = render(
        &doc,
        Options {
            hidden_layers: Some(vec![red.num].into()),
            ..Options::default()
        },
    );
    assert_eq!(rgb(&r, 25, 25), [255, 255, 255]);
    assert_eq!(rgb(&r, 75, 25), [0, 0, 255]);
}

fn line_art() -> Document {
    let mut doc = Document::new();
    doc.add_page(
        [100.0, 50.0],
        b"0 g 10 10 30 30 re f 1 0 0 rg 60 10 30 30 re f",
        Dict::new(),
    )
    .unwrap();
    doc
}

#[test]
fn replace_document_colors_background_and_line_art() {
    let doc = line_art();
    let green_on_black = ReplaceColors::HIGH_CONTRAST[0].1;
    // Text only: the background changes, line art keeps its colours.
    let r = render(
        &doc,
        Options {
            replace_colors: Some(green_on_black),
            ..Options::default()
        },
    );
    assert_eq!(rgb(&r, 5, 5), [0, 0, 0]);
    assert_eq!(rgb(&r, 25, 25), [0, 0, 0]);
    assert_eq!(rgb(&r, 75, 25), [255, 0, 0]);
    // "Change the color of line art as well as text".
    let r = render(
        &doc,
        Options {
            replace_colors: Some(ReplaceColors {
                line_art: true,
                ..green_on_black
            }),
            ..Options::default()
        },
    );
    assert_eq!(rgb(&r, 25, 25), [0, 255, 0]);
    assert_eq!(rgb(&r, 75, 25), [0, 255, 0]);
    // "Only change the color of black text or line art".
    let r = render(
        &doc,
        Options {
            replace_colors: Some(ReplaceColors {
                line_art: true,
                only_black: true,
                ..green_on_black
            }),
            ..Options::default()
        },
    );
    assert_eq!(rgb(&r, 25, 25), [0, 255, 0]);
    assert_eq!(rgb(&r, 75, 25), [255, 0, 0]);
}

#[test]
fn hostile_optional_content_is_bounded() {
    let mut doc = Document::new();
    // An OCMD whose visibility expression nests far too deep.
    let mut ve = Object::Array(vec![Object::name("Not"), Object::Null]);
    for _ in 0..500 {
        ve = Object::Array(vec![Object::name("Not"), ve]);
    }
    let mut md = Dict::new();
    md.set("Type", Object::name("OCMD"));
    md.set("VE", ve);
    let md = doc.add(md.into());
    let mut props = Dict::new();
    props.set("M", md.into());
    let mut res = Dict::new();
    res.set("Properties", props.into());
    let mut content = b"/OC /M BDC ".repeat(5000);
    content.extend_from_slice(b"0 g 0 0 10 10 re f");
    content.extend_from_slice(&b" EMC".repeat(9000));
    doc.add_page([50.0, 50.0], &content, res).unwrap();
    let r = render(&doc, Options::default());
    assert_eq!(r.pixmap.width(), 50);
    assert!(oc::layers(&doc).is_empty());
}
