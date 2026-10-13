# Folio — plan de dezvoltare

**Folio** este un atelier PDF open-source, scris de la zero în Rust pur: o reimplementare
„clean-room” a funcțiilor din Adobe Acrobat Pro. Nu are cont, nu are telemetrie, rulează
local pe Windows, macOS și Linux. Licență: `MIT OR Apache-2.0`.

> Folio nu este afiliat cu Adobe. „Acrobat” e folosit doar ca reper de funcționalitate.
> Comportamentul se construiește numai din specificații publice (ISO 32000-2, ISO 19005,
> ISO 14289, RFC 3161/5652, ETSI PAdES) și observație din exterior — vezi `AGENTS.md`.

---

## 1. Inventarul complet al funcțiilor Acrobat Pro

Fiecare funcție de mai jos are un `id` care apare în `parity/acrobat-features.toml`.
Acolo se marchează `planned` → `partial` → `shipped`, iar `shipped` cere un test care
trece (verificat automat de `cargo xtask parity`).

### A. Vizualizare și citire (`view.*`)
| id | Funcție |
|---|---|
| view.open | Deschidere PDF, inclusiv fișiere deteriorate (reparare automată) |
| view.render | Randare pagini: text, vectori, imagini, transparență, culori |
| view.navigate | Navigare pagini, zoom, potrivire lățime/pagină, rotire vizualizare |
| view.layout | O pagină / continuu / două pagini / prezentare (full screen) |
| view.thumbnails | Panou miniaturi |
| view.bookmarks | Semne de carte (outline) — citire și navigare |
| view.search | Căutare text, cu evidențiere; căutare avansată în mai multe fișiere |
| view.select-copy | Selectare și copiere text / imagine |
| view.layers | Straturi (Optional Content Groups) — afișare/ascundere |
| view.attachments | Fișiere atașate — listare, extragere |
| view.tabs | Mai multe documente în taburi |
| view.night-mode | Mod întunecat / culori de citire înlocuite |
| view.read-aloud | Citire cu voce tare (TTS al sistemului) |
| view.properties | Proprietăți document: metadate, fonturi, versiune, securitate |

### B. Organizare pagini (`organize.*`)
| id | Funcție |
|---|---|
| organize.merge | Combinare fișiere |
| organize.split | Împărțire (la N pagini, pe semne de carte, pe dimensiune) |
| organize.extract | Extragere pagini |
| organize.delete | Ștergere pagini |
| organize.rotate | Rotire pagini |
| organize.reorder | Reordonare pagini |
| organize.insert | Inserare pagini (din alt PDF, pagină goală) |
| organize.replace | Înlocuire pagini |
| organize.crop | Decupare pagini (CropBox) |
| organize.page-labels | Etichete de pagină (i, ii, 1, 2…) |

### C. Editare PDF (`edit.*`)
| id | Funcție |
|---|---|
| edit.text | Editare text existent (reflow în bloc) |
| edit.add-text | Adăugare text nou |
| edit.images | Adăugare / înlocuire / mutare / decupare imagini |
| edit.links | Linkuri (URL, pagină, fișier) |
| edit.header-footer | Antet și subsol |
| edit.watermark | Filigran |
| edit.background | Fundal |
| edit.bates | Numerotare Bates |
| edit.bookmarks | Creare / editare semne de carte |
| edit.metadata | Editare metadate (Info + XMP) |

### D. Comentarii și adnotări (`comment.*`)
| id | Funcție |
|---|---|
| comment.note | Notă (sticky note) |
| comment.markup | Evidențiere, subliniere, tăiere text |
| comment.textbox | Casetă text, callout |
| comment.draw | Desen: linie, săgeată, dreptunghi, elipsă, poligon, creion |
| comment.stamp | Ștampile (proprii, desenate de noi) |
| comment.attach | Atașare fișier ca adnotare |
| comment.threads | Fire de discuție, răspunsuri, stare (acceptat/respins) |
| comment.list | Listă comentarii, filtrare, sortare |
| comment.import-export | Import / export FDF și XFDF |
| comment.summary | Rezumat comentarii (PDF generat) |
| comment.flatten | Aplatizare adnotări în conținut |

### E. Formulare (`forms.*`)
| id | Funcție |
|---|---|
| forms.fill | Completare câmpuri (text, bife, radio, liste, combo) |
| forms.prepare | Creare câmpuri, inclusiv detecție automată |
| forms.appearance | Generare aspect câmpuri (appearance streams) |
| forms.calc | Calcule, formate, validări (subset sigur de JavaScript, sandbox) |
| forms.data | Import / export date: FDF, XFDF, CSV, JSON |
| forms.flatten | Aplatizare formular |
| forms.reset | Resetare formular |

### F. Semnare (`sign.*`)
| id | Funcție |
|---|---|
| sign.fill-sign | Semnătură desenată / imagine / inițiale (Fill & Sign) |
| sign.digital | Semnătură digitală PAdES (PKCS#12, token PKCS#11) |
| sign.verify | Validare semnături, lanț de certificate, revocare (OCSP/CRL) |
| sign.timestamp | Marcă temporală RFC 3161 |
| sign.ltv | Validare pe termen lung (DSS, LTV) |
| sign.certify | Certificare document (DocMDP) |

### G. Protecție (`protect.*`)
| id | Funcție |
|---|---|
| protect.decrypt | Deschidere fișiere criptate (RC4, AES-128, AES-256) |
| protect.encrypt | Criptare cu parolă (deschidere + permisiuni) |
| protect.permissions | Restricții: tipărire, copiere, editare |
| protect.cert-encrypt | Criptare cu certificat |
| protect.redact | Redactare reală (eliminare definitivă din conținut) |
| protect.search-redact | Căutare și redactare după tipare (CNP, e-mail, IBAN…) |
| protect.sanitize | Eliminare informații ascunse (metadate, scripturi, straturi ascunse) |

### H. Creare și export (`create.*`, `export.*`)
| id | Funcție |
|---|---|
| create.images | PDF din imagini (PNG, JPEG, TIFF) |
| create.blank | Document gol |
| create.office | PDF din DOCX / XLSX / PPTX (motor propriu de layout) |
| create.html | PDF din HTML / pagină web salvată |
| create.text | PDF din text simplu / Markdown |
| create.clipboard | PDF din clipboard |
| export.images | Export pagini ca PNG / JPEG / TIFF |
| export.text | Export text (simplu, cu layout) |
| export.html | Export HTML |
| export.docx | Export Word |
| export.xlsx | Export Excel (tabele) |
| export.pptx | Export PowerPoint |
| export.extract-images | Extragere imagini din document |

### I. Scanare și OCR (`ocr.*`)
| id | Funcție |
|---|---|
| ocr.recognize | Recunoaștere text → strat de text invizibil (PDF căutabil) |
| ocr.enhance | Îmbunătățire scanări: deskew, curățare fundal, contrast |
| ocr.languages | Română, maghiară, engleză + alte limbi |

### J. Optimizare (`optimize.*`)
| id | Funcție |
|---|---|
| optimize.compress | Reducere dimensiune (recomprimare stream-uri, eliminare obiecte neutilizate) |
| optimize.images | Reeșantionare și recomprimare imagini |
| optimize.fonts | Subsetare / eliminare fonturi duplicate |
| optimize.objstm | Object streams și xref streams |
| optimize.linearize | Fast Web View (linearizare) |
| optimize.audit | Audit spațiu (ce ocupă loc) |

### K. Standarde și accesibilitate (`standards.*`, `access.*`)
| id | Funcție |
|---|---|
| standards.pdfa-validate | Validare PDF/A-1/2/3 |
| standards.pdfa-convert | Conversie la PDF/A |
| standards.pdfx | Validare PDF/X |
| standards.pdfua | Validare PDF/UA |
| access.check | Verificare accesibilitate |
| access.autotag | Etichetare automată (tags, ordine de citire) |
| access.alt-text | Text alternativ pentru imagini |

### L. Unelte avansate (`tools.*`)
| id | Funcție |
|---|---|
| tools.compare | Comparare două versiuni (text și vizual) |
| tools.actions | Action Wizard: secvențe de pași rulate pe multe fișiere |
| tools.batch | Procesare în lot pe foldere |
| tools.measure | Măsurare distanță, perimetru, arie (cu scară) |
| tools.index | Index de căutare pe colecții de PDF-uri |
| tools.portfolio | Portofolii PDF (colecții) |
| tools.print | Tipărire, inclusiv broșură și mai multe pagini pe foaie |

### M. Producție tipar (`print.*`)
| id | Funcție |
|---|---|
| print.preflight | Preflight (profile de verificare) |
| print.output-preview | Previzualizare separații, acoperire cerneală |
| print.convert-colors | Conversie culori (ICC) |
| print.marks | Semne de tăiere, bleed |
| print.hairlines | Corectare linii prea subțiri |

### N. Rich media (`media.*`) — prioritate scăzută
| id | Funcție |
|---|---|
| media.av | Audio / video încorporat (redare externă) |
| media.3d | Conținut 3D (doar afișare listă / extragere) |

### O. Agenți și automatizare (`agent.*`) — diferențiatorul Folio
| id | Funcție |
|---|---|
| agent.cli | Fiecare funcție disponibilă în linia de comandă `folio` |
| agent.json | Ieșire JSON pentru orice comandă |
| agent.mcp | Server MCP opțional (pornit doar explicit, doar loopback) |
| agent.script | Scripturi de pași (aceleași ca Action Wizard) |

### În afara scopului (intenționat)
- Servicii cloud Adobe (Document Cloud, „Request e-signatures”, partajare online, cont).
- Asistentul AI generativ al Adobe. (Agenții externi pot folosi `agent.*`.)
- XFA dinamic (formulare LiveCycle) — doar detectare și mesaj clar.

---

## 1b. Fidelitate față de Acrobat Pro

Fiecare funcție se construiește **fidel Acrobat Pro**: aceleași opțiuni, aceleași valori
implicite, același rezultat în fișier. Nu e de ajuns ca „ceva asemănător” să meargă.

- Înainte de implementare, opțiunile funcției din Acrobat se scriu în
  `docs/acrobat/<grup>.md` (din documentația publică Adobe și observație din exterior,
  fără capturi de ecran publicate — vezi `AGENTS.md`).
- Fiecare opțiune devine un parametru în motor și în comanda `folio`, cu un test.
- În `parity/acrobat-features.toml`, `notes` listează opțiunile Acrobat care **lipsesc**
  încă. `shipped` = toate opțiunile Acrobat ale funcției există și sunt testate.
- Diferențele intenționate (de ex. lipsa contului Adobe) se scriu explicit.

## 2. Arhitectura

```
crates/
  folio-core     obiecte PDF, lexer, parser tolerant + reparare, filtre, writer   (Etapa 0)
  folio-ops      operații pe documente: pagini, metadate, text, comprimare       (Etapa 1)
  folio-crypt    criptare / decriptare (RC4, AES, hash-uri R2–R6)                (Etapa 2)
  folio-font     TrueType / OpenType / CFF / Type1 / Type3, CMaps                 (Etapa 3)
  folio-image    JPEG, JPEG2000, JBIG2, CCITT, PNG — decodare                     (Etapa 3)
  folio-render   rasterizare: căi, clipping, transparență, spații de culoare      (Etapa 3)
  folio-annot    adnotări                                                         (Etapa 5)
  folio-forms    AcroForm, aspecte, calcule (sandbox)                             (Etapa 6)
  folio-edit     editare conținut, antet/subsol, filigran, Bates                  (Etapa 7)
  folio-convert  creare din imagini/text/Office/HTML; export DOCX/XLSX/PPTX/HTML  (Etapa 8)
  folio-sign     PAdES, CMS, X.509, RFC 3161, OCSP/CRL                            (Etapa 9)
  folio-ocr      OCR în Rust pur                                                  (Etapa 11)
  folio-std      PDF/A, PDF/UA, PDF/X, accesibilitate                             (Etapa 13)
apps/
  folio-cli      linia de comandă `folio` (+ `folio mcp`)
  folio-app      aplicația desktop (egui + wgpu)                                  (Etapa 4)
xtask/           agentul de verificare: `cargo xtask ci`
parity/          acrobat-features.toml — evidența onestă a parității
```

**Ediția Rust:** `2024` — cea mai nouă ediție stabilă (cargo 1.99 nu cunoaște încă `2026`).
Când apare ediția următoare stabilă, migrăm cu `cargo fix --edition` într-un PR separat.

Reguli de dependențe: `core` nu depinde de nimic din Folio; `ops` și celelalte depind de
`core`; aplicațiile depind de crate-uri, niciodată invers. `cargo xtask` verifică.

---

## 3. Agentul de verificare (pentru contribuitori)

Orice contribuție (om sau agent AI) trece prin aceleași porți înainte de `main`:

1. **`cargo xtask ci`** — același lucru local și în CI:
   `fmt --check` → `clippy -D warnings` (cu regulile never-crash) → toate testele →
   verificare paritate (fiecare `shipped` are test existent) → verificare asset-uri
   (orice imagine/font/PDF are intrare în `ATTRIBUTION.toml`) → verificare că fiecare
   crate moștenește regulile workspace (fără `unsafe`, fără `unwrap` în cod de producție).
2. **Teste pe Windows, macOS, Linux.**
3. **DCO** — fiecare commit are `Signed-off-by:` (contribuitorul certifică proveniența).
4. **Checklist PR** — secțiunile Never-crash și Proveniență bifate.
5. **Agent de review AI** — Claude citește diff-ul, verifică regulile din `AGENTS.md`
   și lasă comentarii în PR. Pentru PR-uri din fork-uri pornește la comanda
   `/agent-review` dată de un maintainer.
6. **Testul anti-crash** — `cargo xtask crash` (vezi secțiunea 4).
7. **Protecția ramurii `main`** — merge doar cu verificările de mai sus trecute.

---

## 4. Etapele, pas cu pas

Fiecare etapă se termină cu: cod + teste + intrări `shipped` în paritate + CI verde
**+ testul anti-crash** (mai jos).

### Testul anti-crash — poarta de la sfârșitul fiecărei etape

O etapă **nu se închide** până când `cargo xtask crash` nu trece. Testul:

1. Generează documente valide care exercită tot ce a adus etapa (pagini, stream-uri
   comprimate, object streams, fonturi, adnotări, câmpuri… după caz).
2. Le corupe determinist, în zeci de mii de variante: octeți schimbați, trunchieri,
   bucăți duplicate sau șterse, numere uriașe/negative, imbricare adâncă, xref greșit.
3. Rulează **fiecare funcție nouă a etapei** pe fiecare variantă (deschidere, scriere,
   operații pe pagini, extragere text, randare, formulare…).
4. Trece doar dacă **nicio variantă nu produce panică, blocare sau consum nelimitat de
   memorie** — orice problemă trebuie să fie o eroare clară, nu un crash.
5. Orice crash găsit devine un test de regresie mic, păstrat permanent în suită.

Rulare: `cargo xtask crash` (implicit 20 000 de variante; `--iters N` pentru mai multe).
În CI rulează la fiecare PR într-o variantă scurtă și complet la închiderea etapei.
Rezultatul se notează în `docs/crash-log.md` (data, etapa, număr variante, rezultat).

### Etapa 0 — Fundament *(în lucru)*
- Repo GitHub, workspace Rust, licențe, `AGENTS.md`, `CONTRIBUTING.md`, șablon PR.
- Agentul de verificare complet (secțiunea 3) + protecția ramurii.
- `folio-core`: lexer, parser tolerant, xref clasic + xref streams + object streams,
  reconstrucție la fișiere stricate, filtre (Flate, LZW, ASCIIHex, ASCII85, RunLength,
  predictori PNG), writer atomic, arbore de pagini, curățare obiecte neutilizate.
- Test „fuzz” determinist: mii de fișiere corupte nu pot produce panică.

### Etapa 1 — Organizare pagini + linia de comandă
- `folio info | merge | split | extract | delete | rotate | reorder | insert | meta |
  compress | text | check`, toate cu `--json`.
- Intervale de pagini (`1-3,5,8-`), suprascriere doar cu `--force`.
- Extragere text de bază (ToUnicode, WinAnsi, Form XObjects).

### Etapa 2 — Securitate
- Decriptare RC4/AES-128/AES-256 (handler standard R2–R6), parolă utilizator/proprietar.
- Criptare cu parolă și permisiuni. `folio decrypt`, `folio encrypt`.

### Etapa 3 — Motorul de randare
- Fonturi (TrueType, CFF, Type1, Type3), imagini (JPEG, JPX, JBIG2, CCITT),
  spații de culoare (Device*, ICC, Indexed, Separation, DeviceN), umbriri, transparență.
- Rasterizator propriu cu antialiasing. `folio render --dpi 150` → PNG.
- Comparare vizuală cu un oracol extern (proces separat) pe un corpus de test.

### Etapa 4 — Aplicația desktop (vizualizare)
- Fereastră egui/wgpu: taburi, miniaturi, semne de carte, zoom, moduri de afișare,
  căutare cu evidențiere, selectare/copiere, straturi, atașamente, proprietăți, mod întunecat.
- Canal de control opțional pentru testare automată a interfeței.

### Etapa 5 — Comentarii
- Toate tipurile de adnotări, fire de discuție, listă, import/export FDF/XFDF, aplatizare.

### Etapa 6 — Formulare
- Completare, creare câmpuri, aspecte, date FDF/XFDF/CSV/JSON, aplatizare,
  calcule într-un interpretor JavaScript minimal, izolat.

### Etapa 7 — Editare conținut
- Editare text și imagini, linkuri, antet/subsol, filigran, fundal, Bates,
  decupare, etichete de pagină, semne de carte.

### Etapa 8 — Creare și export
- PDF din imagini, text, Markdown, HTML; PDF din DOCX/XLSX/PPTX.
- Export PNG/JPEG/TIFF, text, HTML, DOCX, XLSX, PPTX; extragere imagini.

### Etapa 9 — Semnături digitale
- Fill & Sign; semnare PAdES (PKCS#12), verificare, marcă temporală, LTV, certificare.

### Etapa 10 — Redactare și sanitizare
- Redactare reală, căutare după tipare (CNP, IBAN, e-mail, telefon), eliminare date ascunse.

### Etapa 11 — OCR
- Detecție + recunoaștere în Rust pur, strat de text invizibil, deskew, limbile RO/HU/EN.

### Etapa 12 — Optimizare avansată
- Imagini, fonturi, object streams, linearizare, audit spațiu.

### Etapa 13 — Standarde și accesibilitate
- PDF/A validare + conversie, PDF/UA, PDF/X, verificare accesibilitate, autotag, alt text.

### Etapa 14 — Unelte avansate + agenți
- Comparare documente, Action Wizard, lot, index, portofolii, tipărire avansată,
  măsurători, server MCP.

### Etapa 15 — Producție tipar și rich media
- Preflight, output preview, conversie culori, semne de tăiere, linii subțiri; media.

### Etapa 16 — Distribuție
- Instalatoare (MSI Windows, DMG macOS, .deb/.rpm/AppImage Linux), semnare binare,
  release automat din tag-uri, site cu documentație.

---

## 5. Cum se adaugă o funcție (pentru orice etapă)

1. Alege `id`-ul din secțiunea 1; pune-l `partial` în `parity/acrobat-features.toml`.
2. Implementează în crate-ul potrivit, cu `Result`, fără panică.
3. Expune-o în `folio` (CLI) cu `--json`.
4. Scrie testul; adaugă-i numele în `evidence`; treci la `shipped` când e complet.
5. `cargo xtask ci` local → PR → agentul de verificare → merge.
