//! End-to-end: `folio layers`, `attachments`, `fonts`, `render --hide-layers`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::Command;

use folio_core::text_string::encode_text;
use folio_core::{Dict, Document, Object, Stream};

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("folio-panels-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn folio(args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_folio"))
        .args(args)
        .output()
        .unwrap();
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), s)
}

/// Two layers: "Rosu" (on) and "Albastru" (off), plus one attachment.
fn sample() -> Document {
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
    let mut ocp = Dict::new();
    ocp.set("OCGs", Object::Array(vec![red.into(), blue.into()]));
    ocp.set("D", config.into());
    let mut sd = Dict::new();
    sd.set("Type", Object::name("EmbeddedFile"));
    let ef = doc.add(Object::Stream(Stream {
        dict: sd,
        data: b"salut".to_vec(),
    }));
    let mut efd = Dict::new();
    efd.set("F", ef.into());
    let mut fs = Dict::new();
    fs.set("Type", Object::name("Filespec"));
    fs.set("UF", Object::Str(encode_text("salut.txt")));
    fs.set("EF", efd.into());
    let fs = doc.add(fs.into());
    let mut tree = Dict::new();
    tree.set(
        "Names",
        Object::Array(vec![Object::Str(b"salut.txt".to_vec()), fs.into()]),
    );
    let mut names = Dict::new();
    names.set("EmbeddedFiles", tree.into());
    let cat = doc.catalog_mut().unwrap();
    cat.set("OCProperties", ocp.into());
    cat.set("Names", names.into());
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
    doc
}

#[test]
fn cli_layers_attachments_and_fonts() {
    let d = dir("all");
    let pdf = d.join("doc.pdf");
    sample().save(&pdf).unwrap();
    let p = pdf.to_str().unwrap();

    let (code, out) = folio(&["layers", p]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Rosu") && out.contains("Albastru"), "{out}");

    let (code, out) = folio(&["--json", "attachments", p]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("salut.txt"), "{out}");
    let saved = d.join("got.txt");
    let (code, out) = folio(&[
        "attachments",
        p,
        "--save",
        "1",
        "-o",
        saved.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(std::fs::read(&saved).unwrap(), b"salut");
    let (code, _) = folio(&[
        "attachments",
        p,
        "--save",
        "1",
        "-o",
        saved.to_str().unwrap(),
    ]);
    assert_eq!(
        code, 2,
        "an existing file is not overwritten without --force"
    );

    let (code, out) = folio(&["fonts", p]);
    assert_eq!(code, 0, "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn cli_render_shows_and_hides_layers_by_name() {
    let d = dir("render");
    let pdf = d.join("doc.pdf");
    sample().save(&pdf).unwrap();
    let p = pdf.to_str().unwrap();
    let png = d.join("out.png");
    let (code, out) = folio(&[
        "render",
        p,
        "--dpi",
        "72",
        "--hide-layers",
        "Rosu",
        "--show-layers",
        "Albastru",
        "-o",
        png.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{out}");
    let img = tiny_skia::Pixmap::load_png(&png).unwrap();
    let at = |x: u32, y: u32| {
        let c = img.pixel(x, y).unwrap().demultiply();
        [c.red(), c.green(), c.blue()]
    };
    assert_eq!(at(25, 25), [255, 255, 255]);
    assert_eq!(at(75, 25), [0, 0, 255]);
    // An unknown layer name lists the real ones.
    let (code, out) = folio(&["render", p, "--hide-layers", "Verde", "-o", "x.png"]);
    assert_eq!(code, 2);
    assert!(out.contains("Rosu") && out.contains("Albastru"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}
