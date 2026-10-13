//! Password security, option by option (docs/acrobat/protect.md).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use folio_core::crypt::{Changes, Compat, Need, Printing, Scope, Settings};
use folio_core::{Document, Error, Object};

const ALL: [Compat; 5] = [
    Compat::Acrobat3,
    Compat::Acrobat5,
    Compat::Acrobat6,
    Compat::Acrobat7,
    Compat::AcrobatX,
];

fn sample() -> Document {
    let mut d = Document::new();
    d.add_text_page("Secret page (one)").unwrap();
    d.add_text_page("Page two").unwrap();
    d.info_mut()
        .unwrap()
        .set("Title", Object::Str(b"Confidential report".to_vec()));
    d
}

fn title(d: &Document) -> Vec<u8> {
    d.info()
        .unwrap()
        .get(b"Title")
        .unwrap()
        .as_str()
        .unwrap()
        .to_vec()
}

fn content(d: &Document, page: usize) -> Vec<u8> {
    let p = d.pages().unwrap()[page];
    let c = d
        .get(p)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Contents")
        .unwrap();
    let s = d.resolve(c).as_stream().unwrap();
    folio_core::filters::decode_stream(s).unwrap()
}

fn protect(compat: Compat, s: Settings) -> Vec<u8> {
    let mut d = sample();
    d.encrypt(&Settings { compat, ..s }).unwrap();
    d.to_bytes().unwrap()
}

#[test]
fn open_password_roundtrip_for_every_compatibility_level() {
    for compat in ALL {
        let bytes = protect(
            compat,
            Settings {
                open_password: Some("deschide".into()),
                ..Settings::default()
            },
        );
        // The title must not be readable in the file.
        assert!(
            !bytes.windows(12).any(|w| w == b"Confidential"),
            "{compat:?}: plaintext leaked"
        );
        let err = Document::from_bytes(&bytes).unwrap_err();
        assert!(matches!(err, Error::Password(_)), "{compat:?}: {err}");
        let err = Document::from_bytes_with_password(&bytes, "gresit").unwrap_err();
        assert!(err.to_string().contains("not correct"), "{compat:?}: {err}");
        let d = Document::from_bytes_with_password(&bytes, "deschide").unwrap();
        assert_eq!(title(&d), b"Confidential report", "{compat:?}");
        assert!(
            content(&d, 0).windows(6).any(|w| w == b"Secret"),
            "{compat:?}"
        );
        assert_eq!(d.pages().unwrap().len(), 2);
    }
}

#[test]
fn permissions_password_opens_with_full_access_and_restrictions_apply_otherwise() {
    for compat in ALL {
        let bytes = protect(
            compat,
            Settings {
                permissions_password: Some("owner-pw".into()),
                printing: Printing::None,
                changes: Changes::None,
                copy: false,
                screen_reader: false,
                ..Settings::default()
            },
        );
        // No open password: opens without one, but restricted.
        let d = Document::from_bytes(&bytes).unwrap();
        assert_eq!(title(&d), b"Confidential report");
        let sec = d.security.as_ref().unwrap();
        assert!(!sec.owner_access && !sec.needs_open_password, "{compat:?}");
        for need in [
            Need::Assemble,
            Need::Extract,
            Need::Modify,
            Need::Copy,
            Need::Owner,
        ] {
            assert!(
                matches!(d.require(need), Err(Error::Permission(_))),
                "{compat:?} {need:?}"
            );
        }
        let mut d2 = d.clone();
        assert!(d2.remove_security().is_err());
        // The permissions password unlocks everything.
        let mut o = Document::from_bytes_with_password(&bytes, "owner-pw").unwrap();
        assert!(o.security.as_ref().unwrap().owner_access, "{compat:?}");
        assert!(o.require(Need::Modify).is_ok());
        o.remove_security().unwrap();
        let plain = o.to_bytes().unwrap();
        assert!(plain.windows(12).any(|w| w == b"Confidential"));
        assert!(Document::from_bytes(&plain).unwrap().security.is_none());
    }
}

#[test]
fn changes_allowed_choices_map_to_commands() {
    let pages_only = protect(
        Compat::AcrobatX,
        Settings {
            permissions_password: Some("o".into()),
            changes: Changes::Pages,
            ..Settings::default()
        },
    );
    let d = Document::from_bytes(&pages_only).unwrap();
    assert!(d.require(Need::Assemble).is_ok());
    assert!(d.require(Need::Modify).is_err());
    assert!(d.require(Need::Extract).is_err());
    assert!(d.require(Need::Copy).is_ok());

    let any = protect(
        Compat::Acrobat7,
        Settings {
            permissions_password: Some("o".into()),
            changes: Changes::Any,
            copy: false,
            ..Settings::default()
        },
    );
    let d = Document::from_bytes(&any).unwrap();
    assert!(d.require(Need::Modify).is_ok());
    assert!(d.require(Need::Assemble).is_ok());
    assert!(d.require(Need::Copy).is_err());
    assert!(
        d.require(Need::Extract).is_err(),
        "any except extracting pages"
    );
}

#[test]
fn saving_keeps_security_like_acrobat() {
    for compat in ALL {
        let bytes = protect(
            compat,
            Settings {
                open_password: Some("u".into()),
                permissions_password: Some("o".into()),
                ..Settings::default()
            },
        );
        let mut d = Document::from_bytes_with_password(&bytes, "o").unwrap();
        d.add_text_page("Added later").unwrap();
        let again = d.to_bytes().unwrap();
        assert!(matches!(
            Document::from_bytes(&again),
            Err(Error::Password(_))
        ));
        let d = Document::from_bytes_with_password(&again, "u").unwrap();
        assert_eq!(d.pages().unwrap().len(), 3, "{compat:?}");
        assert!(content(&d, 2).windows(5).any(|w| w == b"Added"));
    }
}

#[test]
fn metadata_can_stay_unencrypted() {
    let mut d = sample();
    let xmp = b"<x:xmpmeta>folio-visible-metadata</x:xmpmeta>".to_vec();
    let mut md = folio_core::Dict::new();
    md.set("Type", Object::name("Metadata"));
    md.set("Subtype", Object::name("XML"));
    let r = d.add(Object::Stream(folio_core::Stream {
        dict: md,
        data: xmp,
    }));
    d.catalog_mut().unwrap().set("Metadata", r.into());
    let s = Settings {
        compat: Compat::Acrobat6,
        scope: Scope::ExceptMetadata,
        open_password: Some("pw".into()),
        ..Settings::default()
    };
    d.encrypt(&s).unwrap();
    let bytes = d.to_bytes().unwrap();
    assert!(bytes.windows(22).any(|w| w == b"folio-visible-metadata"));
    assert!(!bytes.windows(12).any(|w| w == b"Confidential"));
    let back = Document::from_bytes_with_password(&bytes, "pw").unwrap();
    assert_eq!(title(&back), b"Confidential report");
}

#[test]
fn attachments_only_leaves_the_document_readable() {
    let mut d = sample();
    let mut ef = folio_core::Dict::new();
    ef.set("Type", Object::name("EmbeddedFile"));
    let att = d.add(Object::Stream(folio_core::Stream {
        dict: ef,
        data: b"attachment-secret-bytes".to_vec(),
    }));
    d.info_mut().unwrap().set("Att", att.into());
    d.encrypt(&Settings {
        compat: Compat::AcrobatX,
        scope: Scope::AttachmentsOnly,
        open_password: Some("att".into()),
        ..Settings::default()
    })
    .unwrap();
    let bytes = d.to_bytes().unwrap();
    assert!(bytes.windows(12).any(|w| w == b"Confidential"));
    assert!(!bytes.windows(17).any(|w| w == b"attachment-secret"));
}

#[test]
fn acrobat_rules_for_passwords_and_compatibility() {
    let mut d = sample();
    let same = Settings {
        open_password: Some("x".into()),
        permissions_password: Some("x".into()),
        ..Settings::default()
    };
    assert!(
        d.encrypt(&same)
            .unwrap_err()
            .to_string()
            .contains("must be different")
    );
    let restricted_without_owner = Settings {
        open_password: Some("x".into()),
        printing: Printing::Low,
        ..Settings::default()
    };
    assert!(d.encrypt(&restricted_without_owner).is_err());
    let none = Settings::default();
    assert!(d.encrypt(&none).is_err());
    let old_meta = Settings {
        compat: Compat::Acrobat5,
        scope: Scope::ExceptMetadata,
        open_password: Some("x".into()),
        ..Settings::default()
    };
    assert!(
        d.encrypt(&old_meta)
            .unwrap_err()
            .to_string()
            .contains("Acrobat 6.0")
    );
    let att_no_open = Settings {
        compat: Compat::AcrobatX,
        scope: Scope::AttachmentsOnly,
        permissions_password: Some("x".into()),
        ..Settings::default()
    };
    assert!(d.encrypt(&att_no_open).is_err());
}

#[test]
fn unicode_passwords_work_for_aes256() {
    let bytes = protect(
        Compat::AcrobatX,
        Settings {
            open_password: Some("parolă-ÁRVÍZTŰRŐ".into()),
            ..Settings::default()
        },
    );
    assert!(Document::from_bytes_with_password(&bytes, "parolă-ÁRVÍZTŰRŐ").is_ok());
    assert!(Document::from_bytes_with_password(&bytes, "parola-ARVIZTURO").is_err());
}

/// Interoperability: if Poppler's pdftotext is installed, it must read the
/// text of what Folio encrypts, with either password.
#[test]
fn poppler_reads_folio_encryption() {
    if std::process::Command::new("pdftotext")
        .arg("-v")
        .output()
        .is_err()
    {
        eprintln!("pdftotext not installed; interoperability check skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("folio-poppler-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for compat in ALL {
        let path = dir.join(format!("{compat:?}.pdf"));
        std::fs::write(
            &path,
            protect(
                compat,
                Settings {
                    open_password: Some("user".into()),
                    permissions_password: Some("owner".into()),
                    ..Settings::default()
                },
            ),
        )
        .unwrap();
        for (flag, pw) in [("-upw", "user"), ("-opw", "owner")] {
            let out = std::process::Command::new("pdftotext")
                .args([flag, pw])
                .arg(&path)
                .arg("-")
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(
                text.contains("Secret page (one)") && text.contains("Page two"),
                "{compat:?} {flag}: {text}{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let wrong = std::process::Command::new("pdftotext")
            .args(["-upw", "nope"])
            .arg(&path)
            .arg("-")
            .output()
            .unwrap();
        assert!(
            !wrong.status.success(),
            "{compat:?}: wrong password accepted"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Interoperability the other way: Ghostscript encrypts (RC4 40 and 128),
/// Folio must open the result with either password.
#[test]
fn folio_opens_ghostscript_encryption() {
    let gs = ["gswin64c", "gs"].into_iter().find(|g| {
        std::process::Command::new(g)
            .arg("--version")
            .output()
            .is_ok()
    });
    let Some(gs) = gs else {
        eprintln!("ghostscript not installed; interoperability check skipped");
        return;
    };
    let dir = std::env::temp_dir().join(format!("folio-gs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("plain.pdf");
    sample().save(&src).unwrap();
    for (r, bits) in [("2", "40"), ("3", "128")] {
        let out = dir.join(format!("gs-r{r}.pdf"));
        let st = std::process::Command::new(gs)
            .args(["-q", "-dNOPAUSE", "-dBATCH", "-dSAFER", "-sDEVICE=pdfwrite"])
            .arg("-sOwnerPassword=gs-owner")
            .arg("-sUserPassword=gs-user")
            .arg(format!("-dEncryptionR={r}"))
            .arg(format!("-dKeyLength={bits}"))
            .arg(format!("-sOutputFile={}", out.display()))
            .arg(&src)
            .status()
            .unwrap();
        assert!(st.success(), "ghostscript failed for R{r}");
        assert!(matches!(Document::load(&out), Err(Error::Password(_))));
        for pw in ["gs-user", "gs-owner"] {
            let d = Document::load_with_password(&out, pw)
                .unwrap_or_else(|e| panic!("R{r} with {pw}: {e}"));
            assert_eq!(d.pages().unwrap().len(), 2, "R{r} with {pw}");
            assert_eq!(
                d.security.as_ref().unwrap().owner_access,
                pw == "gs-owner",
                "R{r} with {pw}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Interoperability: if qpdf is installed (CI installs it), it must open
/// and validate what Folio encrypts. qpdf runs only as an external oracle.
#[test]
fn qpdf_accepts_folio_encryption() {
    if std::process::Command::new("qpdf")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("qpdf not installed; interoperability check skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("folio-qpdf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for compat in ALL {
        let path = dir.join(format!("{compat:?}.pdf"));
        std::fs::write(
            &path,
            protect(
                compat,
                Settings {
                    open_password: Some("user".into()),
                    permissions_password: Some("owner".into()),
                    printing: Printing::Low,
                    ..Settings::default()
                },
            ),
        )
        .unwrap();
        for pw in ["user", "owner"] {
            let out = std::process::Command::new("qpdf")
                .arg(format!("--password={pw}"))
                .arg("--check")
                .arg(&path)
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success() && text.contains("No syntax or stream encoding errors"),
                "{compat:?} with {pw}: {text}{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        // qpdf encrypts, Folio opens (both passwords).
        for bits in ["40", "128", "256"] {
            let theirs = dir.join(format!("{compat:?}-qpdf-{bits}.pdf"));
            let mut c2 = std::process::Command::new("qpdf");
            c2.arg("--password=owner")
                .arg("--encrypt")
                .args(["u2", "o2", bits]);
            if bits == "128" {
                c2.arg("--use-aes=y");
            }
            let ok = c2.arg("--").arg(&path).arg(&theirs).status().unwrap();
            assert!(ok.success(), "qpdf could not re-encrypt at {bits} bits");
            for pw in ["u2", "o2"] {
                let d = Document::load_with_password(&theirs, pw).unwrap();
                assert_eq!(title(&d), b"Confidential report", "qpdf {bits} with {pw}");
            }
        }
        // qpdf decrypts with the user password and Folio reads the result.
        let plain = dir.join(format!("{compat:?}-plain.pdf"));
        let st = std::process::Command::new("qpdf")
            .args(["--password=user", "--decrypt"])
            .arg(&path)
            .arg(&plain)
            .status()
            .unwrap();
        assert!(st.success(), "{compat:?}: qpdf could not decrypt");
        let d = Document::load(&plain).unwrap();
        assert_eq!(title(&d), b"Confidential report");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
