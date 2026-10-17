# Acrobat Pro reference — Organize Pages, Combine, Properties (stage 1)

What Acrobat Pro offers for each feature, from Adobe's public documentation and
black-box use, and how Folio maps it. Where Acrobat's exact behaviour is not documented
publicly, the line says **Folio choice**. "✔" = implemented in stage 1, "later" = the
stage that brings it.

## organize.rotate — Rotate Pages

| Acrobat option | Folio |
|---|---|
| Direction: 90° clockwise, 90° counterclockwise, 180° | ✔ `--direction cw\|ccw\|180` |
| Page range: All / Selected / From–To | ✔ `--pages` (default all) |
| Rotate: Even and Odd / Even only / Odd only | ✔ `--subset all\|even\|odd` |
| Rotate: Landscape and Portrait / Landscape only / Portrait only | ✔ `--orientation any\|landscape\|portrait` |
| Rotation is stored in `/Rotate`, added to the existing value, modulo 360 | ✔ |

## organize.delete — Delete Pages

| Acrobat option | Folio |
|---|---|
| Page range / selection | ✔ `--pages` |
| Deleting every page is refused (one page must remain) | ✔ clear error |

## organize.extract — Extract Pages

| Acrobat option | Folio |
|---|---|
| Page range | ✔ `--pages` |
| Delete pages after extracting | ✔ `--delete-after` (rewrites the source, needs `--force`) |
| Extract pages as separate files | ✔ `--separate` → `<name> <page>.pdf` (**Folio choice** of file names) |

## organize.insert — Insert Pages

| Acrobat option | Folio |
|---|---|
| From File (with page range) | ✔ `--from FILE [--from-pages]` |
| Blank Page, same size as the page it is placed next to | ✔ `--blank [--count N]` |
| From Clipboard | later (stage 4, desktop app) |
| Location: Before / After | ✔ `--before N` / `--after N` |
| Page: First / Last / Page N | ✔ `first`, `last` or a number |

## organize.replace — Replace Pages

| Acrobat option | Folio |
|---|---|
| Original pages From–To | ✔ `--pages` |
| Replacement file and its start page; count follows the original range | ✔ `--from FILE --from-start N` |

## organize.split — Split Document

| Acrobat option | Folio |
|---|---|
| Number of pages (max pages per file) | ✔ `--pages-per-file N` |
| File size (max MB per file) | ✔ `--max-size MB` (greedy; a single larger page gets its own file) |
| Top-level bookmarks | ✔ `--bookmarks` (file named after the bookmark) |
| Output names `<name>_Part1.pdf`, `_Part2`… | ✔ |
| Output folder | ✔ `--out-dir` |

## organize.merge — Combine Files

| Acrobat option | Folio |
|---|---|
| Files in the chosen order | ✔ |
| Page range per file | ✔ `file.pdf:1-3,7` |
| One bookmark per source file, named after the file, with that file's own bookmarks under it | ✔ |
| Non-PDF inputs (images, Office) | later (stage 8) |
| Form fields with equal names kept apart | later (stage 6) |

## organize.reorder — Reverse / reorder

| Acrobat option | Folio |
|---|---|
| Reverse pages | ✔ `--reverse` |
| Drag pages into a new order (thumbnails) | ✔ `--order 3,1,2,...` (every page exactly once) |

## organize.crop — Crop Pages (Set Page Boxes)

| Acrobat option | Folio |
|---|---|
| Box: CropBox / ArtBox / TrimBox / BleedBox | ✔ `--box crop\|art\|trim\|bleed` |
| Margins Top / Bottom / Left / Right | ✔ `--margins T,B,L,R` (or one value for all) |
| Units (pt, in, mm, cm) | ✔ `--unit` |
| Set to zero (remove the box) | ✔ `--reset` |
| Page range All / From–To; Apply to even and odd / even / odd | ✔ `--pages`, `--subset` |
| Margins measured from the MediaBox | ✔ |
| Remove white margins | ✔ `--remove-white-margins` (each page cropped to its non-white content; blank pages unchanged) |
| Change page size (enlarge MediaBox, offsets, centre) | later (stage 7) |

## organize.page-labels — Page Labels

| Acrobat option | Folio |
|---|---|
| Pages From–To | ✔ `--pages` |
| Begin new section: style None, 1 2 3, i ii iii, I II III, a b c, A B C | ✔ `--style none\|decimal\|roman\|ROMAN\|alpha\|ALPHA` |
| Prefix, Start | ✔ `--prefix`, `--start` |
| Extend numbering used in preceding section | ✔ `--extend` |
| Labels are shown by `folio info` | ✔ |

## view.properties / edit.metadata — Document Properties

| Acrobat field (Description / Advanced) | Folio |
|---|---|
| Title, Author, Subject, Keywords (editable) | ✔ `folio meta --title …` |
| Created, Modified, Application, PDF Producer | ✔ shown; Modified is updated on save of metadata |
| PDF Version, File Size, Page Size, Number of Pages | ✔ |
| Tagged PDF, Fast Web View | ✔ shown (yes/no) |
| Editing keeps the XMP metadata in step with the Info dictionary | ✔ when the file has XMP |
| Custom properties | later (stage 7) |

## optimize.compress — Reduce File Size

| Acrobat option | Folio |
|---|---|
| Compatibility: retain existing | ✔ (Folio keeps the version) |
| Re-compress uncompressed streams, drop unused objects | ✔ |
| Image down-sampling, font subsetting, object streams | later (stage 12) |

## export.text — Save As Text (Plain)

| Acrobat option | Folio |
|---|---|
| Encoding: UTF-8 / UTF-16 | ✔ `--encoding utf8\|utf16` |
| Page range | ✔ `--pages` |
| Reading order from the page (not the content stream order) | partial — stream order with position-based spacing; layout-aware order in stage 13 |
| Text from fonts with ToUnicode, standard encodings, Differences, Type0/Identity-H | ✔ |
