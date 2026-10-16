# Acrobat Pro reference — page display and image export (stage 3)

Sources: Adobe's public help for *Page Display preferences* and *Export PDF to image
formats*, and ISO 32000-2 §8 (graphics). "✔" = implemented, "3b…3e" = the sub-stage that
brings it, **Folio choice** = not documented publicly.

## view.render — drawing pages

| Area (ISO 32000-2) | Folio |
|---|---|
| Paths, fill (nonzero, even-odd), stroke (width, caps, joins, miter, dash) | ✔ 3a |
| Clipping paths, graphics state stack, CTM, Form XObjects (matrix, bbox) | ✔ 3a |
| DeviceGray/RGB/CMYK, CalGray/CalRGB, Lab, ICCBased (by component count), Indexed, Separation, DeviceN | ✔ 3a |
| Functions type 0, 2, 3, 4 (PostScript calculator) | ✔ 3a |
| Constant alpha (`CA`/`ca`) and blend modes from ExtGState | ✔ 3a |
| Page rotation, CropBox clipped to MediaBox | ✔ 3a |
| Images and inline images: 1/2/4/8/16-bit, Decode arrays, every colour space | ✔ 3b |
| JPEG (DCT, incl. Adobe CMYK), CCITT Group 3/4 fax | ✔ 3b, tested with real encoded data |
| JPEG 2000 (JPX, incl. SMaskInData), JBIG2 (with globals) | ✔ 3b, decoded by hayro; covered by the anti-crash gate, no generated samples yet |
| Stencil masks (ImageMask), colour-key masks, explicit masks, soft masks (SMask) | ✔ 3b |
| Image smoothing: smooth when shrinking, crisp when enlarging unless /Interpolate | ✔ 3b |
| Text: all text operators, spacing, horizontal scaling, rise, leading, TJ kerning | ✔ 3c |
| Text rendering modes 0–7 (fill, stroke, invisible, clip) | ✔ 3c |
| TrueType, OpenType (glyf and CFF), bare CFF (Type1C, CIDFontType0C), Type 1 (PFA/PFB, eexec, flex, seac), Type 3 | ✔ 3c, each tested with a synthetic font and against Poppler |
| Composite fonts: Identity-H/V, embedded CMaps, CIDToGIDMap, CID-keyed CFF, W/DW widths | ✔ 3c (vertical writing is drawn horizontally for now) |
| Encodings: Standard, WinAnsi, MacRoman, Differences, TrueType cmap rules (3,1)/(3,0)/(1,0)/post | ✔ 3c |
| Fonts not embedded (standard 14 and others): drawn with an installed metric-compatible font, stretched to the PDF widths | ✔ 3c (**Folio choice**; Acrobat uses its own bundled fonts) |
| Predefined CJK CMaps (UniGB, UniJIS...) | later: needs CMap resources |
| Shadings 1–7: function-based, axial, radial (with Extend), Gouraud triangle meshes, lattices, Coons and tensor patches | ✔ 3d (tensor patches use their boundary curves) |
| Patterns: tiling (coloured and uncoloured), shading patterns, for fills, strokes and text | ✔ 3d |
| Soft masks (luminosity with backdrop, alpha, transfer function), transparency groups composited as a whole | ✔ 3d (knockout and non-isolated groups are drawn as isolated) |
| Annotation appearances (normal appearance, appearance states, Hidden/NoView flags) | ✔ 3d; `--no-annotations` |
| ICC colour management (CMYK through a press profile, as Acrobat does) | later (print.convert-colors); Folio converts DeviceCMYK with the simple formula until then, so CMYK colours look brighter than in Acrobat |
| Overprint preview | later (print.output-preview) |

## Page Display preferences

| Acrobat preference | Folio |
|---|---|
| Smooth line art | ✔ on by default; `--no-smooth-line-art` |
| Enhance thin lines | ✔ on by default; `--no-enhance-thin-lines` |
| Smooth images | ✔ on by default; `--no-smooth-images` |
| Smooth text | ✔ on by default; `--no-smooth-text` |
| (Folio) installed fonts for missing fonts | ✔ on by default; `--no-system-fonts` |
| Resolution | ✔ `--dpi` |

## export.images — Export PDF → Image

| Acrobat option | Folio `folio render` |
|---|---|
| PNG | ✔ |
| JPEG, TIFF | 3e |
| Resolution 72–2400 ppi | ✔ `--dpi` (1–9600 accepted; a page is capped at 268 megapixels) |
| Colour space: automatic / RGB / grayscale | 3e |
| Page range | ✔ `--pages` |
| Output file names `<name>_Page_<n>.png` | ✔ (**Folio choice** of names) |
| Needs content copying to be allowed on protected files | ✔ (**Folio choice**) |
