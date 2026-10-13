//! Document Properties → Description: Title, Author, Subject, Keywords, and
//! the read-only dates and producer. Editing keeps the XMP metadata stream
//! in step with the Info dictionary, as Acrobat does.

use folio_core::text_string::{decode_text, encode_text};
use folio_core::{Dict, Document, Object, Result, Stream};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    /// "Application" in Acrobat.
    pub creator: Option<String>,
    /// "PDF Producer" in Acrobat.
    pub producer: Option<String>,
    pub created: Option<String>,
    pub modified: Option<String>,
}

pub fn read(doc: &Document) -> Meta {
    let Some(info) = doc.info() else {
        return Meta::default();
    };
    let get = |k: &[u8]| {
        doc.resolve_key(info, k)
            .and_then(Object::as_str)
            .map(decode_text)
            .filter(|s| !s.is_empty())
    };
    Meta {
        title: get(b"Title"),
        author: get(b"Author"),
        subject: get(b"Subject"),
        keywords: get(b"Keywords"),
        creator: get(b"Creator"),
        producer: get(b"Producer"),
        created: get(b"CreationDate").map(|d| pretty_date(&d)),
        modified: get(b"ModDate").map(|d| pretty_date(&d)),
    }
}

/// The editable fields. `Some("")` clears a field; `None` leaves it alone.
#[derive(Clone, Debug, Default)]
pub struct Update {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
}

impl Update {
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.author.is_none()
            && self.subject.is_none()
            && self.keywords.is_none()
    }
}

pub fn update(doc: &mut Document, u: &Update, now: std::time::SystemTime) -> Result<()> {
    let (pdf_now, xmp_now) = dates(now);
    {
        let info = doc.info_mut()?;
        for (key, val) in [
            ("Title", &u.title),
            ("Author", &u.author),
            ("Subject", &u.subject),
            ("Keywords", &u.keywords),
        ] {
            match val.as_deref() {
                None => {}
                Some("") => {
                    info.remove(key.as_bytes());
                }
                Some(v) => info.set(key, Object::Str(encode_text(v))),
            }
        }
        info.set("ModDate", Object::Str(pdf_now.into_bytes()));
    }
    write_xmp(doc, &xmp_now)
}

/// Regenerate the XMP packet from the Info dictionary. PDF/A and PDF/UA
/// identification found in the old packet is kept.
fn write_xmp(doc: &mut Document, xmp_now: &str) -> Result<()> {
    let m = read(doc);
    let raw_info = doc.info().cloned().unwrap_or_default();
    let old = existing_xmp(doc).unwrap_or_default();
    let keep = |name: &str| attr_or_elem(&old, name);
    let created = raw_info
        .get(b"CreationDate")
        .and_then(Object::as_str)
        .map(decode_text)
        .and_then(|d| pdf_date_to_xmp(&d));

    let mut desc = String::new();
    let mut add = |s: String| {
        desc.push_str(&s);
        desc.push('\n');
    };
    if let Some(t) = &m.title {
        add(format!(
            "   <dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>",
            esc(t)
        ));
    }
    if let Some(a) = &m.author {
        add(format!(
            "   <dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>",
            esc(a)
        ));
    }
    if let Some(s) = &m.subject {
        add(format!(
            "   <dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:description>",
            esc(s)
        ));
    }
    if let Some(k) = &m.keywords {
        add(format!("   <pdf:Keywords>{}</pdf:Keywords>", esc(k)));
    }
    if let Some(p) = &m.producer {
        add(format!("   <pdf:Producer>{}</pdf:Producer>", esc(p)));
    }
    if let Some(c) = &m.creator {
        add(format!("   <xmp:CreatorTool>{}</xmp:CreatorTool>", esc(c)));
    }
    if let Some(c) = created {
        add(format!("   <xmp:CreateDate>{c}</xmp:CreateDate>"));
    }
    add(format!("   <xmp:ModifyDate>{xmp_now}</xmp:ModifyDate>"));
    add(format!("   <xmp:MetadataDate>{xmp_now}</xmp:MetadataDate>"));
    add("   <dc:format>application/pdf</dc:format>".into());
    for (ns, name) in [
        ("pdfaid", "part"),
        ("pdfaid", "conformance"),
        ("pdfaid", "amd"),
        ("pdfuaid", "part"),
    ] {
        if let Some(v) = keep(&format!("{ns}:{name}")) {
            add(format!("   <{ns}:{name}>{}</{ns}:{name}>", esc(&v)));
        }
    }
    let packet = format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
 <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
  <rdf:Description rdf:about=\"\"\n\
    xmlns:dc=\"http://purl.org/dc/elements/1.1/\"\n\
    xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\"\n\
    xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"\n\
    xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\"\n\
    xmlns:pdfuaid=\"http://www.aiim.org/pdfua/ns/id/\">\n\
{desc}  </rdf:Description>\n\
 </rdf:RDF>\n\
</x:xmpmeta>\n\
<?xpacket end=\"w\"?>"
    );
    let mut sd = Dict::new();
    sd.set("Type", Object::name("Metadata"));
    sd.set("Subtype", Object::name("XML"));
    let r = doc.add(Object::Stream(Stream {
        dict: sd,
        data: packet.into_bytes(),
    }));
    doc.catalog_mut()?.set("Metadata", r.into());
    Ok(())
}

/// The current XMP packet as text, if the document has a readable one.
pub fn existing_xmp(doc: &Document) -> Option<String> {
    let cat = doc.catalog().ok()?;
    let s = doc.resolve_key(cat, b"Metadata")?.as_stream()?;
    let bytes = folio_core::filters::decode_stream(s).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// `name="v"` or `<name>v</name>` in an XMP packet.
fn attr_or_elem(xml: &str, name: &str) -> Option<String> {
    if let Some(p) = xml.find(&format!("{name}=\"")) {
        let rest = xml.get(p + name.len() + 2..)?;
        return Some(rest.get(..rest.find('"')?)?.to_string());
    }
    let open = format!("<{name}>");
    let p = xml.find(&open)?;
    let rest = xml.get(p + open.len()..)?;
    Some(rest.get(..rest.find('<')?)?.trim().to_string())
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `(D:YYYYMMDDHHmmSSZ, YYYY-MM-DDTHH:MM:SSZ)` for `t`, in UTC.
pub fn dates(t: std::time::SystemTime) -> (String, String) {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, mo, d) = civil(secs / 86_400);
    let (h, mi, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    (
        format!("D:{y:04}{mo:02}{d:02}{h:02}{mi:02}{s:02}Z"),
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z"),
    )
}

fn civil(days: u64) -> (i64, u32, u32) {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// PDF date `D:YYYYMMDDHHmmSS+HH'mm'` → `YYYY-MM-DD HH:MM:SS +HH:MM`.
pub fn pretty_date(d: &str) -> String {
    let s = d.trim().trim_start_matches("D:");
    let digits: String = s.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() < 4 {
        return d.to_string();
    }
    let part = |a: usize, b: usize, def: &str| digits.get(a..b).unwrap_or(def).to_string();
    let mut out = format!(
        "{}-{}-{} {}:{}:{}",
        part(0, 4, "0000"),
        part(4, 6, "01"),
        part(6, 8, "01"),
        part(8, 10, "00"),
        part(10, 12, "00"),
        part(12, 14, "00")
    );
    let tz: String = s.chars().skip(digits.len()).collect();
    let tz = tz.replace('\'', "");
    if tz.starts_with('Z') {
        out.push_str(" UTC");
    } else if let Some(sign) = tz.chars().next().filter(|c| *c == '+' || *c == '-') {
        let hh = tz.get(1..3).unwrap_or("00");
        let mm = tz.get(3..5).unwrap_or("00");
        out.push_str(&format!(" {sign}{hh}:{mm}"));
    }
    out
}

fn pdf_date_to_xmp(d: &str) -> Option<String> {
    let p = pretty_date(d);
    if p == d {
        return None;
    }
    let (date, rest) = p.split_once(' ')?;
    let (time, tz) = rest.split_once(' ').unwrap_or((rest, ""));
    let tz = match tz {
        "UTC" | "" => "Z".to_string(),
        other => other.to_string(),
    };
    Some(format!("{date}T{time}{tz}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_dates_are_readable() {
        assert_eq!(
            pretty_date("D:20261013170412+03'00'"),
            "2026-10-13 17:04:12 +03:00"
        );
        assert_eq!(pretty_date("D:2026"), "2026-01-01 00:00:00");
        assert_eq!(pretty_date("garbage"), "garbage");
    }

    #[test]
    fn utc_dates_from_system_time() {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_791_911_052);
        let (pdf, xmp) = dates(t);
        assert_eq!(pdf, "D:20261013170412Z");
        assert_eq!(xmp, "2026-10-13T17:04:12Z");
    }
}
