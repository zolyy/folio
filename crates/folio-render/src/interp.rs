//! The content-stream interpreter (ISO 32000-2 §8): graphics state, paths,
//! painting, clipping, colour operators and Form XObjects.

use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;

use folio_core::filters::decode_stream;
use folio_core::lexer::Token;
use folio_core::parser::Parser;
use folio_core::{Dict, Document, Error, ObjRef, Object, Result};
use tiny_skia::{
    BlendMode, Color, FillRule, LineCap, LineJoin, Mask, Paint, PathBuilder, Pixmap, Stroke,
    StrokeDash, Transform,
};

use crate::color::ColorSpace;
use crate::{MAX_PIXELS, Options, Rendered};

/// Operators per page before giving up (hostile content streams).
const MAX_OPS: usize = 2_000_000;
/// Nesting of q/Q and of Form XObjects.
const MAX_STACK: usize = 256;
const MAX_FORM_DEPTH: usize = 16;
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

impl PageGeometry {
    pub fn new(doc: &Document, page: ObjRef, dpi: f64) -> Result<Self> {
        if !dpi.is_finite() || !(1.0..=9600.0).contains(&dpi) {
            return Err(Error::Invalid(format!(
                "resolution must be between 1 and 9600 dpi, not {dpi}"
            )));
        }
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
        let s = dpi / 72.0;
        let rot = doc
            .page_attr(page, b"Rotate")
            .and_then(|o| o.as_int())
            .unwrap_or(0)
            .rem_euclid(360);
        let rot = rot - rot % 90;
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
}

impl GState {
    fn new(ctm: Transform) -> Self {
        GState {
            ctm,
            clip: None,
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
}

pub(crate) fn render(doc: &Document, page: ObjRef, opts: &Options) -> Result<Rendered> {
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
    };
    let res = doc
        .page_attr(page, b"Resources")
        .and_then(|o| o.as_dict().cloned())
        .unwrap_or_default();
    let content = page_content(doc, page);
    ctx.run(&content, &res, GState::new(geo.base), 0);
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
                b"SC" | b"SCN" => gs.stroke = all,
                b"sc" | b"scn" => gs.fill = all,
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
                    loop {
                        match p.lex.token() {
                            Ok(Some(Token::Keyword(k))) if k == b"ID" => break,
                            Ok(Some(_)) => continue,
                            _ => break,
                        }
                    }
                    p.lex.skip_inline_image_data();
                    self.skip("image");
                }
                b"sh" => self.skip("shading"),
                b"Tj" | b"TJ" | b"'" | b"\"" => self.skip("text"),
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
        if matches!(cs, ColorSpace::Pattern) {
            self.skip("pattern");
            return None;
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
        let Some(c) = self.color(&gs.fill_cs.clone(), &gs.fill, gs.fill_alpha) else {
            return;
        };
        let paint = self.paint(c, gs.blend);
        self.pixmap
            .fill_path(path, &paint, rule, gs.ctm, gs.clip.as_deref());
    }

    fn stroke(&mut self, path: &tiny_skia::Path, gs: &GState) {
        let Some(c) = self.color(&gs.stroke_cs.clone(), &gs.stroke, gs.stroke_alpha) else {
            return;
        };
        let paint = self.paint(c, gs.blend);
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
        self.pixmap
            .stroke_path(path, &paint, &stroke, gs.ctm, gs.clip.as_deref());
    }

    fn clip(&mut self, path: Option<&tiny_skia::Path>, rule: FillRule, gs: &mut GState) {
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let aa = self.opts.smooth_line_art;
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
        if let Some(sm) = doc.resolve_key(d, b"SMask")
            && sm.as_name() != Some(b"None")
        {
            self.skip("soft mask");
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
                self.skip("image");
                return;
            }
            Some(b"Form") => {}
            _ => return,
        }
        if depth >= MAX_FORM_DEPTH || !self.forms_active.insert(r.num) {
            return;
        }
        if let Ok(data) = decode_stream(s) {
            let mut inner = gs.clone();
            if let Some(Object::Array(m)) = doc.resolve_key(&s.dict, b"Matrix") {
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
            if let Some(bb) = doc
                .resolve_key(&s.dict, b"BBox")
                .and_then(|o| rect_of(doc, o))
            {
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
                .resolve_key(&s.dict, b"Resources")
                .and_then(Object::as_dict)
                .cloned()
                .unwrap_or_else(|| res.clone());
            self.run(&data, &form_res, inner, depth + 1);
        }
        self.forms_active.remove(&r.num);
    }
}
