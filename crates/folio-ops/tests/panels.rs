//! Stage 4c: the Attachments panel and the Fonts tab of Document Properties.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use folio_core::text_string::encode_text;
use folio_core::{Dict, Document, Object, Stream};
use folio_ops::{attachments, fonts};

/// A document with one document-level attachment and one in a comment.
pub fn with_attachments() -> Document {
    let mut doc = Document::new();
    doc.add_text_page("Pagina cu atașamente").unwrap();
    let file = |doc: &mut Document, name: &str, body: &[u8], desc: &str| {
        let mut params = Dict::new();
        params.set("Size", Object::Int(body.len() as i64));
        params.set("ModDate", Object::Str(b"D:20261017120000Z".to_vec()));
        let mut sd = Dict::new();
        sd.set("Type", Object::name("EmbeddedFile"));
        sd.set("Params", params.into());
        let ef = doc.add(Object::Stream(Stream {
            dict: sd,
            data: body.to_vec(),
        }));
        let mut efd = Dict::new();
        efd.set("F", ef.into());
        let mut fs = Dict::new();
        fs.set("Type", Object::name("Filespec"));
        fs.set("F", Object::Str(name.as_bytes().to_vec()));
        fs.set("UF", Object::Str(encode_text(name)));
        fs.set("Desc", Object::Str(encode_text(desc)));
        fs.set("EF", efd.into());
        doc.add(fs.into())
    };
    let note = file(&mut doc, "nota.txt", b"continut atasat", "O notă");
    let data = file(&mut doc, "date.csv", b"a,b\n1,2\n", "Tabel");
    let mut tree = Dict::new();
    tree.set(
        "Names",
        Object::Array(vec![Object::Str(b"nota.txt".to_vec()), note.into()]),
    );
    let mut names = Dict::new();
    names.set("EmbeddedFiles", tree.into());
    doc.catalog_mut().unwrap().set("Names", names.into());
    let mut annot = Dict::new();
    annot.set("Type", Object::name("Annot"));
    annot.set("Subtype", Object::name("FileAttachment"));
    annot.set("Rect", folio_core::document::rect([10.0, 10.0, 30.0, 30.0]));
    annot.set("FS", data.into());
    let annot = doc.add(annot.into());
    let page = doc.pages().unwrap()[0];
    doc.get_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", Object::Array(vec![annot.into()]));
    doc
}

#[test]
fn attachments_panel_lists_document_and_comment_files() {
    let doc = with_attachments();
    let list = attachments::list(&doc);
    assert_eq!(list.len(), 2, "{list:?}");
    assert_eq!(list[0].name, "nota.txt");
    assert_eq!(list[0].description.as_deref(), Some("O notă"));
    assert_eq!(list[0].size, Some(15));
    assert_eq!(list[0].page, None);
    assert_eq!(list[1].name, "date.csv");
    assert_eq!(list[1].page, Some(0));
    assert_eq!(
        attachments::contents(&doc, &list[1]).unwrap(),
        b"a,b\n1,2\n"
    );
}

#[test]
fn fonts_tab_lists_type_encoding_and_embedding() {
    let doc = with_attachments();
    let f = fonts::list(&doc);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0].name, "Helvetica");
    assert_eq!(f[0].kind, "Type 1");
    assert_eq!(f[0].encoding.as_deref(), Some("Ansi"));
    assert!(!f[0].embedded && !f[0].subset);
}
