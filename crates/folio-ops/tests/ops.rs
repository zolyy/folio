//! Behaviour of each stage-1 feature, option by option, as listed in
//! docs/acrobat/organize.md. Test names are cited as parity evidence.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use folio_core::{Document, Object};
use folio_ops::labels::{self, LabelChange, Style};
use folio_ops::merge::{self, Input};
use folio_ops::pages::{self, Margins, Orientation, PageBox, Subset};
use folio_ops::split::{self, SplitBy};
use folio_ops::{compress, info, meta, outline, ranges, samples, text};

/// Reload through bytes, as a saved file would be.
fn roundtrip(doc: &Document) -> Document {
    Document::from_bytes(&doc.to_bytes().unwrap()).unwrap()
}

fn page_texts(doc: &Document) -> Vec<String> {
    doc.pages()
        .unwrap()
        .into_iter()
        .map(|p| text::page_text(doc, p).unwrap())
        .collect()
}

// ---- organize.rotate ----

#[test]
fn rotate_clockwise_counterclockwise_and_180_accumulate() {
    let mut doc = samples::numbered(2).unwrap();
    pages::rotate(&mut doc, &[0], 90).unwrap();
    pages::rotate(&mut doc, &[0], 90).unwrap();
    pages::rotate(&mut doc, &[1], -90).unwrap();
    let doc = roundtrip(&doc);
    let p = doc.pages().unwrap();
    assert_eq!(pages::rotation(&doc, p[0]), 180);
    assert_eq!(pages::rotation(&doc, p[1]), 270);
    assert!(pages::rotate(&mut doc.clone(), &[0], 45).is_err());
}

#[test]
fn rotate_even_odd_landscape_portrait_subsets() {
    let doc = samples::rich().unwrap(); // page 2 is landscape
    let all = [0, 1, 2];
    assert_eq!(
        pages::filter(&doc, &all, Subset::Odd, Orientation::Any).unwrap(),
        vec![0, 2]
    );
    assert_eq!(
        pages::filter(&doc, &all, Subset::Even, Orientation::Any).unwrap(),
        vec![1]
    );
    assert_eq!(
        pages::filter(&doc, &all, Subset::All, Orientation::Landscape).unwrap(),
        vec![1]
    );
    assert_eq!(
        pages::filter(&doc, &all, Subset::All, Orientation::Portrait).unwrap(),
        vec![0, 2]
    );
}

// ---- organize.delete / extract / reorder ----

#[test]
fn delete_pages_and_refuse_deleting_all() {
    let mut doc = samples::numbered(5).unwrap();
    pages::delete(&mut doc, &ranges::parse("2,4", 5).unwrap()).unwrap();
    let doc = roundtrip(&doc);
    assert_eq!(page_texts(&doc), vec!["Page 1", "Page 3", "Page 5"]);
    let err = pages::delete(&mut doc.clone(), &[0, 1, 2]).unwrap_err();
    assert!(err.to_string().contains("at least one page"));
}

#[test]
fn extract_keeps_order_and_drops_unused_objects() {
    let mut doc = samples::numbered(6).unwrap();
    let before = doc.objects.len();
    pages::select(&mut doc, &ranges::parse("5,2", 6).unwrap()).unwrap();
    assert!(doc.objects.len() < before);
    assert_eq!(page_texts(&roundtrip(&doc)), vec!["Page 5", "Page 2"]);
}

#[test]
fn reverse_and_reorder_every_page_once() {
    let mut doc = samples::numbered(3).unwrap();
    pages::reverse(&mut doc).unwrap();
    assert_eq!(page_texts(&doc), vec!["Page 3", "Page 2", "Page 1"]);
    pages::reorder(&mut doc, &[1, 2, 0]).unwrap();
    assert_eq!(page_texts(&doc), vec!["Page 2", "Page 1", "Page 3"]);
    assert!(pages::reorder(&mut doc, &[0, 0, 1]).is_err());
    assert!(pages::reorder(&mut doc, &[0, 1]).is_err());
}

// ---- organize.insert / replace ----

#[test]
fn insert_from_file_before_after_first_last() {
    let mut doc = samples::numbered(2).unwrap();
    let other = samples::numbered(3).unwrap();
    // After page 1, only pages 2-3 of the other file.
    pages::insert(&mut doc, other, Some(&[1, 2]), 1).unwrap();
    let doc = roundtrip(&doc);
    assert_eq!(
        page_texts(&doc),
        vec!["Page 1", "Page 2", "Page 3", "Page 2"]
    );
    let mut first = samples::numbered(1).unwrap();
    pages::insert(&mut first, samples::numbered(1).unwrap(), None, 0).unwrap();
    assert_eq!(first.pages().unwrap().len(), 2);
    assert!(pages::insert(&mut first, samples::numbered(1).unwrap(), None, 9).is_err());
}

#[test]
fn insert_blank_pages_match_the_neighbouring_page_size() {
    let mut doc = samples::rich().unwrap();
    pages::insert_blank(&mut doc, 2, 2).unwrap(); // after page 2 (landscape A4)
    let doc = roundtrip(&doc);
    let p = doc.pages().unwrap();
    assert_eq!(p.len(), 5);
    assert_eq!(doc.media_box(p[2]), [0.0, 0.0, 842.0, 595.0]);
    assert_eq!(doc.media_box(p[3]), [0.0, 0.0, 842.0, 595.0]);
}

#[test]
fn replace_pages_with_pages_of_another_file() {
    let mut doc = samples::numbered(4).unwrap();
    let mut other = Document::new();
    for t in ["X", "Y", "Z"] {
        other.add_text_page(t).unwrap();
    }
    pages::replace(&mut doc, &[1, 2], other.clone(), 1).unwrap();
    assert_eq!(
        page_texts(&roundtrip(&doc)),
        vec!["Page 1", "Y", "Z", "Page 4"]
    );
    assert!(pages::replace(&mut doc, &[0, 1, 2], other, 1).is_err());
}

// ---- organize.split ----

#[test]
fn split_by_number_of_pages() {
    let doc = samples::numbered(5).unwrap();
    let parts = split::split(&doc, SplitBy::Pages(2)).unwrap();
    let counts: Vec<usize> = parts.iter().map(|p| p.doc.pages().unwrap().len()).collect();
    assert_eq!(counts, vec![2, 2, 1]);
    assert_eq!(page_texts(&parts[2].doc), vec!["Page 5"]);
}

#[test]
fn split_by_top_level_bookmarks_including_named_destinations() {
    let doc = samples::rich().unwrap();
    let parts = split::split(&doc, SplitBy::Bookmarks).unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].title.as_deref(), Some("Chapter 1"));
    assert_eq!(parts[0].pages, vec![0, 1]);
    assert_eq!(parts[1].title.as_deref(), Some("Capitolul 2 — ș"));
    assert_eq!(parts[1].pages, vec![2]);
    assert!(split::split(&samples::numbered(2).unwrap(), SplitBy::Bookmarks).is_err());
}

#[test]
fn split_by_file_size_keeps_each_part_under_the_limit() {
    let doc = samples::numbered(30).unwrap();
    let whole = doc.to_bytes().unwrap().len() as u64;
    let limit = whole / 3;
    let parts = split::split(&doc, SplitBy::MaxBytes(limit)).unwrap();
    assert!(parts.len() >= 3, "{} parts", parts.len());
    for p in &parts {
        let size = p.doc.to_bytes().unwrap().len() as u64;
        assert!(size <= limit, "part of {size} bytes over {limit}");
    }
    let total: usize = parts.iter().map(|p| p.pages.len()).sum();
    assert_eq!(total, 30);
}

#[test]
fn split_file_names_are_safe() {
    assert_eq!(split::safe_file_name("a/b:c?"), "a_b_c_");
    assert_eq!(split::safe_file_name("  "), "Untitled");
}

// ---- organize.merge ----

#[test]
fn merge_combines_pages_in_order_with_ranges() {
    let a = samples::numbered(3).unwrap();
    let b = samples::numbered(2).unwrap();
    let out = merge::merge(
        vec![
            Input {
                doc: a,
                name: "a".into(),
                pages: Some(vec![2, 0]),
            },
            Input {
                doc: b,
                name: "b".into(),
                pages: None,
            },
        ],
        &merge::Options::default(),
    )
    .unwrap();
    let out = roundtrip(&out);
    assert_eq!(
        page_texts(&out),
        vec!["Page 3", "Page 1", "Page 1", "Page 2"]
    );
}

#[test]
fn merge_adds_a_bookmark_per_file_with_its_own_bookmarks_inside() {
    let out = merge::merge(
        vec![
            Input {
                doc: samples::numbered(1).unwrap(),
                name: "Intro".into(),
                pages: None,
            },
            Input {
                doc: samples::rich().unwrap(),
                name: "Raport anual".into(),
                pages: None,
            },
        ],
        &merge::Options::default(),
    )
    .unwrap();
    let out = roundtrip(&out);
    let top = outline::top_level(&out);
    let titles: Vec<&str> = top.iter().map(|b| b.title.as_str()).collect();
    assert_eq!(titles, vec!["Intro", "Raport anual"]);
    let pages = out.pages().unwrap();
    assert_eq!(top[1].page, Some(pages[1]));
    // The second file's bookmarks are nested and still reach their pages,
    // the named destination included.
    let child = out.get(top[1].item).unwrap().as_dict().unwrap();
    let first = child.get(b"First").unwrap().as_objref().unwrap();
    let kids = outline::siblings(&out, Some(first));
    assert_eq!(kids.len(), 2);
    let second = out.get(kids[1]).unwrap().as_dict().unwrap();
    let dest = second.get(b"Dest").unwrap();
    assert_eq!(outline::dest_page(&out, dest, 0), Some(pages[3]));
}

// ---- organize.crop ----

#[test]
fn crop_margins_respect_rotation_box_kind_and_reset() {
    let mut doc = samples::numbered(2).unwrap();
    pages::rotate(&mut doc, &[1], 90).unwrap();
    let m = Margins {
        top: 10.0,
        bottom: 20.0,
        left: 30.0,
        right: 40.0,
    };
    pages::crop(&mut doc, &[0, 1], PageBox::Crop, m).unwrap();
    pages::crop(&mut doc, &[0], PageBox::Trim, m).unwrap();
    let doc2 = roundtrip(&doc);
    let p = doc2.pages().unwrap();
    let cb = |i: usize, key: &[u8]| doc2.get(p[i]).unwrap().as_dict().unwrap().get(key).cloned();
    let nums = |o: Object| -> Vec<f64> {
        o.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap())
            .collect()
    };
    assert_eq!(
        nums(cb(0, b"CropBox").unwrap()),
        vec![30.0, 20.0, 572.0, 782.0]
    );
    // Rotated 90°: displayed top is the page's left side, and so on.
    assert_eq!(
        nums(cb(1, b"CropBox").unwrap()),
        vec![10.0, 30.0, 592.0, 752.0]
    );
    assert!(cb(0, b"TrimBox").is_some());
    assert_eq!(pages::reset_box(&mut doc, &[0], PageBox::Trim).unwrap(), 1);
    let huge = Margins {
        top: 500.0,
        bottom: 500.0,
        ..Margins::default()
    };
    assert!(pages::crop(&mut doc, &[0], PageBox::Crop, huge).is_err());
}

// ---- organize.page-labels ----

#[test]
fn page_labels_new_section_extend_and_survive_reordering() {
    let doc = samples::rich().unwrap();
    let got: Vec<String> = labels::per_page(&doc, 3)
        .unwrap()
        .iter()
        .map(|l| l.text())
        .collect();
    assert_eq!(got, vec!["i", "ii", "A-1"]);

    let mut doc = samples::numbered(6).unwrap();
    labels::apply(
        &mut doc,
        0,
        1,
        &LabelChange::NewSection {
            style: Style::RomanLower,
            prefix: String::new(),
            start: 1,
        },
    )
    .unwrap();
    labels::apply(
        &mut doc,
        2,
        3,
        &LabelChange::NewSection {
            style: Style::AlphaUpper,
            prefix: "Ax-".into(),
            start: 3,
        },
    )
    .unwrap();
    labels::apply(&mut doc, 4, 4, &LabelChange::Extend).unwrap();
    let doc = roundtrip(&doc);
    let got: Vec<String> = labels::per_page(&doc, 6)
        .unwrap()
        .iter()
        .map(|l| l.text())
        .collect();
    assert_eq!(got, vec!["i", "ii", "Ax-C", "Ax-D", "Ax-E", "6"]);

    let mut moved = doc.clone();
    pages::select(&mut moved, &[5, 0]).unwrap();
    let got: Vec<String> = labels::per_page(&moved, 2)
        .unwrap()
        .iter()
        .map(|l| l.text())
        .collect();
    assert_eq!(got, vec!["6", "i"]);
}

// ---- view.properties / edit.metadata ----

#[test]
fn properties_show_acrobat_description_and_advanced_fields() {
    let doc = samples::rich().unwrap();
    let i = info::info(&doc, Some(12_345)).unwrap();
    assert_eq!(i.meta.title.as_deref(), Some("Rich sample"));
    assert_eq!(i.meta.author.as_deref(), Some("Gáll Zoltán"));
    assert_eq!(
        i.meta.created.as_deref(),
        Some("2026-10-13 12:00:00 +03:00")
    );
    assert_eq!(i.page_count, 3);
    assert_eq!(i.pages[1].width, 842.0);
    assert_eq!(i.pages[0].label.as_deref(), Some("i"));
    assert!(!i.tagged);
    let t = i.to_text();
    assert!(t.contains("Page Size:     8.50 x 11.00 in (Letter)"), "{t}");
    assert!(i.to_json().contains("\"page_count\":3"));
}

#[test]
fn metadata_edit_updates_info_and_xmp() {
    let mut doc = samples::rich().unwrap();
    let u = meta::Update {
        title: Some("Raport — 2026".into()),
        author: Some(String::new()),
        subject: None,
        keywords: Some("pdf, folio".into()),
    };
    meta::update(&mut doc, &u, std::time::SystemTime::now()).unwrap();
    let doc = roundtrip(&doc);
    let m = meta::read(&doc);
    assert_eq!(m.title.as_deref(), Some("Raport — 2026"));
    assert_eq!(m.author, None);
    assert_eq!(m.keywords.as_deref(), Some("pdf, folio"));
    assert!(m.modified.is_some());
    let xmp = meta::existing_xmp(&doc).unwrap();
    assert!(xmp.contains("Raport — 2026"), "{xmp}");
    assert!(xmp.contains("<pdf:Keywords>pdf, folio</pdf:Keywords>"));
    assert!(!xmp.contains("Gáll"));
}

// ---- optimize.compress ----

#[test]
fn reduce_file_size_compresses_streams_and_drops_garbage() {
    let mut doc = samples::numbered(3).unwrap();
    let big = "lorem ipsum dolor sit amet ".repeat(400);
    let r = doc.add(Object::Stream(folio_core::Stream {
        dict: folio_core::Dict::new(),
        data: big.clone().into_bytes(),
    }));
    // Make it reachable so it is kept, then measure.
    doc.info_mut().unwrap().set("Blob", r.into());
    doc.add(Object::Str(b"unused".to_vec()));
    let before = doc.to_bytes().unwrap().len();
    let report = compress::compress(&mut doc);
    let after = doc.to_bytes().unwrap().len();
    assert!(report.streams_compressed >= 1);
    assert!(report.objects_removed >= 1);
    assert!(after < before, "{after} >= {before}");
    let doc = roundtrip(&doc);
    assert_eq!(page_texts(&doc), vec!["Page 1", "Page 2", "Page 3"]);
}

// ---- export.text ----

#[test]
fn text_extraction_handles_kerning_type0_differences_and_forms() {
    let doc = samples::rich().unwrap();
    let t = page_texts(&doc);
    assert_eq!(t[0], "Hello Folio\nWorld again");
    assert_eq!(t[1], "Ștabc");
    assert_eq!(t[2], "șță\nInside form");
}
