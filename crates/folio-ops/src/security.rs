//! Document Properties → Security, with Acrobat's Document Restrictions
//! Summary (docs/acrobat/protect.md).

use folio_core::Document;
use folio_core::crypt::{Permissions, Security};

use crate::json;

/// One row of the summary: label and value, as Acrobat words them.
pub type Row = (&'static str, String);

pub fn summary(doc: &Document) -> Vec<Row> {
    let Some(s) = &doc.security else {
        return vec![
            ("Security Method", "No Security".into()),
            ("Printing", "Allowed".into()),
            ("Changing the Document", "Allowed".into()),
            ("Document Assembly", "Allowed".into()),
            ("Content Copying", "Allowed".into()),
            ("Content Copying for Accessibility", "Allowed".into()),
            ("Page Extraction", "Allowed".into()),
            ("Commenting", "Allowed".into()),
            ("Filling of form fields", "Allowed".into()),
            ("Signing", "Allowed".into()),
            ("Creation of Template Pages", "Allowed".into()),
        ];
    };
    let p = s.permissions;
    let r3 = s.revision >= 3;
    let yes = |b: bool| if b { "Allowed" } else { "Not Allowed" }.to_string();
    let printing = if !p.bit(3) {
        "Not Allowed".into()
    } else if r3 && !p.bit(12) {
        "Low Resolution".into()
    } else if r3 {
        "High Resolution".into()
    } else {
        "Allowed".into()
    };
    vec![
        ("Security Method", "Password Security".into()),
        ("Encryption Level", s.method()),
        (
            "Document Open Password",
            if s.needs_open_password { "Yes" } else { "No" }.into(),
        ),
        (
            "Permissions Password",
            if restricted(s) { "Yes" } else { "No" }.into(),
        ),
        ("Printing", printing),
        ("Changing the Document", yes(p.bit(4))),
        ("Document Assembly", yes(p.bit(4) || (r3 && p.bit(11)))),
        ("Content Copying", yes(p.bit(5))),
        (
            "Content Copying for Accessibility",
            yes(p.bit(5) || (r3 && p.bit(10))),
        ),
        ("Page Extraction", yes(p.bit(4) && p.bit(5))),
        ("Commenting", yes(p.bit(6))),
        ("Filling of form fields", yes(p.bit(6) || (r3 && p.bit(9)))),
        ("Signing", yes(p.bit(6) || (r3 && p.bit(9)))),
        ("Creation of Template Pages", yes(p.bit(4))),
        (
            "Your access",
            if s.owner_access {
                "full (permissions password)".into()
            } else {
                "restricted to the rows above".into()
            },
        ),
    ]
}

/// Whether any Acrobat permission is withheld.
pub fn restricted(s: &Security) -> bool {
    let p: Permissions = s.permissions;
    let base = [3, 4, 5, 6];
    let extra: &[u32] = if s.revision >= 3 {
        &[9, 10, 11, 12]
    } else {
        &[]
    };
    base.iter().chain(extra).any(|&b| !p.bit(b))
}

pub fn to_text(rows: &[Row]) -> String {
    rows.iter()
        .map(|(k, v)| format!("{:<36}{v}\n", format!("{k}:")))
        .collect()
}

pub fn to_json(rows: &[Row]) -> String {
    let fields: Vec<(&str, String)> = rows.iter().map(|(k, v)| (*k, json::s(v))).collect();
    json::obj(&fields)
}
