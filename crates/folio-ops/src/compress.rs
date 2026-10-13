//! Reduce File Size (the part that needs no rendering): compress streams
//! stored uncompressed, recompress plain Flate streams when that is smaller,
//! and drop objects nothing refers to. Image down-sampling, font subsetting
//! and object streams arrive in stage 12.

use folio_core::filters::{decode_stream, flate_encode};
use folio_core::{Document, Object};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub streams_compressed: usize,
    pub objects_removed: usize,
}

pub fn compress(doc: &mut Document) -> Report {
    let mut report = Report {
        objects_removed: doc.prune(),
        ..Report::default()
    };
    for obj in doc.objects.values_mut() {
        let Object::Stream(s) = obj else { continue };
        // XMP stays readable as plain text (Acrobat leaves it uncompressed too).
        if s.dict.get_name(b"Type") == Some(b"Metadata") {
            continue;
        }
        let filter = s.dict.get(b"Filter");
        let plain_flate = matches!(filter, Some(Object::Name(n)) if n == b"FlateDecode")
            && !s.dict.contains(b"DecodeParms");
        let raw = match filter {
            None => s.data.clone(),
            Some(_) if plain_flate => match decode_stream(s) {
                Ok(d) => d,
                Err(_) => continue,
            },
            Some(_) => continue,
        };
        let packed = flate_encode(&raw);
        if packed.len() < s.data.len() {
            s.data = packed;
            s.dict.set("Filter", Object::name("FlateDecode"));
            s.dict.remove(b"DecodeParms");
            report.streams_compressed += 1;
        }
    }
    report
}
