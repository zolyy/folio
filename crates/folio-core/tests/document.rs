#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use folio_core::crashgen::{core_samples, xref_stream_sample};
use folio_core::{Document, Object};

#[test]
fn written_documents_read_back() {
    let mut doc = Document::new();
    doc.add_text_page("one").unwrap();
    doc.add_text_page("two").unwrap();
    let bytes = doc.to_bytes().unwrap();
    let back = Document::from_bytes(&bytes).unwrap();
    assert_eq!(back.pages().unwrap().len(), 2);
    assert!(back.repairs.is_empty(), "{:?}", back.repairs);
}

#[test]
fn xref_streams_object_streams_and_png_predictor() {
    let doc = Document::from_bytes(&xref_stream_sample()).unwrap();
    assert!(doc.repairs.is_empty(), "{:?}", doc.repairs);
    let pages = doc.pages().unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(doc.media_box(pages[0]), [0.0, 0.0, 300.0, 300.0]);
}

#[test]
fn incremental_updates_use_the_newest_objects() {
    let samples = core_samples();
    let doc = Document::from_bytes(&samples[0]).unwrap();
    let title = doc.info().unwrap().get(b"Title").unwrap();
    assert_eq!(title, &Object::Str(b"Updated".to_vec()));
    assert_eq!(doc.pages().unwrap().len(), 3);
}

#[test]
fn broken_startxref_is_rebuilt_by_scanning() {
    let mut doc = Document::new();
    doc.add_text_page("survivor").unwrap();
    let mut bytes = doc.to_bytes().unwrap();
    let at = bytes.windows(9).rposition(|w| w == b"startxref").unwrap();
    bytes.truncate(at);
    bytes.extend_from_slice(b"startxref\n12\n%%EOF\n");
    let back = Document::from_bytes(&bytes).unwrap();
    assert_eq!(back.pages().unwrap().len(), 1);
    assert!(!back.repairs.is_empty());
}

#[test]
fn cyclic_page_tree_does_not_loop() {
    let src = b"%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [2 0 R 3 0 R] /Count 1 >> endobj\n3 0 obj << /Type /Page /Parent 2 0 R >> endobj\ntrailer << /Root 1 0 R >>\n";
    let doc = Document::from_bytes(src).unwrap();
    assert_eq!(doc.pages().unwrap().len(), 1);
}

#[test]
fn not_a_pdf_is_a_clear_error() {
    let err = Document::from_bytes(b"hello, I am a text file").unwrap_err();
    assert!(err.to_string().contains("not a PDF"), "{err}");
}

#[test]
fn encrypted_files_are_reported_not_misread() {
    let src = b"%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [] /Count 0 >> endobj\ntrailer << /Root 1 0 R /Encrypt << /Filter /Standard >> >>\n";
    let err = Document::from_bytes(src).unwrap_err();
    assert!(err.to_string().contains("encrypted"), "{err}");
}

#[test]
fn prune_removes_unreachable_objects() {
    let mut doc = Document::new();
    doc.add_text_page("x").unwrap();
    doc.add(Object::Int(42));
    assert!(doc.prune() >= 1);
}

#[test]
fn save_is_atomic_and_readable() {
    let dir = std::env::temp_dir().join(format!("folio-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("out.pdf");
    let mut doc = Document::new();
    doc.add_text_page("saved").unwrap();
    doc.save(&path).unwrap();
    assert_eq!(Document::load(&path).unwrap().pages().unwrap().len(), 1);
    let leftovers = std::fs::read_dir(&dir).unwrap().count();
    assert_eq!(leftovers, 1, "temporary file left behind");
    std::fs::remove_dir_all(&dir).unwrap();
}
