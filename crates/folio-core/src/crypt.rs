//! The standard security handler (ISO 32000-2 §7.6.4): passwords,
//! permissions, and RC4 / AES-128 / AES-256 encryption of strings and
//! streams. Revisions 2, 3, 4, 5 (read only) and 6.
//!
//! Block ciphers and hashes come from the RustCrypto crates (pure Rust,
//! audited); only RC4 and the PDF key derivations are written here.

use aes::{Aes128, Aes256};
use cipher::block_padding::{NoPadding, Pkcs7};
use cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use md5::{Digest, Md5};
use sha2::{Sha256, Sha384, Sha512};

use crate::error::{Error, Result};
use crate::object::{Dict, Object};

/// Padding string for passwords (Algorithm 2, step a).
const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cipher {
    Rc4,
    Aes128,
    Aes256,
    Identity,
}

/// Acrobat's "Compatibility" choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Compat {
    /// 40-bit RC4 (R2).
    Acrobat3,
    /// 128-bit RC4 (R3).
    Acrobat5,
    /// 128-bit RC4 with crypt filters (R4); allows leaving metadata clear.
    Acrobat6,
    /// 128-bit AES (R4); allows encrypting only attachments.
    Acrobat7,
    /// 256-bit AES (R6).
    AcrobatX,
}

/// Acrobat's "Select Document Components to Encrypt".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    All,
    ExceptMetadata,
    AttachmentsOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Printing {
    None,
    Low,
    High,
}

/// Acrobat's "Changes Allowed".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Changes {
    None,
    /// Inserting, deleting, and rotating pages.
    Pages,
    /// Filling in form fields and signing existing signature fields.
    Forms,
    /// Commenting, filling in form fields, and signing existing signature fields.
    Comments,
    /// Any except extracting pages.
    Any,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub compat: Compat,
    pub scope: Scope,
    pub open_password: Option<String>,
    pub permissions_password: Option<String>,
    pub printing: Printing,
    pub changes: Changes,
    pub copy: bool,
    pub screen_reader: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            compat: Compat::AcrobatX,
            scope: Scope::All,
            open_password: None,
            permissions_password: None,
            printing: Printing::High,
            changes: Changes::Any,
            copy: true,
            screen_reader: true,
        }
    }
}

/// The `/P` permission bits (ISO 32000-2 Table 22), numbered from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Permissions(pub u32);

impl Permissions {
    pub fn bit(self, n: u32) -> bool {
        (1..=32).contains(&n) && (self.0 >> (n - 1)) & 1 == 1
    }
}

/// What a command needs to be allowed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// Document Assembly: insert, delete, rotate, reorder pages.
    Assemble,
    /// Page Extraction.
    Extract,
    /// Changing the Document.
    Modify,
    /// Content Copying.
    Copy,
    /// Changing or removing security.
    Owner,
}

impl Need {
    pub fn describe(self) -> &'static str {
        match self {
            Need::Assemble => "document assembly (inserting, deleting, rotating pages)",
            Need::Extract => "page extraction",
            Need::Modify => "changing the document",
            Need::Copy => "content copying",
            Need::Owner => "changing security",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Security {
    /// The /Encrypt dictionary written with the file.
    pub dict: Dict,
    key: Vec<u8>,
    pub revision: i64,
    stm: Cipher,
    strf: Cipher,
    eff: Cipher,
    pub encrypt_metadata: bool,
    pub permissions: Permissions,
    /// Opened with the permissions (owner) password, or created here.
    pub owner_access: bool,
    /// Whether the file needs a password to open.
    pub needs_open_password: bool,
}

impl Security {
    /// A short description, as Acrobat's "Encryption Level".
    pub fn method(&self) -> String {
        let c = match self.stm {
            Cipher::Identity => self.eff,
            c => c,
        };
        let bits = match (c, self.key.len()) {
            (Cipher::Aes256, _) => 256,
            (Cipher::Aes128, _) => 128,
            (_, n) => n * 8,
        };
        let name = match c {
            Cipher::Rc4 => "RC4",
            Cipher::Aes128 | Cipher::Aes256 => "AES",
            Cipher::Identity => "none",
        };
        format!("{bits}-bit {name} (revision {})", self.revision)
    }

    pub fn allows(&self, need: Need) -> bool {
        if self.owner_access {
            return true;
        }
        let p = self.permissions;
        match need {
            Need::Assemble => p.bit(4) || (self.revision >= 3 && p.bit(11)),
            Need::Extract => p.bit(4) && p.bit(5),
            Need::Modify => p.bit(4),
            Need::Copy => p.bit(5),
            Need::Owner => false,
        }
    }

    /// Open: try the password as the owner password, then as the user password.
    pub fn open(enc: &Dict, id0: &[u8], password: &str) -> Result<Security> {
        let filter = enc.get_name(b"Filter").unwrap_or(b"Standard");
        if filter != b"Standard" {
            return Err(Error::Unsupported(format!(
                "this PDF is encrypted with the /{} security handler (certificate or rights-management security)",
                String::from_utf8_lossy(filter)
            )));
        }
        let v = enc.get_int(b"V").unwrap_or(0);
        let r = enc.get_int(b"R").unwrap_or(0);
        let o = enc.get(b"O").and_then(Object::as_str).unwrap_or_default();
        let u = enc.get(b"U").and_then(Object::as_str).unwrap_or_default();
        let p = Permissions(enc.get_int(b"P").unwrap_or(-1) as i32 as u32);
        let encrypt_metadata = !matches!(enc.get(b"EncryptMetadata"), Some(Object::Bool(false)));
        let (stm, strf, eff) = ciphers(enc, v)?;
        let mut sec = Security {
            dict: enc.clone(),
            key: Vec::new(),
            revision: r,
            stm,
            strf,
            eff,
            encrypt_metadata,
            permissions: p,
            owner_access: false,
            needs_open_password: true,
        };
        match r {
            2..=4 => {
                let n = if r == 2 {
                    5
                } else {
                    (enc.get_int(b"Length").unwrap_or(40) / 8).clamp(5, 16) as usize
                };
                let n = if v >= 4 { 16 } else { n };
                let pw = legacy_password(password);
                sec.needs_open_password = legacy_user_key(b"", o, p, id0, r, n, encrypt_metadata)
                    .filter(|k| legacy_check_user(k, u, id0, r))
                    .is_none();
                let as_user = owner_to_user(&pw, o, r, n);
                if let Some(k) = legacy_user_key(&as_user, o, p, id0, r, n, encrypt_metadata)
                    .filter(|k| legacy_check_user(k, u, id0, r))
                {
                    sec.key = k;
                    sec.owner_access = true;
                } else if let Some(k) = legacy_user_key(&pw, o, p, id0, r, n, encrypt_metadata)
                    .filter(|k| legacy_check_user(k, u, id0, r))
                {
                    sec.key = k;
                } else {
                    return Err(password_error(password));
                }
            }
            5 | 6 => {
                let pw = utf8_password(password);
                let (Some(_), Some(u48)) = (o.get(..48), u.get(..48)) else {
                    return Err(Error::Invalid(
                        "the encryption dictionary is damaged".into(),
                    ));
                };
                let oe = enc.get(b"OE").and_then(Object::as_str).unwrap_or_default();
                let ue = enc.get(b"UE").and_then(Object::as_str).unwrap_or_default();
                let empty_user = hash_r6(r, b"", sub(u, 32, 40), &[]);
                sec.needs_open_password = Some(empty_user.as_slice()) != u.get(..32);
                if Some(hash_r6(r, &pw, sub(o, 32, 40), u48).as_slice()) == o.get(..32) {
                    let k = hash_r6(r, &pw, sub(o, 40, 48), u48);
                    sec.key = aes256_nopad_decrypt(&k, oe)?;
                    sec.owner_access = true;
                } else if Some(hash_r6(r, &pw, sub(u, 32, 40), &[]).as_slice()) == u.get(..32) {
                    let k = hash_r6(r, &pw, sub(u, 40, 48), &[]);
                    sec.key = aes256_nopad_decrypt(&k, ue)?;
                } else {
                    return Err(password_error(password));
                }
                if sec.key.len() != 32 {
                    return Err(Error::Invalid("the encryption key is damaged".into()));
                }
            }
            other => {
                return Err(Error::Unsupported(format!(
                    "this PDF is encrypted with security handler revision {other}"
                )));
            }
        }
        Ok(sec)
    }

    /// New security, as Acrobat's Password Security – Settings dialog makes it.
    pub fn create(s: &Settings, id0: &[u8]) -> Result<Security> {
        let open = s.open_password.clone().filter(|p| !p.is_empty());
        let perms_pw = s.permissions_password.clone().filter(|p| !p.is_empty());
        if open.is_none() && perms_pw.is_none() {
            return Err(Error::Invalid(
                "give an open password, a permissions password, or both".into(),
            ));
        }
        if open.is_some() && open == perms_pw {
            return Err(Error::Invalid(
                "the Document Open password and the Permissions password must be different".into(),
            ));
        }
        let restricted = s.printing != Printing::High
            || s.changes != Changes::Any
            || !s.copy
            || !s.screen_reader;
        if restricted && perms_pw.is_none() {
            return Err(Error::Invalid(
                "restricting printing, editing or copying needs a permissions password".into(),
            ));
        }
        if s.scope == Scope::ExceptMetadata && s.compat < Compat::Acrobat6 {
            return Err(Error::Invalid(
                "leaving metadata unencrypted needs Acrobat 6.0 compatibility or later".into(),
            ));
        }
        if s.scope == Scope::AttachmentsOnly {
            if s.compat < Compat::Acrobat7 {
                return Err(Error::Invalid(
                    "encrypting only attachments needs Acrobat 7.0 compatibility or later".into(),
                ));
            }
            if open.is_none() {
                return Err(Error::Invalid(
                    "encrypting only attachments needs an open password".into(),
                ));
            }
        }
        let r: i64 = match s.compat {
            Compat::Acrobat3 => 2,
            Compat::Acrobat5 => 3,
            Compat::Acrobat6 | Compat::Acrobat7 => 4,
            Compat::AcrobatX => 6,
        };
        let p = permission_bits(s, r);
        let user = open.clone().unwrap_or_default();
        let owner = perms_pw.clone().unwrap_or_else(|| user.clone());
        let encrypt_metadata = s.scope != Scope::ExceptMetadata;
        let cipher = match s.compat {
            Compat::Acrobat3 | Compat::Acrobat5 | Compat::Acrobat6 => Cipher::Rc4,
            Compat::Acrobat7 => Cipher::Aes128,
            Compat::AcrobatX => Cipher::Aes256,
        };
        let (stm, strf, eff) = if s.scope == Scope::AttachmentsOnly {
            (Cipher::Identity, Cipher::Identity, cipher)
        } else {
            (cipher, cipher, cipher)
        };

        let mut d = Dict::new();
        d.set("Filter", Object::name("Standard"));
        let key;
        if r == 6 {
            key = random(32)?;
            let upw = utf8_password(&user);
            let opw = utf8_password(&owner);
            let (uvs, uks) = (random(8)?, random(8)?);
            let mut u = hash_r6(6, &upw, &uvs, &[]);
            u.extend_from_slice(&uvs);
            u.extend_from_slice(&uks);
            let ue = aes256_nopad_encrypt(&hash_r6(6, &upw, &uks, &[]), &key)?;
            let (ovs, oks) = (random(8)?, random(8)?);
            let mut o = hash_r6(6, &opw, &ovs, &u);
            o.extend_from_slice(&ovs);
            o.extend_from_slice(&oks);
            let oe = aes256_nopad_encrypt(&hash_r6(6, &opw, &oks, &u), &key)?;
            let mut perms = Vec::with_capacity(16);
            perms.extend_from_slice(&p.0.to_le_bytes());
            perms.extend_from_slice(&[0xFF; 4]);
            perms.push(if encrypt_metadata { b'T' } else { b'F' });
            perms.extend_from_slice(b"adb");
            perms.extend_from_slice(&random(4)?);
            let perms = aes256_nopad_encrypt(&key, &perms)?;
            d.set("V", Object::Int(5));
            d.set("R", Object::Int(6));
            d.set("Length", Object::Int(256));
            d.set("O", Object::Str(o));
            d.set("U", Object::Str(u));
            d.set("OE", Object::Str(oe));
            d.set("UE", Object::Str(ue));
            d.set("Perms", Object::Str(perms));
        } else {
            let n = if r == 2 { 5 } else { 16 };
            let upw = legacy_password(&user);
            let opw = legacy_password(&owner);
            let o = compute_o(&opw, &upw, r, n);
            key = legacy_user_key(&upw, &o, p, id0, r, n, encrypt_metadata)
                .ok_or_else(|| Error::Invalid("could not derive the encryption key".into()))?;
            let u = compute_u(&key, id0, r);
            let (v, len) = match r {
                2 => (1, 40),
                3 => (2, 128),
                _ => (4, 128),
            };
            d.set("V", Object::Int(v));
            d.set("R", Object::Int(r));
            d.set("Length", Object::Int(len));
            d.set("O", Object::Str(o));
            d.set("U", Object::Str(u));
        }
        d.set("P", Object::Int(i64::from(p.0 as i32)));
        if r >= 4 {
            let cfm = match cipher {
                Cipher::Rc4 => "V2",
                Cipher::Aes128 => "AESV2",
                _ => "AESV3",
            };
            let mut std_cf = Dict::new();
            std_cf.set("Type", Object::name("CryptFilter"));
            std_cf.set("CFM", Object::name(cfm));
            std_cf.set(
                "AuthEvent",
                Object::name(if s.scope == Scope::AttachmentsOnly {
                    "EFOpen"
                } else {
                    "DocOpen"
                }),
            );
            std_cf.set("Length", Object::Int(if r == 6 { 32 } else { 16 }));
            let mut cf = Dict::new();
            cf.set("StdCF", std_cf.into());
            d.set("CF", cf.into());
            if s.scope == Scope::AttachmentsOnly {
                d.set("StmF", Object::name("Identity"));
                d.set("StrF", Object::name("Identity"));
                d.set("EFF", Object::name("StdCF"));
            } else {
                d.set("StmF", Object::name("StdCF"));
                d.set("StrF", Object::name("StdCF"));
            }
            if !encrypt_metadata {
                d.set("EncryptMetadata", Object::Bool(false));
            }
        }
        Ok(Security {
            dict: d,
            key,
            revision: r,
            stm,
            strf,
            eff,
            encrypt_metadata,
            permissions: p,
            owner_access: true,
            needs_open_password: open.is_some(),
        })
    }

    fn object_key(&self, num: u32, generation: u16, aes: bool) -> Vec<u8> {
        if self.revision >= 5 {
            return self.key.clone();
        }
        let mut h = Md5::new();
        h.update(&self.key);
        h.update(&num.to_le_bytes()[..3]);
        h.update(generation.to_le_bytes());
        if aes {
            h.update(b"sAlT");
        }
        let digest = h.finalize();
        let n = (self.key.len() + 5).min(16);
        digest.get(..n).unwrap_or_default().to_vec()
    }

    fn apply(&self, c: Cipher, num: u32, generation: u16, data: &[u8], encrypt: bool) -> Vec<u8> {
        let key = self.object_key(num, generation, matches!(c, Cipher::Aes128));
        match (c, encrypt) {
            (Cipher::Identity, _) => data.to_vec(),
            (Cipher::Rc4, _) => rc4(&key, data),
            (Cipher::Aes128, false) => aes_cbc_decrypt::<Aes128>(&key, data),
            (Cipher::Aes256, false) => aes_cbc_decrypt::<Aes256>(&key, data),
            (Cipher::Aes128, true) => aes_cbc_encrypt::<Aes128>(&key, data),
            (Cipher::Aes256, true) => aes_cbc_encrypt::<Aes256>(&key, data),
        }
    }

    /// Decrypt every string and stream inside indirect object `num gen`.
    pub fn decrypt_object(&self, num: u32, generation: u16, obj: &mut Object) {
        self.walk(num, generation, obj, false);
    }

    /// Encrypt every string and stream inside indirect object `num gen`.
    pub fn encrypt_object(&self, num: u32, generation: u16, obj: &mut Object) {
        self.walk(num, generation, obj, true);
    }

    fn walk(&self, num: u32, g: u16, obj: &mut Object, enc: bool) {
        match obj {
            Object::Str(s) => *s = self.apply(self.strf, num, g, s, enc),
            Object::Array(a) => a.iter_mut().for_each(|o| self.walk(num, g, o, enc)),
            Object::Dict(d) => self.walk_dict(num, g, d, enc),
            Object::Stream(s) => {
                let ty = s.dict.get_name(b"Type").map(<[u8]>::to_vec);
                // Cross-reference streams are never encrypted (§7.6.2).
                if ty.as_deref() == Some(b"XRef") {
                    return;
                }
                let c = match ty.as_deref() {
                    Some(b"XRef") => Cipher::Identity,
                    Some(b"Metadata") if !self.encrypt_metadata => Cipher::Identity,
                    Some(b"EmbeddedFile") => self.eff,
                    _ => self.stm,
                };
                let identity_filter = matches!(s.dict.get(b"Filter"), Some(Object::Array(a))
                    if a.first().and_then(Object::as_name) == Some(b"Crypt"));
                if !identity_filter {
                    s.data = self.apply(c, num, g, &s.data, enc);
                }
                self.walk_dict(num, g, &mut s.dict, enc);
            }
            _ => {}
        }
    }

    fn walk_dict(&self, num: u32, g: u16, d: &mut Dict, enc: bool) {
        // Signature values are never encrypted (§7.6.2).
        let is_sig = d.contains(b"ByteRange");
        for (k, v) in d.iter_mut_pairs() {
            if is_sig && k == b"Contents" {
                continue;
            }
            self.walk(num, g, v, enc);
        }
    }
}

fn password_error(password: &str) -> Error {
    if password.is_empty() {
        Error::Password("this PDF needs a password to open; pass --password".into())
    } else {
        Error::Password("the password is not correct".into())
    }
}

fn sub(b: &[u8], from: usize, to: usize) -> &[u8] {
    b.get(from..to).unwrap_or_default()
}

fn ciphers(enc: &Dict, v: i64) -> Result<(Cipher, Cipher, Cipher)> {
    if v < 4 {
        return Ok((Cipher::Rc4, Cipher::Rc4, Cipher::Rc4));
    }
    let cf = enc.get(b"CF").and_then(Object::as_dict);
    let lookup = |name: Option<&[u8]>| -> Result<Cipher> {
        let Some(name) = name else {
            return Ok(Cipher::Identity);
        };
        if name == b"Identity" {
            return Ok(Cipher::Identity);
        }
        let f = cf
            .and_then(|c| c.get(name))
            .and_then(Object::as_dict)
            .ok_or_else(|| {
                Error::Invalid("the encryption dictionary names a missing crypt filter".into())
            })?;
        match f.get_name(b"CFM") {
            Some(b"V2") => Ok(Cipher::Rc4),
            Some(b"AESV2") => Ok(Cipher::Aes128),
            Some(b"AESV3") => Ok(Cipher::Aes256),
            Some(b"None") | None => Ok(Cipher::Identity),
            Some(other) => Err(Error::Unsupported(format!(
                "crypt filter method /{}",
                String::from_utf8_lossy(other)
            ))),
        }
    };
    let stm = lookup(enc.get_name(b"StmF"))?;
    let strf = lookup(enc.get_name(b"StrF"))?;
    let eff = match enc.get_name(b"EFF") {
        Some(n) => lookup(Some(n))?,
        None => stm,
    };
    Ok((stm, strf, eff))
}

fn permission_bits(s: &Settings, r: i64) -> Permissions {
    let mut p: u32 = if r == 2 { 0xFFFF_FFC0 } else { 0xFFFF_F0C0 };
    let mut set = |bit: u32| p |= 1 << (bit - 1);
    match s.printing {
        Printing::None => {}
        Printing::Low => set(3),
        Printing::High => {
            set(3);
            if r >= 3 {
                set(12);
            }
        }
    }
    if r == 2 {
        match s.changes {
            Changes::None => {}
            Changes::Forms | Changes::Comments => set(6),
            Changes::Pages => set(4),
            Changes::Any => {
                set(4);
                set(6);
            }
        }
    } else {
        match s.changes {
            Changes::None => {}
            Changes::Pages => set(11),
            Changes::Forms => set(9),
            Changes::Comments => {
                set(6);
                set(9);
            }
            Changes::Any => {
                set(4);
                set(6);
                set(9);
                set(11);
            }
        }
        if s.screen_reader || s.copy {
            set(10);
        }
    }
    if s.copy {
        set(5);
    }
    Permissions(p)
}

/// Passwords for R2–R4: PDFDocEncoding (Latin-1 here), at most 32 bytes.
fn legacy_password(p: &str) -> Vec<u8> {
    p.chars()
        .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
        .take(32)
        .collect()
}

/// Passwords for R5/R6: UTF-8, at most 127 bytes (SASLprep left to the user).
fn utf8_password(p: &str) -> Vec<u8> {
    let mut b = p.as_bytes().to_vec();
    b.truncate(127);
    while std::str::from_utf8(&b).is_err() && !b.is_empty() {
        b.pop();
    }
    b
}

fn padded(pw: &[u8]) -> [u8; 32] {
    let mut out = PAD;
    for (slot, b) in out.iter_mut().zip(pw.iter()) {
        *slot = *b;
    }
    if pw.len() < 32 {
        for (i, slot) in out.iter_mut().enumerate().skip(pw.len()) {
            *slot = PAD.get(i - pw.len()).copied().unwrap_or(0);
        }
    }
    out
}

/// Algorithm 2.
fn legacy_user_key(
    pw: &[u8],
    o: &[u8],
    p: Permissions,
    id0: &[u8],
    r: i64,
    n: usize,
    encrypt_metadata: bool,
) -> Option<Vec<u8>> {
    let mut h = Md5::new();
    h.update(padded(pw));
    h.update(o.get(..32).unwrap_or(o));
    h.update(p.0.to_le_bytes());
    h.update(id0);
    if r >= 4 && !encrypt_metadata {
        h.update([0xFF; 4]);
    }
    let mut d = h.finalize().to_vec();
    if r >= 3 {
        for _ in 0..50 {
            d = Md5::digest(d.get(..n)?).to_vec();
        }
    }
    d.truncate(n);
    Some(d)
}

/// Algorithms 6 (check) and 4/5 (compute) for the /U entry.
fn compute_u(key: &[u8], id0: &[u8], r: i64) -> Vec<u8> {
    if r == 2 {
        return rc4(key, &PAD);
    }
    let mut h = Md5::new();
    h.update(PAD);
    h.update(id0);
    let mut x = rc4(key, &h.finalize());
    for i in 1..=19u8 {
        let k: Vec<u8> = key.iter().map(|b| b ^ i).collect();
        x = rc4(&k, &x);
    }
    x.extend_from_slice(&[0u8; 16]);
    x
}

fn legacy_check_user(key: &[u8], u: &[u8], id0: &[u8], r: i64) -> bool {
    let calc = compute_u(key, id0, r);
    let n = if r == 2 { 32 } else { 16 };
    calc.get(..n).is_some_and(|c| u.get(..n) == Some(c))
}

/// Algorithm 3 (owner key) shared by computing O and reading it back.
fn owner_rc4_key(owner: &[u8], r: i64, n: usize) -> Vec<u8> {
    let mut d = Md5::digest(padded(owner)).to_vec();
    if r >= 3 {
        for _ in 0..50 {
            d = Md5::digest(&d).to_vec();
        }
    }
    d.truncate(n);
    d
}

fn compute_o(owner: &[u8], user: &[u8], r: i64, n: usize) -> Vec<u8> {
    let key = owner_rc4_key(owner, r, n);
    let mut x = rc4(&key, &padded(user));
    if r >= 3 {
        for i in 1..=19u8 {
            let k: Vec<u8> = key.iter().map(|b| b ^ i).collect();
            x = rc4(&k, &x);
        }
    }
    x
}

/// Algorithm 7: recover the user password from /O with an owner password.
fn owner_to_user(owner: &[u8], o: &[u8], r: i64, n: usize) -> Vec<u8> {
    let key = owner_rc4_key(owner, r, n);
    let mut x = o.get(..32).unwrap_or(o).to_vec();
    if r == 2 {
        x = rc4(&key, &x);
    } else {
        for i in (0..=19u8).rev() {
            let k: Vec<u8> = key.iter().map(|b| b ^ i).collect();
            x = rc4(&k, &x);
        }
    }
    x
}

/// Algorithm 2.B (R6), or plain SHA-256 for the deprecated R5.
fn hash_r6(r: i64, pw: &[u8], salt: &[u8], udata: &[u8]) -> Vec<u8> {
    let mut k = {
        let mut h = Sha256::new();
        h.update(pw);
        h.update(salt);
        h.update(udata);
        h.finalize().to_vec()
    };
    if r == 5 {
        return k;
    }
    let mut i: u32 = 0;
    loop {
        let mut k1 = Vec::with_capacity(64 * (pw.len() + k.len() + udata.len()));
        for _ in 0..64 {
            k1.extend_from_slice(pw);
            k1.extend_from_slice(&k);
            k1.extend_from_slice(udata);
        }
        let e = match cbc::Encryptor::<Aes128>::new_from_slices(sub(&k, 0, 16), sub(&k, 16, 32)) {
            Ok(c) => c.encrypt_padded_vec_mut::<NoPadding>(&k1),
            Err(_) => return k,
        };
        let m = e.iter().take(16).map(|&b| u32::from(b)).sum::<u32>() % 3;
        k = match m {
            0 => Sha256::digest(&e).to_vec(),
            1 => Sha384::digest(&e).to_vec(),
            _ => Sha512::digest(&e).to_vec(),
        };
        i += 1;
        let last = u32::from(e.last().copied().unwrap_or(0));
        if i >= 64 && last <= i - 32 {
            break;
        }
        if i > 10_000 {
            break;
        }
    }
    k.truncate(32);
    k
}

fn aes256_nopad_decrypt(key: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    let data = data.get(..32).unwrap_or(data);
    cbc::Decryptor::<Aes256>::new_from_slices(key, &[0u8; 16])
        .map_err(|_| Error::Invalid("bad AES-256 key".into()))?
        .decrypt_padded_vec_mut::<NoPadding>(data)
        .map_err(|_| Error::Invalid("the encryption key is damaged".into()))
}

fn aes256_nopad_encrypt(key: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    Ok(cbc::Encryptor::<Aes256>::new_from_slices(key, &[0u8; 16])
        .map_err(|_| Error::Invalid("bad AES-256 key".into()))?
        .encrypt_padded_vec_mut::<NoPadding>(data))
}

fn aes_cbc_decrypt<C>(key: &[u8], data: &[u8]) -> Vec<u8>
where
    C: cipher::BlockCipher + cipher::BlockDecrypt + cipher::KeyInit,
    cbc::Decryptor<C>: KeyIvInit + BlockDecryptMut,
{
    let (Some(iv), Some(rest)) = (data.get(..16), data.get(16..)) else {
        return Vec::new();
    };
    let whole = rest.len() - rest.len() % 16;
    let body = rest.get(..whole).unwrap_or_default();
    let Ok(c) = cbc::Decryptor::<C>::new_from_slices(key, iv) else {
        return data.to_vec();
    };
    let Ok(mut out) = c.decrypt_padded_vec_mut::<NoPadding>(body) else {
        return Vec::new();
    };
    // Strip PKCS#7 padding when it is well formed; keep the bytes otherwise.
    if let Some(&n) = out.last() {
        let n = usize::from(n);
        if (1..=16).contains(&n)
            && n <= out.len()
            && out
                .get(out.len() - n..)
                .is_some_and(|t| t.iter().all(|&b| usize::from(b) == n))
        {
            out.truncate(out.len() - n);
        }
    }
    out
}

fn aes_cbc_encrypt<C>(key: &[u8], data: &[u8]) -> Vec<u8>
where
    C: cipher::BlockCipher + cipher::BlockEncrypt + cipher::KeyInit,
    cbc::Encryptor<C>: KeyIvInit + BlockEncryptMut,
{
    let iv = random(16).unwrap_or_else(|_| vec![0x5A; 16]);
    let Ok(c) = cbc::Encryptor::<C>::new_from_slices(key, &iv) else {
        return data.to_vec();
    };
    let mut out = iv;
    out.extend_from_slice(&c.encrypt_padded_vec_mut::<Pkcs7>(data));
    out
}

pub fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    if !key.is_empty() {
        let mut j: u8 = 0;
        for i in 0..256 {
            let k = key.get(i % key.len()).copied().unwrap_or(0);
            j = j.wrapping_add(s[i]).wrapping_add(k);
            s.swap(i, usize::from(j));
        }
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|&b| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[usize::from(i)]);
            s.swap(usize::from(i), usize::from(j));
            let t = s[usize::from(i)].wrapping_add(s[usize::from(j)]);
            b ^ s[usize::from(t)]
        })
        .collect()
}

pub fn random(n: usize) -> Result<Vec<u8>> {
    let mut b = vec![0u8; n];
    getrandom::getrandom(&mut b)
        .map_err(|e| Error::Invalid(format!("the system random generator failed: {e}")))?;
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc4_known_vector() {
        // "Key" / "Plaintext" from the classic RC4 test vectors.
        assert_eq!(
            rc4(b"Key", b"Plaintext"),
            vec![0xBB, 0xF3, 0x16, 0xE8, 0xD9, 0x40, 0xAF, 0x0A, 0xD3]
        );
    }

    #[test]
    fn aes_roundtrip_and_padding() {
        let key = [7u8; 16];
        for len in [0, 1, 15, 16, 17, 100] {
            let data: Vec<u8> = (0..len as u8).collect();
            let enc = aes_cbc_encrypt::<Aes128>(&key, &data);
            assert_eq!(aes_cbc_decrypt::<Aes128>(&key, &enc), data);
        }
        assert!(aes_cbc_decrypt::<Aes128>(&key, &[1, 2, 3]).is_empty());
    }

    #[test]
    fn permission_bits_follow_acrobat_choices() {
        let s = Settings {
            printing: Printing::Low,
            changes: Changes::Pages,
            copy: false,
            screen_reader: true,
            ..Settings::default()
        };
        let p = permission_bits(&s, 4);
        assert!(p.bit(3) && !p.bit(12));
        assert!(p.bit(11) && !p.bit(4) && !p.bit(5));
        assert!(p.bit(10));
        assert!(p.bit(7) && p.bit(8) && p.bit(13) && p.bit(32));
        assert!(!p.bit(1) && !p.bit(2));
    }
}
