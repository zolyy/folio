# Acrobat Pro reference — Protect: passwords and permissions (stage 2)

Sources: Adobe's public help for *Securing PDFs with passwords* and the standard security
handler in ISO 32000-2 §7.6. "✔" = implemented in stage 2, "later" = the stage that brings
it, **Folio choice** = Acrobat's exact behaviour is not publicly documented.

## protect.decrypt — opening protected files

| Acrobat behaviour | Folio |
|---|---|
| Files with an empty open password open without a prompt | ✔ |
| Otherwise asks for a password; the permissions (owner) password also opens the file | ✔ `--password` or `FOLIO_PASSWORD` |
| Wrong password: the file stays closed | ✔ clear error, exit code 3 |
| RC4 40-bit, RC4 128-bit, AES-128, AES-256 (R2, R3, R4, R5, R6) | ✔ |
| Restrictions are enforced when the file was opened without the permissions password | ✔ every command checks the permission it needs |
| Saving a protected file keeps its security | ✔ |
| Security Method: No Security (remove) — needs the permissions password when one is set | ✔ `folio decrypt` |
| Certificate security, Adobe LiveCycle rights management | later (stage 9) / out of scope |

## protect.encrypt — Encrypt with Password (Password Security – Settings)

| Acrobat option | Folio `folio encrypt` |
|---|---|
| Compatibility: Acrobat 3.0 and later (40-bit RC4) | ✔ `--compat acrobat3` |
| Compatibility: Acrobat 5.0 and later (128-bit RC4) | ✔ `--compat acrobat5` |
| Compatibility: Acrobat 6.0 and later (128-bit AES) | ✔ `--compat acrobat6` |
| Compatibility: Acrobat X and later (256-bit AES) — the default | ✔ `--compat acrobatx` |
| Encrypt all document contents | ✔ `--encrypt all` (default) |
| Encrypt all document contents except metadata (Acrobat 6.0 and later) | ✔ `--encrypt except-metadata` |
| Encrypt only file attachments (Acrobat 7.0 and later) | ✔ `--encrypt attachments` (needs an open password) |
| Require a password to open the document | ✔ `--open-password` |
| Restrict editing and printing; change permissions password | ✔ `--permissions-password` (required with any restriction) |
| Open and permissions passwords must differ | ✔ refused with a clear message |
| Printing Allowed: None / Low Resolution (150 dpi) / High Resolution | ✔ `--printing none\|low\|high` |
| Changes Allowed: None | ✔ `--changes none` |
| Changes Allowed: Inserting, deleting, and rotating pages | ✔ `--changes pages` |
| Changes Allowed: Filling in form fields and signing existing signature fields | ✔ `--changes forms` |
| Changes Allowed: Commenting, filling in form fields, and signing | ✔ `--changes comments` |
| Changes Allowed: Any except extracting pages | ✔ `--changes any` |
| Enable copying of text, images, and other content | ✔ on unless `--no-copy` |
| Enable text access for screen reader devices | ✔ on unless `--no-screen-reader` |
| 40-bit RC4 offers only the four basic permissions | ✔ as in the specification |

## protect.permissions — Document Restrictions Summary

`folio security FILE` prints the same rows as Acrobat's summary: Printing, Changing the
Document, Document Assembly, Content Copying, Content Copying for Accessibility, Page
Extraction, Commenting, Filling of form fields, Signing, Creation of Template Pages.

Mapping of permission bits (ISO 32000-2 Table 22) to Folio commands:

| Restriction | Bits | Folio commands that need it |
|---|---|---|
| Document Assembly | 11 (or 4) | rotate, delete, insert, replace, reorder, crop, labels |
| Page Extraction | 4 and 5 (**Folio choice**) | extract, split, merge (as a source) |
| Changing the Document | 4 | meta, compress |
| Content Copying | 5 | text |
| Printing | 3 (12 for high quality) | later (stage 14, printing) |
| Commenting / Forms / Signing | 6, 9 | later (stages 5, 6, 9) |
| Changing security | permissions password | encrypt on a protected file |
