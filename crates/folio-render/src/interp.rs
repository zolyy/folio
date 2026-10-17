//! The content-stream interpreter (ISO 32000-2 §8): graphics state, paths,
//! painting, clipping, colour operators and Form XObjects.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use folio_core::filters::decode_stream;
use folio_core::lexer::Token;
use folio_core::parser::Parser;
use folio_core::{Dict, Document, Error, ObjRef, Object, Result};
use tiny_skia::{
    BlendMode, Color, FillRule, FilterQuality, IntRect, LineCap, LineJoin, Mask, Paint,
    PathBuilder, Pattern, Pixmap, PixmapPaint, SpreadMode, Stroke, StrokeDash, Transform,
};

use crate::color::ColorSpace;
use crate::font::PdfFont;
use crate::shading::Shading;
use crate::{MAX_PIXELS, Options, Rendered};

/// Operators per page before giving up (hostile content streams).
const MAX_OPS: usize = 2_000_000;
/// Nesting of q/Q and of Form XObjects.
const MAX_STACK: usize = 256;
const MAX_FORM_DEPTH: usize = 16;
/// Decoded image pixels per page (repeated huge images are refused).
const MAX_PAGE_IMAGE_PIXELS: u64 = 400_000_000;
/// Transparency layers (groups, soft masks) only for pages up to this size.
const MAX_LAYER_PIXELS: u64 = 40_000_000;
/// Path segments per path.
const MAX_SEGMENTS: usize = 1_000_000;

pub(crate) struct PageGeometry {
    pub width: u32,
    pub height: u32,
    /// Default user space → pixels.
    pub base: Transform,
}

fn rect_of(doc: &Document, o: &Object) -> Option<[f64; 4]> {
    let a = o.as_array()?;
    let v: Vec<f64> = a.iter().filter_map(|x| doc.resolve(x).as_f64()).collect();
    match v.as_slice() {
        [a, b, c, d] => Some([a.min(*c), b.min(*d), a.max(*c), b.max(*d)]),
        _ => None,
    }
}

/// The visible area of a page (crop box clipped to the media box, normalised)
/// and its `/Rotate` as 0, 90, 180 or 270.
pub(crate) fn visible_box(doc: &Document, page: ObjRef) -> ([f64; 4], i64) {
    let media = {
        let m = doc.media_box(page);
        [
            m[0].min(m[2]),
            m[1].min(m[3]),
            m[0].max(m[2]),
            m[1].max(m[3]),
        ]
    };
    let b = match doc
        .page_attr(page, b"CropBox")
        .and_then(|o| rect_of(doc, &o))
    {
        // The visible area is the crop box clipped to the media box.
        Some(c) => [
            c[0].max(media[0]),
            c[1].max(media[1]),
            c[2].min(media[2]),
            c[3].min(media[3]),
        ],
        None => media,
    };
    let b = if b[2] - b[0] < 1.0 || b[3] - b[1] < 1.0 {
        media
    } else {
        b
    };
    let rot = doc
        .page_attr(page, b"Rotate")
        .and_then(|o| o.as_int())
        .unwrap_or(0)
        .rem_euclid(360);
    (b, rot - rot % 90)
}

impl PageGeometry {
    pub fn new(doc: &Document, page: ObjRef, dpi: f64) -> Result<Self> {
        if !dpi.is_finite() || !(1.0..=9600.0).contains(&dpi) {
            return Err(Error::Invalid(format!(
                "resolution must be between 1 and 9600 dpi, not {dpi}"
            )));
        }
        let (b, rot) = visible_box(doc, page);
        let s = dpi / 72.0;
        let (w_pt, h_pt) = (b[2] - b[0], b[3] - b[1]);
        let (dw, dh) = if rot % 180 == 90 {
            (h_pt, w_pt)
        } else {
            (w_pt, h_pt)
        };
        let width = (dw * s).ceil().clamp(1.0, 1e9) as u64;
        let height = (dh * s).ceil().clamp(1.0, 1e9) as u64;
        if width.saturating_mul(height) > MAX_PIXELS || width > 65_535 || height > 65_535 {
            return Err(Error::Limit(format!(
                "this page would be {width} x {height} pixels at {dpi} dpi; lower the resolution"
            )));
        }
        let (x0, y0, x1, y1) = (b[0], b[1], b[2], b[3]);
        let f = |v: f64| v as f32;
        let base = match rot {
            90 => Transform::from_row(0.0, f(s), f(s), 0.0, f(-y0 * s), f(-x0 * s)),
            180 => Transform::from_row(f(-s), 0.0, 0.0, f(s), f(x1 * s), f(-y0 * s)),
            270 => Transform::from_row(0.0, f(-s), f(-s), 0.0, f(y1 * s), f(x1 * s)),
            _ => Transform::from_row(f(s), 0.0, 0.0, f(-s), f(-x0 * s), f(y1 * s)),
        };
        Ok(PageGeometry {
            width: width as u32,
            height: height as u32,
            base,
        })
    }
}

#[derive(Clone)]
struct GState {
    ctm: Transform,
    clip: Option<Rc<Mask>>,
    /// Soft mask from the ExtGState (§11.6.5.2), in device space.
    smask: Option<Rc<Mask>>,
    /// clip × soft mask: what painting uses.
    mask: Option<Rc<Mask>>,
    fill_pattern: Option<Rc<Object>>,
    stroke_pattern: Option<Rc<Object>>,
    fill_cs: Rc<ColorSpace>,
    fill: Vec<f64>,
    stroke_cs: Rc<ColorSpace>,
    stroke: Vec<f64>,
    line_width: f32,
    cap: LineCap,
    join: LineJoin,
    miter: f32,
    dash: Vec<f32>,
    dash_phase: f32,
    fill_alpha: f32,
    stroke_alpha: f32,
    blend: BlendMode,
    font: Option<Rc<PdfFont>>,
    font_size: f32,
    char_space: f32,
    word_space: f32,
    hscale: f32,
    leading: f32,
    rise: f32,
    render_mode: u8,
}

impl GState {
    fn new(ctm: Transform) -> Self {
        GState {
            ctm,
            clip: None,
            smask: None,
            mask: None,
            fill_pattern: None,
            stroke_pattern: None,
            fill_cs: Rc::new(ColorSpace::Gray),
            fill: vec![0.0],
            stroke_cs: Rc::new(ColorSpace::Gray),
            stroke: vec![0.0],
            line_width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter: 10.0,
            dash: Vec::new(),
            dash_phase: 0.0,
            fill_alpha: 1.0,
            stroke_alpha: 1.0,
            blend: BlendMode::SourceOver,
            font: None,
            font_size: 12.0,
            char_space: 0.0,
            word_space: 0.0,
            hscale: 1.0,
            leading: 0.0,
            rise: 0.0,
            render_mode: 0,
        }
    }
}

struct Ctx<'a> {
    doc: &'a Document,
    opts: &'a Options,
    pixmap: Pixmap,
    ops: usize,
    skipped: BTreeMap<&'static str, usize>,
    forms_active: HashSet<u32>,
    images: HashMap<u32, Rc<Pixmap>>,
    image_pixels: u64,
    fonts: HashMap<u32, Rc<PdfFont>>,
    /// Default user space → device (patterns are anchored to it).
    base: Transform,
    /// Uncoloured tiling patterns paint in this colour.
    forced: Option<Color>,
    pattern_depth: usize,
}

pub fn render(doc: &Document, page: ObjRef, opts: &Options) -> Result<Rendered> {
    let geo = PageGeometry::new(doc, page, opts.dpi)?;
    let mut pixmap = Pixmap::new(geo.width, geo.height).ok_or_else(|| {
        Error::Limit(format!(
            "could not allocate a {} x {} image",
            geo.width, geo.height
        ))
    })?;
    if opts.white_background {
        pixmap.fill(Color::WHITE);
    }
    let mut ctx = Ctx {
        doc,
        opts,
        pixmap,
        ops: 0,
        skipped: BTreeMap::new(),
        forms_active: HashSet::new(),
        images: HashMap::new(),
        image_pixels: 0,
        fonts: HashMap::new(),
        base: geo.base,
        forced: None,
        pattern_depth: 0,
    };
    let res = doc
        .page_attr(page, b"Resources")
        .and_then(|o| o.as_dict().cloned())
        .unwrap_or_default();
    let content = page_content(doc, page);
    ctx.run(&content, &res, GState::new(geo.base), 0);
    if opts.annotations {
        ctx.annotations(page);
    }
    Ok(Rendered {
        pixmap: ctx.pixmap,
        skipped: ctx.skipped.into_iter().collect(),
    })
}

fn page_content(doc: &Document, page: ObjRef) -> Vec<u8> {
    let Some(d) = doc.get(page).and_then(Object::as_dict) else {
        return Vec::new();
    };
    let parts: Vec<&Object> = match d.get(b"Contents").map(|c| doc.resolve(c)) {
        Some(Object::Array(a)) => a.iter().collect(),
        Some(o @ Object::Stream(_)) => vec![o],
        _ => Vec::new(),
    };
    let mut out = Vec::new();
    for p in parts {
        if let Object::Stream(s) = doc.resolve(p)
            && let Ok(data) = decode_stream(s)
        {
            out.extend_from_slice(&data);
            out.push(b'\n');
        }
    }
    out
}

fn finite(v: f64) -> f32 {
    if v.is_finite() {
        v.clamp(-1e7, 1e7) as f32
    } else {
        0.0
    }
}

fn blend_mode(name: &[u8]) -> BlendMode {
    match name {
        b"Multiply" => BlendMode::Multiply,
        b"Screen" => BlendMode::Screen,
        b"Overlay" => BlendMode::Overlay,
        b"Darken" => BlendMode::Darken,
        b"Lighten" => BlendMode::Lighten,
        b"ColorDodge" => BlendMode::ColorDodge,
        b"ColorBurn" => BlendMode::ColorBurn,
        b"HardLight" => BlendMode::HardLight,
        b"SoftLight" => BlendMode::SoftLight,
        b"Difference" => BlendMode::Difference,
        b"Exclusion" => BlendMode::Exclusion,
        b"Hue" => BlendMode::Hue,
        b"Saturation" => BlendMode::Saturation,
        b"Color" => BlendMode::Color,
        b"Luminosity" => BlendMode::Luminosity,
        _ => BlendMode::SourceOver,
    }
}

impl Ctx<'_> {
    fn skip(&mut self, what: &'static str) {
        *self.skipped.entry(what).or_insert(0) += 1;
    }

    fn run(&mut self, content: &[u8], res: &Dict, initial: GState, depth: usize) {
        let mut gs = initial;
        let mut stack: Vec<GState> = Vec::new();
        let mut path = PathBuilder::new();
        let mut segments = 0usize;
        let mut cur = (0.0f32, 0.0f32);
        let mut pending_clip: Option<FillRule> = None;
        let mut operands: Vec<Object> = Vec::new();
        let mut tm = Transform::identity();
        let mut tlm = Transform::identity();
        let mut text_clip: Vec<tiny_skia::Path> = Vec::new();
        let mut p = Parser::content(content);
        loop {
            let at = p.lex.pos;
            let tok = match p.lex.token() {
                Ok(Some(t)) => t,
                _ => break,
            };
            let op = match tok {
                Token::Keyword(k) => k,
                t => {
                    match p.from_token(t, 0, at) {
                        Ok(o) if operands.len() < 64 => operands.push(o),
                        Ok(_) => {}
                        Err(_) => break,
                    }
                    continue;
                }
            };
            self.ops += 1;
            if self.ops > MAX_OPS {
                self.skip("operators beyond the safety limit");
                break;
            }
            let n = |i: usize| -> f32 {
                finite(operands.get(i).and_then(Object::as_f64).unwrap_or(0.0))
            };
            let all: Vec<f64> = operands.iter().filter_map(Object::as_f64).collect();
            let mut paint: Option<(bool, bool, FillRule, bool)> = None; // fill, stroke, rule, close
            match op.as_slice() {
                b"q" => {
                    if stack.len() < MAX_STACK {
                        stack.push(gs.clone());
                    }
                }
                b"Q" => {
                    if let Some(s) = stack.pop() {
                        gs = s;
                    }
                }
                b"cm" => {
                    let m = Transform::from_row(n(0), n(1), n(2), n(3), n(4), n(5));
                    gs.ctm = gs.ctm.pre_concat(m);
                }
                b"w" => gs.line_width = n(0).abs(),
                b"J" => {
                    gs.cap = match n(0) as i32 {
                        1 => LineCap::Round,
                        2 => LineCap::Square,
                        _ => LineCap::Butt,
                    }
                }
                b"j" => {
                    gs.join = match n(0) as i32 {
                        1 => LineJoin::Round,
                        2 => LineJoin::Bevel,
                        _ => LineJoin::Miter,
                    }
                }
                b"M" => gs.miter = n(0).max(1.0),
                b"d" => {
                    gs.dash = operands
                        .first()
                        .and_then(Object::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(Object::as_f64)
                                .map(|v| finite(v).abs())
                                .take(64)
                                .collect()
                        })
                        .unwrap_or_default();
                    gs.dash_phase = n(1);
                }
                b"gs" => {
                    if let Some(name) = operands.first().and_then(Object::as_name) {
                        let name = name.to_vec();
                        self.ext_gstate(res, &name, &mut gs);
                    }
                }
                b"m" => {
                    if segments < MAX_SEGMENTS {
                        path.move_to(n(0), n(1));
                        segments += 1;
                    }
                    cur = (n(0), n(1));
                }
                b"l" => {
                    if segments < MAX_SEGMENTS {
                        path.line_to(n(0), n(1));
                        segments += 1;
                    }
                    cur = (n(0), n(1));
                }
                b"c" => {
                    if segments < MAX_SEGMENTS {
                        path.cubic_to(n(0), n(1), n(2), n(3), n(4), n(5));
                        segments += 1;
                    }
                    cur = (n(4), n(5));
                }
                b"v" => {
                    if segments < MAX_SEGMENTS {
                        path.cubic_to(cur.0, cur.1, n(0), n(1), n(2), n(3));
                        segments += 1;
                    }
                    cur = (n(2), n(3));
                }
                b"y" => {
                    if segments < MAX_SEGMENTS {
                        path.cubic_to(n(0), n(1), n(2), n(3), n(2), n(3));
                        segments += 1;
                    }
                    cur = (n(2), n(3));
                }
                b"h" => path.close(),
                b"re" => {
                    let (x, y, w, h) = (n(0), n(1), n(2), n(3));
                    if segments < MAX_SEGMENTS {
                        path.move_to(x, y);
                        path.line_to(x + w, y);
                        path.line_to(x + w, y + h);
                        path.line_to(x, y + h);
                        path.close();
                        segments += 4;
                    }
                    cur = (x, y);
                }
                b"S" => paint = Some((false, true, FillRule::Winding, false)),
                b"s" => paint = Some((false, true, FillRule::Winding, true)),
                b"f" | b"F" => paint = Some((true, false, FillRule::Winding, false)),
                b"f*" => paint = Some((true, false, FillRule::EvenOdd, false)),
                b"B" => paint = Some((true, true, FillRule::Winding, false)),
                b"B*" => paint = Some((true, true, FillRule::EvenOdd, false)),
                b"b" => paint = Some((true, true, FillRule::Winding, true)),
                b"b*" => paint = Some((true, true, FillRule::EvenOdd, true)),
                b"n" => paint = Some((false, false, FillRule::Winding, false)),
                b"W" => pending_clip = Some(FillRule::Winding),
                b"W*" => pending_clip = Some(FillRule::EvenOdd),
                b"CS" | b"cs" => {
                    let cs = operands
                        .first()
                        .and_then(|o| ColorSpace::parse(self.doc, o, Some(res)))
                        .unwrap_or(ColorSpace::Gray);
                    let init = cs.initial();
                    if op.as_slice() == b"CS" {
                        gs.stroke_cs = Rc::new(cs);
                        gs.stroke = init;
                    } else {
                        gs.fill_cs = Rc::new(cs);
                        gs.fill = init;
                    }
                }
                b"SC" | b"SCN" => {
                    gs.stroke = all;
                    gs.stroke_pattern = operands
                        .last()
                        .and_then(Object::as_name)
                        .and_then(|n| self.pattern(res, n));
                }
                b"sc" | b"scn" => {
                    gs.fill = all;
                    gs.fill_pattern = operands
                        .last()
                        .and_then(Object::as_name)
                        .and_then(|n| self.pattern(res, n));
                }
                b"G" | b"g" | b"RG" | b"rg" | b"K" | b"k" => {
                    let cs = match op.as_slice() {
                        b"G" | b"g" => ColorSpace::Gray,
                        b"RG" | b"rg" => ColorSpace::Rgb,
                        _ => ColorSpace::Cmyk,
                    };
                    if op.first().is_some_and(u8::is_ascii_uppercase) {
                        gs.stroke_cs = Rc::new(cs);
                        gs.stroke = all;
                    } else {
                        gs.fill_cs = Rc::new(cs);
                        gs.fill = all;
                    }
                }
                b"Do" => {
                    if let Some(name) = operands.first().and_then(Object::as_name) {
                        let name = name.to_vec();
                        self.xobject(res, &name, &gs, depth);
                    }
                }
                b"BI" => {
                    // Inline image: abbreviated dictionary, `ID`, data, `EI`.
                    let mut d = Dict::new();
                    let mut ok = false;
                    loop {
                        let at = p.lex.pos;
                        match p.lex.token() {
                            Ok(Some(Token::Keyword(k))) if k == b"ID" => {
                                ok = true;
                                break;
                            }
                            Ok(Some(Token::Obj(Object::Name(k)))) => {
                                let vat = p.lex.pos;
                                match p.lex.token() {
                                    Ok(Some(t)) => match p.from_token(t, 0, vat) {
                                        Ok(v) if d.len() < 64 => d.set(k, v),
                                        Ok(_) => {}
                                        Err(_) => break,
                                    },
                                    _ => break,
                                }
                            }
                            Ok(Some(_)) => {
                                let _ = at;
                            }
                            _ => break,
                        }
                    }
                    if !ok {
                        break;
                    }
                    let data_start = p.lex.pos + 1;
                    let end = inline_image_end(p.lex.data(), data_start, &d, self.doc, res);
                    let image = p.lex.data().get(data_start..end.0).unwrap_or_default();
                    let image = image.to_vec();
                    p.lex.pos = end.1;
                    self.draw_image_dict(&d, &image, res, &gs, None);
                }
                b"sh" => {
                    if let Some(name) = operands.first().and_then(Object::as_name) {
                        let name = name.to_vec();
                        self.sh(res, &name, &gs);
                    }
                }
                b"BT" => {
                    tm = Transform::identity();
                    tlm = Transform::identity();
                    text_clip.clear();
                }
                b"ET" => {
                    if !text_clip.is_empty() {
                        let paths = std::mem::take(&mut text_clip);
                        self.clip_to_glyphs(&paths, &mut gs);
                    }
                }
                b"Tf" => {
                    gs.font_size = n(1);
                    gs.font = operands
                        .first()
                        .and_then(Object::as_name)
                        .map(<[u8]>::to_vec)
                        .and_then(|name| self.font(res, &name));
                }
                b"Tc" => gs.char_space = n(0),
                b"Tw" => gs.word_space = n(0),
                b"Tz" => gs.hscale = n(0) / 100.0,
                b"TL" => gs.leading = n(0),
                b"Ts" => gs.rise = n(0),
                b"Tr" => gs.render_mode = (n(0) as i32).clamp(0, 7) as u8,
                b"Td" => {
                    tlm = tlm.pre_concat(Transform::from_translate(n(0), n(1)));
                    tm = tlm;
                }
                b"TD" => {
                    gs.leading = -n(1);
                    tlm = tlm.pre_concat(Transform::from_translate(n(0), n(1)));
                    tm = tlm;
                }
                b"Tm" => {
                    tlm = Transform::from_row(n(0), n(1), n(2), n(3), n(4), n(5));
                    tm = tlm;
                }
                b"T*" => {
                    tlm = tlm.pre_concat(Transform::from_translate(0.0, -gs.leading));
                    tm = tlm;
                }
                b"Tj" | b"'" | b"\"" => {
                    if op.as_slice() == b"\"" {
                        gs.word_space = n(0);
                        gs.char_space = n(1);
                    }
                    if op.as_slice() != b"Tj" {
                        tlm = tlm.pre_concat(Transform::from_translate(0.0, -gs.leading));
                        tm = tlm;
                    }
                    if let Some(s) = operands.last().and_then(Object::as_str) {
                        let s = s.to_vec();
                        self.show(&s, &mut tm, &gs, res, &mut text_clip, depth);
                    }
                }
                b"TJ" => {
                    if let Some(Object::Array(items)) = operands.first() {
                        let items = items.clone();
                        for it in &items {
                            match it {
                                Object::Str(s) => {
                                    self.show(s, &mut tm, &gs, res, &mut text_clip, depth)
                                }
                                o => {
                                    let adj = finite(o.as_f64().unwrap_or(0.0));
                                    let tx = -adj / 1000.0 * gs.font_size * gs.hscale;
                                    tm = tm.pre_concat(Transform::from_translate(tx, 0.0));
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            if let Some((fill, stroke, rule, close)) = paint {
                if close {
                    path.close();
                }
                let built = std::mem::replace(&mut path, PathBuilder::new()).finish();
                segments = 0;
                if let Some(pth) = &built {
                    if fill {
                        self.fill(pth, rule, &gs);
                    }
                    if stroke {
                        self.stroke(pth, &gs);
                    }
                }
                if let Some(clip_rule) = pending_clip.take() {
                    self.clip(built.as_ref(), clip_rule, &mut gs);
                }
            }
            operands.clear();
        }
    }

    fn color(&mut self, cs: &ColorSpace, comps: &[f64], alpha: f32) -> Option<Color> {
        if matches!(cs, ColorSpace::Pattern(_)) {
            return None;
        }
        if let Some(c) = self.forced {
            return Some(c);
        }
        let [r, g, b] = cs.to_rgb(comps)?;
        Color::from_rgba(r, g, b, alpha.clamp(0.0, 1.0))
    }

    fn paint(&self, color: Color, blend: BlendMode) -> Paint<'static> {
        let mut p = Paint {
            anti_alias: self.opts.smooth_line_art,
            blend_mode: blend,
            ..Paint::default()
        };
        p.set_color(color);
        p
    }

    fn fill(&mut self, path: &tiny_skia::Path, rule: FillRule, gs: &GState) {
        if matches!(*gs.fill_cs, ColorSpace::Pattern(_)) {
            self.paint_pattern(path, rule, gs.ctm, gs, false);
            return;
        }
        let Some(c) = self.color(&gs.fill_cs.clone(), &gs.fill, gs.fill_alpha) else {
            return;
        };
        let paint = self.paint(c, gs.blend);
        if !drawable(path, gs.ctm, 0.0) {
            self.skip("geometry outside the safe range");
            return;
        }
        self.pixmap
            .fill_path(path, &paint, rule, gs.ctm, gs.mask.as_deref());
    }

    fn stroke(&mut self, path: &tiny_skia::Path, gs: &GState) {
        let pattern = matches!(*gs.stroke_cs, ColorSpace::Pattern(_));
        let color = if pattern {
            Color::BLACK
        } else {
            match self.color(&gs.stroke_cs.clone(), &gs.stroke, gs.stroke_alpha) {
                Some(c) => c,
                None => return,
            }
        };
        let paint = self.paint(color, gs.blend);
        // Device scale of the current transform, to find sub-pixel strokes.
        let t = gs.ctm;
        let scale = ((t.sx * t.sy - t.kx * t.ky).abs()).sqrt();
        let mut width = gs.line_width;
        if width * scale < 1.0 && self.opts.enhance_thin_lines {
            width = 0.0; // a one-pixel hairline
        }
        let dash = if gs.dash.iter().any(|&d| d > 0.0) {
            let mut arr = gs.dash.clone();
            if arr.len() % 2 == 1 {
                arr.extend_from_within(..);
            }
            StrokeDash::new(arr, gs.dash_phase)
        } else {
            None
        };
        let stroke = Stroke {
            width,
            miter_limit: gs.miter,
            line_cap: gs.cap,
            line_join: gs.join,
            dash,
        };
        if pattern {
            // Pattern strokes: fill the outline of the stroke with the pattern.
            let res_scale = tiny_skia::PathStroker::compute_resolution_scale(&gs.ctm);
            let outline = if stroke.width == 0.0 {
                let thin = Stroke {
                    width: 1.0 / scale.max(1e-6),
                    ..stroke.clone()
                };
                path.stroke(&thin, res_scale)
            } else {
                path.stroke(&stroke, res_scale)
            };
            if let Some(o) = outline {
                self.paint_pattern(&o, FillRule::Winding, gs.ctm, gs, true);
            }
            return;
        }
        if !drawable(path, gs.ctm, stroke.width * scale) {
            self.skip("geometry outside the safe range");
            return;
        }
        self.pixmap
            .stroke_path(path, &paint, &stroke, gs.ctm, gs.mask.as_deref());
    }

    fn clip(&mut self, path: Option<&tiny_skia::Path>, rule: FillRule, gs: &mut GState) {
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let aa = self.opts.smooth_line_art;
        // Absurd clip geometry (outside the safe range) clips nothing.
        if path.is_some_and(|p| !drawable(p, gs.ctm, 0.0)) {
            return;
        }
        let mask = match (path, &gs.clip) {
            // Clipping to an empty path hides everything after it.
            (None, _) => Mask::new(w, h),
            (Some(p), None) => Mask::new(w, h).map(|mut m| {
                m.fill_path(p, rule, aa, gs.ctm);
                m
            }),
            (Some(p), Some(old)) => {
                let mut m = (**old).clone();
                m.intersect_path(p, rule, aa, gs.ctm);
                Some(m)
            }
        };
        if let Some(m) = mask {
            gs.clip = Some(Rc::new(m));
            refresh_mask(gs);
        }
    }

    fn ext_gstate(&mut self, res: &Dict, name: &[u8], gs: &mut GState) {
        let doc = self.doc;
        let Some(d) = doc
            .resolve_key(res, b"ExtGState")
            .and_then(Object::as_dict)
            .and_then(|e| e.get(name))
            .map(|o| doc.resolve(o))
            .and_then(Object::as_dict)
        else {
            return;
        };
        let num = |k: &[u8]| doc.resolve_key(d, k).and_then(Object::as_f64);
        if let Some(v) = num(b"LW") {
            gs.line_width = finite(v).abs();
        }
        if let Some(v) = num(b"LC") {
            gs.cap = match v as i32 {
                1 => LineCap::Round,
                2 => LineCap::Square,
                _ => LineCap::Butt,
            };
        }
        if let Some(v) = num(b"LJ") {
            gs.join = match v as i32 {
                1 => LineJoin::Round,
                2 => LineJoin::Bevel,
                _ => LineJoin::Miter,
            };
        }
        if let Some(v) = num(b"ML") {
            gs.miter = finite(v).max(1.0);
        }
        if let Some(Object::Array(dash)) = doc.resolve_key(d, b"D") {
            gs.dash = dash
                .first()
                .and_then(Object::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Object::as_f64)
                        .map(|v| finite(v).abs())
                        .take(64)
                        .collect()
                })
                .unwrap_or_default();
            gs.dash_phase = finite(dash.get(1).and_then(Object::as_f64).unwrap_or(0.0));
        }
        if let Some(v) = num(b"CA") {
            gs.stroke_alpha = finite(v).clamp(0.0, 1.0);
        }
        if let Some(v) = num(b"ca") {
            gs.fill_alpha = finite(v).clamp(0.0, 1.0);
        }
        match doc.resolve_key(d, b"BM") {
            Some(Object::Name(n)) => gs.blend = blend_mode(n),
            Some(Object::Array(a)) => {
                if let Some(n) = a.first().and_then(Object::as_name) {
                    gs.blend = blend_mode(n);
                }
            }
            _ => {}
        }
        match doc.resolve_key(d, b"SMask") {
            Some(Object::Name(n)) if n == b"None" => {
                gs.smask = None;
                refresh_mask(gs);
            }
            Some(Object::Dict(sm)) => {
                let sm = sm.clone();
                match self.soft_mask(&sm, gs, res) {
                    Some(m) => gs.smask = Some(Rc::new(m)),
                    None => self.skip("soft mask"),
                }
                refresh_mask(gs);
            }
            _ => {}
        }
    }

    /// The font named `name` in the resources, loaded once per page.
    fn font(&mut self, res: &Dict, name: &[u8]) -> Option<Rc<PdfFont>> {
        let doc = self.doc;
        let entry = doc
            .resolve_key(res, b"Font")
            .and_then(Object::as_dict)
            .and_then(|f| f.get(name))?;
        let num = entry.as_objref().map(|r| r.num);
        if let Some(n) = num
            && let Some(f) = self.fonts.get(&n)
        {
            return Some(f.clone());
        }
        let fd = doc.resolve(entry).as_dict()?;
        let f = Rc::new(PdfFont::load(doc, fd, self.opts.system_fonts));
        if let Some(n) = num
            && self.fonts.len() < 4096
        {
            self.fonts.insert(n, f.clone());
        }
        Some(f)
    }

    /// Show a string: draw each glyph by the text rendering mode and advance
    /// the text matrix (§9.4.4).
    fn show(
        &mut self,
        bytes: &[u8],
        tm: &mut Transform,
        gs: &GState,
        res: &Dict,
        clip: &mut Vec<tiny_skia::Path>,
        depth: usize,
    ) {
        let Some(font) = gs.font.clone() else {
            self.skip("text without a font");
            return;
        };
        if font.unavailable {
            self.skip("text (font not embedded and no substitute installed)");
        }
        let mode = gs.render_mode;
        let fill = matches!(mode, 0 | 2 | 4 | 6);
        let stroke = matches!(mode, 1 | 2 | 5 | 6);
        let to_clip = mode >= 4;
        let pattern_fill = fill && matches!(*gs.fill_cs, ColorSpace::Pattern(_));
        let fill_paint = if fill && font.type3.is_none() && !pattern_fill {
            self.color(&gs.fill_cs.clone(), &gs.fill, gs.fill_alpha)
                .map(|c| {
                    let mut p = Paint {
                        anti_alias: self.opts.smooth_text,
                        blend_mode: gs.blend,
                        ..Paint::default()
                    };
                    p.set_color(c);
                    p
                })
        } else {
            None
        };
        let (size, hs) = (gs.font_size, gs.hscale);
        for (code, len) in font.codes(bytes) {
            let text_to_user =
                tm.pre_concat(Transform::from_row(size * hs, 0.0, 0.0, size, 0.0, gs.rise));
            if let Some(t3) = &font.type3 {
                if let Some(proc_data) = font.type3_proc(code)
                    && depth < 4
                    && mode != 3
                {
                    let m = t3.matrix;
                    let mut inner = gs.clone();
                    inner.ctm = gs
                        .ctm
                        .pre_concat(text_to_user)
                        .pre_concat(Transform::from_row(
                            m[0] as f32,
                            m[1] as f32,
                            m[2] as f32,
                            m[3] as f32,
                            m[4] as f32,
                            m[5] as f32,
                        ));
                    inner.font = None;
                    let r3 = t3.resources.clone().unwrap_or_else(|| res.clone());
                    let data = proc_data.to_vec();
                    self.run(&data, &r3, inner, depth + 1);
                }
            } else if !font.unavailable
                && let Some(path) = font.glyph(code, len)
            {
                let trm = gs.ctm.pre_concat(text_to_user);
                if pattern_fill {
                    self.paint_pattern(&path, FillRule::Winding, trm, gs, false);
                }
                if !drawable(&path, trm, 0.0) {
                    self.skip("geometry outside the safe range");
                } else if let Some(p) = &fill_paint {
                    self.pixmap
                        .fill_path(&path, p, FillRule::Winding, trm, gs.mask.as_deref());
                }
                if stroke && let Some(user) = (*path).clone().transform(text_to_user) {
                    self.stroke(&user, gs);
                }
                if to_clip
                    && clip.len() < 100_000
                    && let Some(dev) = (*path).clone().transform(trm)
                {
                    clip.push(dev);
                }
            }
            let w = font.width(code, len) as f32;
            let mut tx = w * size + gs.char_space;
            if len == 1 && code == 32 {
                tx += gs.word_space;
            }
            *tm = tm.pre_concat(Transform::from_translate(finite(f64::from(tx * hs)), 0.0));
        }
    }

    /// Text rendering modes 4–7: intersect the clip with the glyphs shown.
    fn clip_to_glyphs(&mut self, paths: &[tiny_skia::Path], gs: &mut GState) {
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let Some(mut m) = Mask::new(w, h) else { return };
        for p in paths
            .iter()
            .filter(|p| drawable(p, Transform::identity(), 0.0))
        {
            m.fill_path(
                p,
                FillRule::Winding,
                self.opts.smooth_text,
                Transform::identity(),
            );
        }
        if let Some(old) = &gs.clip {
            for (a, b) in m.data_mut().iter_mut().zip(old.data()) {
                *a = ((u16::from(*a) * u16::from(*b) + 127) / 255) as u8;
            }
        }
        gs.clip = Some(Rc::new(m));
        refresh_mask(gs);
    }

    /// Decode (or take from the cache) and draw an image in the unit square.
    fn draw_image_dict(
        &mut self,
        dict: &Dict,
        data: &[u8],
        res: &Dict,
        gs: &GState,
        cache_key: Option<u32>,
    ) {
        let doc = self.doc;
        let is_mask = matches!(
            dict.get(b"ImageMask").or_else(|| dict.get(b"IM")),
            Some(Object::Bool(true))
        );
        // Stencil masks take the current fill colour, so they are not cached.
        let cached = cache_key
            .filter(|_| !is_mask)
            .and_then(|k| self.images.get(&k).cloned());
        let img = match cached {
            Some(img) => img,
            None => {
                let fill = match self.color(&gs.fill_cs.clone(), &gs.fill, 1.0) {
                    Some(c) => [c.red(), c.green(), c.blue(), c.alpha()],
                    None => return,
                };
                if self.image_pixels > MAX_PAGE_IMAGE_PIXELS {
                    self.skip("images beyond the per-page limit");
                    return;
                }
                match crate::image::decode(doc, dict, data, Some(res), fill) {
                    Ok(p) => {
                        self.image_pixels += u64::from(p.width()) * u64::from(p.height());
                        let p = Rc::new(p);
                        if let Some(k) = cache_key.filter(|_| !is_mask) {
                            self.images.insert(k, p.clone());
                        }
                        p
                    }
                    Err(_) => {
                        self.skip("undecodable image");
                        return;
                    }
                }
            }
        };
        let (w, h) = (img.width() as f32, img.height() as f32);
        // Image space: the unit square, with the first row at the top.
        let t = gs
            .ctm
            .pre_concat(Transform::from_row(1.0 / w, 0.0, 0.0, -1.0 / h, 0.0, 1.0));
        // Smooth when shrinking; keep pixels crisp when enlarging unless the
        // image asks for interpolation (as Acrobat does).
        let device_w = (gs.ctm.sx.hypot(gs.ctm.ky)).abs();
        let interpolate = matches!(
            dict.get(b"Interpolate").or_else(|| dict.get(b"I")),
            Some(Object::Bool(true))
        );
        let quality = if !self.opts.smooth_images {
            FilterQuality::Nearest
        } else if device_w < w || interpolate {
            FilterQuality::Bilinear
        } else {
            FilterQuality::Nearest
        };
        let shader = Pattern::new(
            Pixmap::as_ref(&img),
            SpreadMode::Pad,
            quality,
            gs.fill_alpha.clamp(0.0, 1.0),
            Transform::identity(),
        );
        let paint = Paint {
            shader,
            blend_mode: gs.blend,
            anti_alias: self.opts.smooth_images,
            ..Paint::default()
        };
        if let Some(rect) = tiny_skia::Rect::from_xywh(0.0, 0.0, w, h)
            && drawable(&PathBuilder::from_rect(rect), t, 0.0)
        {
            self.pixmap.fill_rect(rect, &paint, t, gs.mask.as_deref());
        }
    }

    fn xobject(&mut self, res: &Dict, name: &[u8], gs: &GState, depth: usize) {
        let doc = self.doc;
        let Some(r) = doc
            .resolve_key(res, b"XObject")
            .and_then(Object::as_dict)
            .and_then(|x| x.get(name))
            .and_then(Object::as_objref)
        else {
            return;
        };
        let Some(Object::Stream(s)) = doc.get(r) else {
            return;
        };
        match s.dict.get_name(b"Subtype") {
            Some(b"Image") => {
                let (dict, data) = (s.dict.clone(), s.data.clone());
                self.draw_image_dict(&dict, &data, res, gs, Some(r.num));
                return;
            }
            Some(b"Form") => {}
            _ => return,
        }
        if depth >= MAX_FORM_DEPTH {
            return;
        }
        let (dict, data) = (s.dict.clone(), s.data.clone());
        self.run_form(r.num, &dict, &data, gs, res, depth);
    }

    /// Draw a Form XObject (§8.10): its Matrix, BBox clip and resources. A
    /// transparency group is drawn into its own layer and composited with
    /// the current alpha, blend mode and soft mask (§11.6.6).
    fn run_form(
        &mut self,
        num: u32,
        dict: &Dict,
        raw: &[u8],
        gs: &GState,
        res: &Dict,
        depth: usize,
    ) {
        let doc = self.doc;
        if depth >= MAX_FORM_DEPTH || !self.forms_active.insert(num) {
            return;
        }
        let stream = folio_core::Stream {
            dict: dict.clone(),
            data: raw.to_vec(),
        };
        if let Ok(data) = decode_stream(&stream) {
            let mut inner = gs.clone();
            if let Some(Object::Array(m)) = doc.resolve_key(dict, b"Matrix") {
                let v: Vec<f32> = m
                    .iter()
                    .filter_map(|x| doc.resolve(x).as_f64())
                    .map(finite)
                    .collect();
                if let [a, b, c, d, e, f] = v.as_slice() {
                    inner.ctm = inner
                        .ctm
                        .pre_concat(Transform::from_row(*a, *b, *c, *d, *e, *f));
                }
            }
            if let Some(bb) = doc.resolve_key(dict, b"BBox").and_then(|o| rect_of(doc, o)) {
                let mut pb = PathBuilder::new();
                if let Some(rect) = tiny_skia::Rect::from_ltrb(
                    finite(bb[0]),
                    finite(bb[1]),
                    finite(bb[2]),
                    finite(bb[3]),
                ) {
                    pb.push_rect(rect);
                }
                let p = pb.finish();
                self.clip(p.as_ref(), FillRule::Winding, &mut inner);
            }
            let form_res = doc
                .resolve_key(dict, b"Resources")
                .and_then(Object::as_dict)
                .cloned()
                .unwrap_or_else(|| res.clone());
            let is_group = doc
                .resolve_key(dict, b"Group")
                .and_then(Object::as_dict)
                .is_some_and(|g| g.get_name(b"S") == Some(b"Transparency"));
            let (w, h) = (self.pixmap.width(), self.pixmap.height());
            let small_enough = u64::from(w) * u64::from(h) <= MAX_LAYER_PIXELS;
            let needs_layer =
                gs.fill_alpha < 1.0 || gs.blend != BlendMode::SourceOver || gs.smask.is_some();
            if is_group
                && needs_layer
                && small_enough
                && let Some(layer) = Pixmap::new(w, h)
            {
                // Inside the group, alpha, blend mode and soft mask start fresh.
                inner.fill_alpha = 1.0;
                inner.stroke_alpha = 1.0;
                inner.blend = BlendMode::SourceOver;
                inner.smask = None;
                refresh_mask(&mut inner);
                let saved = std::mem::replace(&mut self.pixmap, layer);
                self.run(&data, &form_res, inner, depth + 1);
                let layer = std::mem::replace(&mut self.pixmap, saved);
                let paint = PixmapPaint {
                    opacity: gs.fill_alpha.clamp(0.0, 1.0),
                    blend_mode: gs.blend,
                    quality: FilterQuality::Nearest,
                };
                self.pixmap.draw_pixmap(
                    0,
                    0,
                    layer.as_ref(),
                    &paint,
                    Transform::identity(),
                    gs.mask.as_deref(),
                );
            } else {
                self.run(&data, &form_res, inner, depth + 1);
            }
        }
        self.forms_active.remove(&num);
    }

    /// A pattern from the resources, kept with the graphics state.
    fn pattern(&mut self, res: &Dict, name: &[u8]) -> Option<Rc<Object>> {
        let doc = self.doc;
        let o = doc
            .resolve_key(res, b"Pattern")
            .and_then(Object::as_dict)
            .and_then(|p| p.get(name))?;
        Some(Rc::new(doc.resolve(o).clone()))
    }

    fn page_rect(&self) -> Option<IntRect> {
        IntRect::from_xywh(0, 0, self.pixmap.width(), self.pixmap.height())
    }

    /// Device-space bounds of a path, clipped to the page.
    fn device_bounds(&self, path: &tiny_skia::Path, t: Transform) -> Option<IntRect> {
        let b = path.clone().transform(t)?.bounds();
        let page = self.page_rect()?;
        let x0 = (b.left().floor() as i64).clamp(0, i64::from(page.width()));
        let y0 = (b.top().floor() as i64).clamp(0, i64::from(page.height()));
        let x1 = (b.right().ceil() as i64).clamp(0, i64::from(page.width()));
        let y1 = (b.bottom().ceil() as i64).clamp(0, i64::from(page.height()));
        IntRect::from_ltrb(x0 as i32, y0 as i32, x1 as i32, y1 as i32)
    }

    /// The `sh` operator: paint a shading over the current clip (§8.7.4.2).
    fn sh(&mut self, res: &Dict, name: &[u8], gs: &GState) {
        let doc = self.doc;
        let Some(o) = doc
            .resolve_key(res, b"Shading")
            .and_then(Object::as_dict)
            .and_then(|s| s.get(name))
        else {
            return;
        };
        let Some(sh) = Shading::parse(doc, o, Some(res)) else {
            self.skip("undecodable shading");
            return;
        };
        let area = match sh.bbox {
            Some(b) => {
                let mut pb = PathBuilder::new();
                if let Some(r) = tiny_skia::Rect::from_ltrb(
                    finite(b[0]),
                    finite(b[1]),
                    finite(b[2]),
                    finite(b[3]),
                ) {
                    pb.push_rect(r);
                }
                pb.finish().and_then(|p| self.device_bounds(&p, gs.ctm))
            }
            None => self.page_rect(),
        };
        let Some(area) = area else { return };
        let Some(layer) = sh.render(gs.ctm, area, false) else {
            return;
        };
        let paint = PixmapPaint {
            opacity: gs.fill_alpha.clamp(0.0, 1.0),
            blend_mode: gs.blend,
            quality: FilterQuality::Nearest,
        };
        self.pixmap.draw_pixmap(
            area.x(),
            area.y(),
            layer.as_ref(),
            &paint,
            Transform::identity(),
            gs.mask.as_deref(),
        );
    }

    /// Fill `path` (with `transform` to device space) with the current
    /// pattern: shading patterns (type 2) and tiling patterns (type 1),
    /// coloured or uncoloured (§8.7.3).
    fn paint_pattern(
        &mut self,
        path: &tiny_skia::Path,
        rule: FillRule,
        transform: Transform,
        gs: &GState,
        stroke_side: bool,
    ) {
        let doc = self.doc;
        let pat = if stroke_side {
            gs.stroke_pattern.clone()
        } else {
            gs.fill_pattern.clone()
        };
        let Some(pat) = pat else {
            self.skip("pattern");
            return;
        };
        if !drawable(path, transform, 0.0) {
            self.skip("geometry outside the safe range");
            return;
        }
        let Some(d) = pat.as_dict() else { return };
        let m = match doc.resolve_key(d, b"Matrix") {
            Some(Object::Array(a)) => {
                let v: Vec<f32> = a
                    .iter()
                    .filter_map(|x| doc.resolve(x).as_f64())
                    .map(finite)
                    .collect();
                match v.as_slice() {
                    [a, b, c, e, f, g] => Transform::from_row(*a, *b, *c, *e, *f, *g),
                    _ => Transform::identity(),
                }
            }
            _ => Transform::identity(),
        };
        let to_dev = self.base.pre_concat(m);
        let alpha = if stroke_side {
            gs.stroke_alpha
        } else {
            gs.fill_alpha
        }
        .clamp(0.0, 1.0);
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        match d.get_int(b"PatternType") {
            Some(2) => {
                let Some(sh) = d.get(b"Shading").and_then(|o| Shading::parse(doc, o, None)) else {
                    self.skip("undecodable shading");
                    return;
                };
                let Some(area) = self.device_bounds(path, transform) else {
                    return;
                };
                let Some(mut shape) = Mask::new(w, h) else {
                    return;
                };
                shape.fill_path(path, rule, self.opts.smooth_line_art, transform);
                if let Some(mm) = &gs.mask {
                    for (a, b) in shape.data_mut().iter_mut().zip(mm.data()) {
                        *a = ((u16::from(*a) * u16::from(*b) + 127) / 255) as u8;
                    }
                }
                let Some(layer) = sh.render(to_dev, area, true) else {
                    return;
                };
                let paint = PixmapPaint {
                    opacity: alpha,
                    blend_mode: gs.blend,
                    quality: FilterQuality::Nearest,
                };
                self.pixmap.draw_pixmap(
                    area.x(),
                    area.y(),
                    layer.as_ref(),
                    &paint,
                    Transform::identity(),
                    Some(&shape),
                );
            }
            Some(1) => {
                if self.pattern_depth >= 3 {
                    return;
                }
                let Some(st) = pat.as_stream() else { return };
                let Some(bbox) = doc.resolve_key(d, b"BBox").and_then(|o| rect_of(doc, o)) else {
                    return;
                };
                let num = |k: &[u8]| {
                    doc.resolve_key(d, k)
                        .and_then(Object::as_f64)
                        .unwrap_or(0.0)
                };
                let (xs, ys) = (num(b"XStep").abs(), num(b"YStep").abs());
                if !(xs > 0.0 && ys > 0.0) || !xs.is_finite() || !ys.is_finite() {
                    return;
                }
                let det = f64::from((to_dev.sx * to_dev.sy - to_dev.kx * to_dev.ky).abs());
                let mut s = det.sqrt();
                if s.is_nan() || s <= 0.0 || !s.is_finite() {
                    return;
                }
                // Keep one tile under 2048 × 2048 pixels.
                let device_s = s;
                s = s.min(2048.0 / xs).min(2048.0 / ys);
                // At device resolution a tile maps pixel to pixel: sample it exactly.
                let quality = if (s - device_s).abs() < 1e-9 {
                    FilterQuality::Nearest
                } else {
                    FilterQuality::Bilinear
                };
                let tw = (xs * s).ceil().clamp(1.0, 2048.0) as u32;
                let th = (ys * s).ceil().clamp(1.0, 2048.0) as u32;
                let tile_ctm = Transform::from_row(
                    s as f32,
                    0.0,
                    0.0,
                    -(s as f32),
                    (-bbox[0] * s) as f32,
                    ((bbox[1] + ys) * s) as f32,
                );
                let Some(tile) = Pixmap::new(tw, th) else {
                    return;
                };
                let Ok(data) = decode_stream(st) else { return };
                let pres = doc
                    .resolve_key(d, b"Resources")
                    .and_then(Object::as_dict)
                    .cloned()
                    .unwrap_or_default();
                let forced = if d.get_int(b"PaintType") == Some(2) {
                    let (cs, comps) = if stroke_side {
                        (gs.stroke_cs.clone(), &gs.stroke)
                    } else {
                        (gs.fill_cs.clone(), &gs.fill)
                    };
                    match &*cs {
                        ColorSpace::Pattern(Some(base)) => base
                            .to_rgb(comps)
                            .and_then(|[r, g, b]| Color::from_rgba(r, g, b, 1.0)),
                        _ => Some(Color::BLACK),
                    }
                } else {
                    None
                };
                let saved = std::mem::replace(&mut self.pixmap, tile);
                let saved_forced = std::mem::replace(&mut self.forced, forced);
                self.pattern_depth += 1;
                self.run(&data, &pres, GState::new(tile_ctm), 1);
                self.pattern_depth -= 1;
                self.forced = saved_forced;
                let tile = std::mem::replace(&mut self.pixmap, saved);
                let (Some(inv), Some(tile_inv)) = (transform.invert(), tile_ctm.invert()) else {
                    return;
                };
                let shader_t = inv.pre_concat(to_dev).pre_concat(tile_inv);
                let shader =
                    Pattern::new(tile.as_ref(), SpreadMode::Repeat, quality, alpha, shader_t);
                let paint = Paint {
                    shader,
                    blend_mode: gs.blend,
                    anti_alias: self.opts.smooth_line_art,
                    ..Paint::default()
                };
                self.pixmap
                    .fill_path(path, &paint, rule, transform, gs.mask.as_deref());
            }
            _ => self.skip("pattern"),
        }
    }

    /// A soft mask from an ExtGState SMask dictionary (§11.6.5.2): the
    /// group's luminosity over its backdrop, or its alpha.
    fn soft_mask(&mut self, sm: &Dict, gs: &GState, res: &Dict) -> Option<Mask> {
        let doc = self.doc;
        let g = sm.get(b"G")?.as_objref()?;
        let Some(Object::Stream(st)) = doc.get(g) else {
            return None;
        };
        let (dict, data) = (st.dict.clone(), st.data.clone());
        let luminosity = sm.get_name(b"S") != Some(b"Alpha");
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        if u64::from(w) * u64::from(h) > MAX_LAYER_PIXELS {
            return None;
        }
        let mut layer = Pixmap::new(w, h)?;
        if luminosity {
            let bc: Vec<f64> = doc
                .resolve_key(sm, b"BC")
                .and_then(Object::as_array)
                .map(|a| a.iter().filter_map(|x| doc.resolve(x).as_f64()).collect())
                .unwrap_or_default();
            let cs = doc
                .resolve_key(&dict, b"Group")
                .and_then(Object::as_dict)
                .and_then(|grp| grp.get(b"CS"))
                .and_then(|c| ColorSpace::parse(doc, c, Some(res)))
                .unwrap_or(ColorSpace::Gray);
            let backdrop = if bc.is_empty() {
                [0.0, 0.0, 0.0]
            } else {
                cs.to_rgb(&bc).unwrap_or([0.0, 0.0, 0.0])
            };
            if let Some(c) = Color::from_rgba(backdrop[0], backdrop[1], backdrop[2], 1.0) {
                layer.fill(c);
            }
        }
        let saved = std::mem::replace(&mut self.pixmap, layer);
        let mut inner = GState::new(gs.ctm);
        inner.fill_alpha = 1.0;
        self.run_form(g.num, &dict, &data, &inner, res, 1);
        let layer = std::mem::replace(&mut self.pixmap, saved);
        let tr = doc
            .resolve_key(sm, b"TR")
            .filter(|o| o.as_name() != Some(b"Identity"))
            .and_then(|f| crate::function::Function::parse(doc, f));
        let mut lut = [0u8; 256];
        for (i, slot) in lut.iter_mut().enumerate() {
            let v = i as f64 / 255.0;
            let v = tr
                .as_ref()
                .and_then(|f| f.eval(&[v]).first().copied())
                .unwrap_or(v);
            *slot = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
        let values: Vec<u8> = layer
            .pixels()
            .iter()
            .map(|p| {
                let raw = if luminosity {
                    let c = p.demultiply();
                    let l = 0.30 * f32::from(c.red())
                        + 0.59 * f32::from(c.green())
                        + 0.11 * f32::from(c.blue());
                    l.round().clamp(0.0, 255.0) as u8
                } else {
                    p.alpha()
                };
                lut[usize::from(raw)]
            })
            .collect();
        Mask::from_vec(values, tiny_skia::IntSize::from_wh(w, h)?)
    }

    /// Draw the normal appearance of every visible annotation (§12.5.5).
    fn annotations(&mut self, page: ObjRef) {
        let doc = self.doc;
        let Some(pd) = doc.get(page).and_then(Object::as_dict) else {
            return;
        };
        let Some(Object::Array(annots)) = doc.resolve_key(pd, b"Annots") else {
            return;
        };
        let annots = annots.clone();
        for a in annots.iter().take(10_000) {
            let Some(ad) = doc.resolve(a).as_dict() else {
                continue;
            };
            let flags = ad.get_int(b"F").unwrap_or(0);
            // Hidden (bit 2) and NoView (bit 6).
            if flags & (2 | 32) != 0 || ad.get_name(b"Subtype") == Some(b"Popup") {
                continue;
            }
            let Some(ap) = doc.resolve_key(ad, b"AP").and_then(Object::as_dict) else {
                continue;
            };
            let Some(n) = ap.get(b"N") else { continue };
            let r = match doc.resolve(n) {
                Object::Stream(_) => n.as_objref(),
                Object::Dict(states) => ad
                    .get_name(b"AS")
                    .and_then(|s| states.get(s))
                    .and_then(Object::as_objref),
                _ => None,
            };
            let Some(r) = r else { continue };
            let Some(Object::Stream(st)) = doc.get(r) else {
                continue;
            };
            let Some(rect) = doc.resolve_key(ad, b"Rect").and_then(|o| rect_of(doc, o)) else {
                continue;
            };
            let Some(bb) = doc
                .resolve_key(&st.dict, b"BBox")
                .and_then(|o| rect_of(doc, o))
            else {
                continue;
            };
            // Algorithm 8.1: map the transformed BBox onto Rect.
            let m = match doc.resolve_key(&st.dict, b"Matrix") {
                Some(Object::Array(a)) => {
                    let v: Vec<f32> = a
                        .iter()
                        .filter_map(|x| doc.resolve(x).as_f64())
                        .map(finite)
                        .collect();
                    match v.as_slice() {
                        [a, b, c, e, f, g] => Transform::from_row(*a, *b, *c, *e, *f, *g),
                        _ => Transform::identity(),
                    }
                }
                _ => Transform::identity(),
            };
            let corners = [
                (bb[0], bb[1]),
                (bb[2], bb[1]),
                (bb[0], bb[3]),
                (bb[2], bb[3]),
            ];
            let pts: Vec<(f32, f32)> = corners
                .iter()
                .map(|&(x, y)| {
                    let mut p = tiny_skia::Point::from_xy(finite(x), finite(y));
                    m.map_point(&mut p);
                    (p.x, p.y)
                })
                .collect();
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for (x, y) in &pts {
                x0 = x0.min(*x);
                y0 = y0.min(*y);
                x1 = x1.max(*x);
                y1 = y1.max(*y);
            }
            if !(x1 > x0 && y1 > y0) {
                continue;
            }
            let sx = (rect[2] - rect[0]) as f32 / (x1 - x0);
            let sy = (rect[3] - rect[1]) as f32 / (y1 - y0);
            let a_m = Transform::from_row(
                sx,
                0.0,
                0.0,
                sy,
                rect[0] as f32 - x0 * sx,
                rect[1] as f32 - y0 * sy,
            );
            let gs = GState::new(self.base.pre_concat(a_m));
            let (dict, data) = (st.dict.clone(), st.data.clone());
            let res = Dict::new();
            self.run_form(r.num, &dict, &data, &gs, &res, 1);
        }
    }
}

/// Where an inline image's data ends, and where parsing resumes after `EI`.
fn inline_image_end(
    data: &[u8],
    start: usize,
    d: &Dict,
    doc: &Document,
    res: &Dict,
) -> (usize, usize) {
    use folio_core::lexer::{is_regular, is_ws};
    let get = |a: &[u8], b: &[u8]| d.get(a).or_else(|| d.get(b));
    if get(b"Filter", b"F").is_none() {
        let int =
            |a: &[u8], b: &[u8]| get(a, b).and_then(Object::as_int).unwrap_or(0).max(0) as usize;
        let mask = matches!(get(b"ImageMask", b"IM"), Some(Object::Bool(true)));
        let bpc = if mask {
            1
        } else {
            int(b"BitsPerComponent", b"BPC").max(1)
        };
        let comps = if mask {
            1
        } else {
            get(b"ColorSpace", b"CS")
                .and_then(|o| ColorSpace::parse(doc, o, Some(res)))
                .map(|c| c.components())
                .unwrap_or(1)
        };
        let row = int(b"Width", b"W")
            .saturating_mul(comps)
            .saturating_mul(bpc)
            .div_ceil(8);
        let len = row.saturating_mul(int(b"Height", b"H"));
        let end = start.saturating_add(len);
        let mut q = end;
        while data.get(q).copied().is_some_and(is_ws) {
            q += 1;
        }
        if end <= data.len() && data.get(q..q + 2) == Some(b"EI") {
            return (end, q + 2);
        }
    }
    let mut i = start;
    while i + 1 < data.len() {
        if data.get(i) == Some(&b'E')
            && data.get(i + 1) == Some(&b'I')
            && i > start
            && data.get(i - 1).copied().is_some_and(is_ws)
            && data.get(i + 2).is_none_or(|&b| !is_regular(b))
        {
            return (i - 1, i + 2);
        }
        i += 1;
    }
    (data.len(), data.len())
}

/// Combine the clip and the soft mask into the mask painting uses.
fn refresh_mask(gs: &mut GState) {
    gs.mask = match (&gs.clip, &gs.smask) {
        (None, None) => None,
        (Some(c), None) => Some(c.clone()),
        (None, Some(m)) => Some(m.clone()),
        (Some(c), Some(m)) => {
            let mut out = (**c).clone();
            for (a, b) in out.data_mut().iter_mut().zip(m.data()) {
                *a = ((u16::from(*a) * u16::from(*b) + 127) / 255) as u8;
            }
            Some(Rc::new(out))
        }
    };
}

/// Largest device coordinate handed to the rasteriser. tiny-skia works in
/// fixed point inside; geometry far outside any page can upset it, and
/// nothing that large is visible anyway.
const MAX_DEVICE_COORD: f32 = 1.0e6;

/// Whether `path` under `t` (plus a stroke margin) is safe to rasterise.
fn drawable(path: &tiny_skia::Path, t: Transform, margin: f32) -> bool {
    let finite = [t.sx, t.kx, t.ky, t.sy, t.tx, t.ty, margin]
        .iter()
        .all(|v| v.is_finite());
    if !finite || margin.abs() > MAX_DEVICE_COORD {
        return false;
    }
    let Some(p) = path.clone().transform(t) else {
        return false;
    };
    let b = p.bounds();
    let lim = MAX_DEVICE_COORD - margin.abs();
    [b.left(), b.top(), b.right(), b.bottom()]
        .iter()
        .all(|v| v.is_finite() && v.abs() <= lim)
}
