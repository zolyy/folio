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
| Images and inline images, image masks, soft masks in images | 3b |
| Text and fonts | 3c |
| Shadings, patterns, transparency groups, soft masks | 3d |
| ICC colour management (CMYK through a press profile, as Acrobat does) | later (print.convert-colors); Folio converts DeviceCMYK with the simple formula until then, so CMYK colours look brighter than in Acrobat |
| Overprint preview | later (print.output-preview) |

## Page Display preferences

| Acrobat preference | Folio |
|---|---|
| Smooth line art | ✔ on by default; `--no-smooth-line-art` |
| Enhance thin lines | ✔ on by default; `--no-enhance-thin-lines` |
| Smooth text, Smooth images | 3c, 3b |
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
