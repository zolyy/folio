//! Shadings (ISO 32000-2 §8.7.4.5): function-based (1), axial (2),
//! radial (3), free-form and lattice triangle meshes (4, 5), Coons and
//! tensor-product patch meshes (6, 7). Colours are computed per device
//! pixel, so Extend, Domain and Background follow the specification
//! exactly; meshes are Gouraud-shaded triangles.

use folio_core::filters::decode_stream;
use folio_core::{Dict, Document, Object};
use tiny_skia::{IntRect, Pixmap, Transform};

use crate::color::ColorSpace;
use crate::function::Function;

/// Triangles per mesh, patches subdivide into at most this many in total.
const MAX_TRIANGLES: usize = 200_000;
/// Colour lookup steps for axial and radial shadings.
const LUT: usize = 1024;

#[derive(Clone, Debug)]
struct Vertex {
    x: f64,
    y: f64,
    /// Colour components, or a single parametric value with a function.
    c: Vec<f64>,
}

#[derive(Clone, Debug)]
enum Kind {
    Function { domain: [f64; 4], matrix: Transform },
    Axial { coords: [f64; 4] },
    Radial { coords: [f64; 6] },
    Mesh(Vec<[Vertex; 3]>),
}

#[derive(Clone, Debug)]
pub struct Shading {
    kind: Kind,
    cs: ColorSpace,
    function: Option<Function>,
    t: [f64; 2],
    extend: [bool; 2],
    pub bbox: Option<[f64; 4]>,
    pub background: Option<Vec<f64>>,
}

fn nums(doc: &Document, d: &Dict, key: &[u8]) -> Vec<f64> {
    match doc.resolve_key(d, key) {
        Some(Object::Array(a)) => a
            .iter()
            .filter_map(|x| doc.resolve(x).as_f64())
            .take(256)
            .collect(),
        _ => Vec::new(),
    }
}

impl Shading {
    pub fn parse(doc: &Document, o: &Object, res: Option<&Dict>) -> Option<Shading> {
        let o = doc.resolve(o);
        let d = o.as_dict()?;
        let cs = ColorSpace::parse(doc, d.get(b"ColorSpace")?, res)?;
        let function = doc
            .resolve_key(d, b"Function")
            .and_then(|f| Function::parse(doc, f));
        let ty = d.get_int(b"ShadingType")?;
        let dom = nums(doc, d, b"Domain");
        let t = match dom.as_slice() {
            [a, b, ..] => [*a, *b],
            _ => [0.0, 1.0],
        };
        let ext = match doc.resolve_key(d, b"Extend") {
            Some(Object::Array(a)) => [
                matches!(a.first(), Some(Object::Bool(true))),
                matches!(a.get(1), Some(Object::Bool(true))),
            ],
            _ => [false, false],
        };
        let bbox = match nums(doc, d, b"BBox").as_slice() {
            [a, b, c, e] => Some([a.min(*c), b.min(*e), a.max(*c), b.max(*e)]),
            _ => None,
        };
        let background = Some(nums(doc, d, b"Background")).filter(|v| !v.is_empty());
        let kind = match ty {
            1 => {
                let m = nums(doc, d, b"Matrix");
                let matrix = match m.as_slice() {
                    [a, b, c, e, f, g] => Transform::from_row(
                        *a as f32, *b as f32, *c as f32, *e as f32, *f as f32, *g as f32,
                    ),
                    _ => Transform::identity(),
                };
                let domain = match dom.as_slice() {
                    [a, b, c, e] => [*a, *b, *c, *e],
                    _ => [0.0, 1.0, 0.0, 1.0],
                };
                Kind::Function { domain, matrix }
            }
            2 => match nums(doc, d, b"Coords").as_slice() {
                [a, b, c, e] => Kind::Axial {
                    coords: [*a, *b, *c, *e],
                },
                _ => return None,
            },
            3 => match nums(doc, d, b"Coords").as_slice() {
                [a, b, c, e, f, g] => Kind::Radial {
                    coords: [*a, *b, c.abs(), *e, *f, g.abs()],
                },
                _ => return None,
            },
            4..=7 => {
                let s = o.as_stream()?;
                let data = decode_stream(s).ok()?;
                let n_out = if function.is_some() {
                    1
                } else {
                    cs.components()
                };
                Kind::Mesh(mesh(doc, d, &data, ty, n_out)?)
            }
            _ => return None,
        };
        if matches!(
            kind,
            Kind::Function { .. } | Kind::Axial { .. } | Kind::Radial { .. }
        ) && function.is_none()
        {
            return None;
        }
        Some(Shading {
            kind,
            cs,
            function,
            t,
            extend: ext,
            bbox,
            background,
        })
    }

    fn rgba(&self, comps: &[f64]) -> Option<[u8; 4]> {
        let c = match &self.function {
            Some(f) => f.eval(comps),
            None => comps.to_vec(),
        };
        let [r, g, b] = self.cs.to_rgb(&c)?;
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        Some([q(r), q(g), q(b), 255])
    }

    fn background_rgba(&self) -> Option<[u8; 4]> {
        let bg = self.background.as_ref()?;
        let [r, g, b] = self.cs.to_rgb(bg)?;
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        Some([q(r), q(g), q(b), 255])
    }

    /// Draw the shading into a transparent pixmap covering `area` (device
    /// pixels). `to_device` maps shading space to device space. With
    /// `use_background`, areas the shading leaves empty get its Background.
    pub fn render(
        &self,
        to_device: Transform,
        area: IntRect,
        use_background: bool,
    ) -> Option<Pixmap> {
        let mut pm = Pixmap::new(area.width(), area.height())?;
        let inv = to_device.invert()?;
        let bg = if use_background {
            self.background_rgba()
        } else {
            None
        };
        let (ox, oy) = (area.x() as f32, area.y() as f32);
        let w = area.width() as usize;
        match &self.kind {
            Kind::Axial { coords } => {
                let lut = self.lut();
                let [x0, y0, x1, y1] = *coords;
                let (dx, dy) = (x1 - x0, y1 - y0);
                let len2 = dx * dx + dy * dy;
                self.fill_each(&mut pm, w, inv, ox, oy, |x, y| {
                    let s = if len2 == 0.0 {
                        0.0
                    } else {
                        ((x - x0) * dx + (y - y0) * dy) / len2
                    };
                    self.lookup(&lut, s).or(bg)
                });
            }
            Kind::Radial { coords } => {
                let lut = self.lut();
                let [x0, y0, r0, x1, y1, r1] = *coords;
                let (cdx, cdy, dr) = (x1 - x0, y1 - y0, r1 - r0);
                let a = cdx * cdx + cdy * cdy - dr * dr;
                self.fill_each(&mut pm, w, inv, ox, oy, |x, y| {
                    let (px, py) = (x - x0, y - y0);
                    let b = px * cdx + py * cdy + r0 * dr;
                    let c = px * px + py * py - r0 * r0;
                    let mut roots = Vec::with_capacity(2);
                    if a.abs() < 1e-12 {
                        if b.abs() > 1e-12 {
                            roots.push(c / (2.0 * b));
                        }
                    } else {
                        let disc = b * b - a * c;
                        if disc >= 0.0 {
                            let sq = disc.sqrt();
                            roots.push((b + sq) / a);
                            roots.push((b - sq) / a);
                        }
                    }
                    // The largest s whose circle has a non-negative radius
                    // and lies in the extended domain wins.
                    roots.sort_by(|p, q| q.total_cmp(p));
                    for s in roots {
                        if !s.is_finite() || r0 + s * dr < 0.0 {
                            continue;
                        }
                        if (s < 0.0 && !self.extend[0]) || (s > 1.0 && !self.extend[1]) {
                            continue;
                        }
                        return self.lookup(&lut, s);
                    }
                    bg
                });
            }
            Kind::Function { domain, matrix } => {
                let inv_m = matrix.invert()?;
                let f = self.function.as_ref()?;
                self.fill_each(&mut pm, w, inv, ox, oy, |x, y| {
                    let mut p = tiny_skia::Point::from_xy(x as f32, y as f32);
                    inv_m.map_point(&mut p);
                    let (u, v) = (f64::from(p.x), f64::from(p.y));
                    if u < domain[0] || u > domain[1] || v < domain[2] || v > domain[3] {
                        return bg;
                    }
                    let c = f.eval(&[u, v]);
                    let [r, g, b] = self.cs.to_rgb(&c)?;
                    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    Some([q(r), q(g), q(b), 255])
                });
            }
            Kind::Mesh(tris) => {
                if let Some(c) = bg {
                    pm.fill(tiny_skia::Color::from_rgba8(c[0], c[1], c[2], 255));
                }
                for t in tris {
                    self.triangle(&mut pm, t, to_device, ox, oy);
                }
            }
        }
        Some(pm)
    }

    /// Colours for s in [0, 1] of the axial/radial parameter.
    fn lut(&self) -> Vec<Option<[u8; 4]>> {
        (0..LUT)
            .map(|i| {
                let s = i as f64 / (LUT - 1) as f64;
                let t = self.t[0] + s * (self.t[1] - self.t[0]);
                self.rgba(&[t])
            })
            .collect()
    }

    fn lookup(&self, lut: &[Option<[u8; 4]>], s: f64) -> Option<[u8; 4]> {
        if !s.is_finite() {
            return None;
        }
        let s = if s < 0.0 {
            if !self.extend[0] {
                return None;
            }
            0.0
        } else if s > 1.0 {
            if !self.extend[1] {
                return None;
            }
            1.0
        } else {
            s
        };
        let i = (s * (LUT - 1) as f64).round() as usize;
        lut.get(i).copied().flatten()
    }

    fn fill_each(
        &self,
        pm: &mut Pixmap,
        w: usize,
        inv: Transform,
        ox: f32,
        oy: f32,
        f: impl Fn(f64, f64) -> Option<[u8; 4]>,
    ) {
        for (i, px) in pm.data_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = (
                (i % w.max(1)) as f32 + ox + 0.5,
                (i / w.max(1)) as f32 + oy + 0.5,
            );
            let mut p = tiny_skia::Point::from_xy(x, y);
            inv.map_point(&mut p);
            if let Some(c) = f(f64::from(p.x), f64::from(p.y)) {
                px.copy_from_slice(&c);
            }
        }
    }

    /// Gouraud-shade one triangle (vertices in shading space).
    fn triangle(&self, pm: &mut Pixmap, t: &[Vertex; 3], to_device: Transform, ox: f32, oy: f32) {
        let dev: Vec<(f64, f64)> = t
            .iter()
            .map(|v| {
                let mut p = tiny_skia::Point::from_xy(v.x as f32, v.y as f32);
                to_device.map_point(&mut p);
                (f64::from(p.x - ox), f64::from(p.y - oy))
            })
            .collect();
        let (Some(&(ax, ay)), Some(&(bx, by)), Some(&(cx, cy))) =
            (dev.first(), dev.get(1), dev.get(2))
        else {
            return;
        };
        let det = (bx - ax) * (cy - ay) - (cx - ax) * (by - ay);
        if det.abs() < 1e-9 || !det.is_finite() {
            return;
        }
        let (w, h) = (pm.width() as f64, pm.height() as f64);
        let x_lo = ax.min(bx).min(cx).floor().max(0.0) as usize;
        let x_hi = ax.max(bx).max(cx).ceil().min(w) as usize;
        let y_lo = ay.min(by).min(cy).floor().max(0.0) as usize;
        let y_hi = ay.max(by).max(cy).ceil().min(h) as usize;
        let stride = pm.width() as usize;
        let n = t[0].c.len();
        let data = pm.data_mut();
        let mut comps = vec![0.0; n];
        for y in y_lo..y_hi {
            for x in x_lo..x_hi {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let l1 = ((bx - px) * (cy - py) - (cx - px) * (by - py)) / det;
                let l2 = ((cx - px) * (ay - py) - (ax - px) * (cy - py)) / det;
                let l3 = 1.0 - l1 - l2;
                if l1 < -1e-6 || l2 < -1e-6 || l3 < -1e-6 {
                    continue;
                }
                for (k, slot) in comps.iter_mut().enumerate() {
                    let g = |v: &Vertex| v.c.get(k).copied().unwrap_or(0.0);
                    *slot = l1 * g(&t[0]) + l2 * g(&t[1]) + l3 * g(&t[2]);
                }
                if let Some(c) = self.rgba(&comps)
                    && let Some(px) = data.get_mut((y * stride + x) * 4..(y * stride + x) * 4 + 4)
                {
                    px.copy_from_slice(&c);
                }
            }
        }
    }
}

/// A patch: its control points and corner colours.
type Patch = (Vec<(f64, f64)>, Vec<Vec<f64>>);

/// Reads big-endian bit fields from mesh data.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn read(&mut self, n: u32) -> Option<u64> {
        if n == 0 || n > 32 {
            return None;
        }
        let mut v: u64 = 0;
        for _ in 0..n {
            let byte = *self.data.get(self.pos / 8)?;
            let bit = (byte >> (7 - (self.pos % 8))) & 1;
            v = (v << 1) | u64::from(bit);
            self.pos += 1;
        }
        Some(v)
    }
    fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }
}

/// Decode a mesh into triangles in shading space.
fn mesh(doc: &Document, d: &Dict, data: &[u8], ty: i64, n_out: usize) -> Option<Vec<[Vertex; 3]>> {
    let bpc = u32::try_from(d.get_int(b"BitsPerCoordinate")?).ok()?;
    let bpcomp = u32::try_from(d.get_int(b"BitsPerComponent")?).ok()?;
    let bpf = u32::try_from(d.get_int(b"BitsPerFlag").unwrap_or(8)).ok()?;
    let dec = nums(doc, d, b"Decode");
    if dec.len() < 4 + 2 * n_out {
        return None;
    }
    let mut r = Bits { data, pos: 0 };
    let scale = |v: u64, bits: u32, lo: f64, hi: f64| {
        let max = ((1u64 << bits) - 1) as f64;
        lo + v as f64 * (hi - lo) / max.max(1.0)
    };
    let read_vertex = |r: &mut Bits<'_>| -> Option<Vertex> {
        let x = scale(r.read(bpc)?, bpc, dec[0], dec[1]);
        let y = scale(r.read(bpc)?, bpc, dec[2], dec[3]);
        let mut c = Vec::with_capacity(n_out);
        for k in 0..n_out {
            let lo = *dec.get(4 + 2 * k)?;
            let hi = *dec.get(5 + 2 * k)?;
            c.push(scale(r.read(bpcomp)?, bpcomp, lo, hi));
        }
        Some(Vertex { x, y, c })
    };
    let mut tris: Vec<[Vertex; 3]> = Vec::new();
    match ty {
        4 => {
            // Flag 0 starts a triangle of three new vertices; flag 1 reuses
            // the last two vertices of the previous triangle, flag 2 its
            // first and last (§8.7.4.5.5).
            let mut last: Option<[Vertex; 3]> = None;
            let mut fresh: Vec<Vertex> = Vec::new();
            while tris.len() < MAX_TRIANGLES {
                let Some(flag) = r.read(bpf) else { break };
                let Some(v) = read_vertex(&mut r) else { break };
                r.align();
                if !fresh.is_empty() || (flag == 0 || last.is_none()) {
                    fresh.push(v);
                    if fresh.len() == 3 {
                        let t = [fresh[0].clone(), fresh[1].clone(), fresh[2].clone()];
                        tris.push(t.clone());
                        last = Some(t);
                        fresh.clear();
                    }
                    continue;
                }
                let Some([pa, pb, pc]) = last.take() else {
                    break;
                };
                let t = if flag == 1 { [pb, pc, v] } else { [pa, pc, v] };
                tris.push(t.clone());
                last = Some(t);
            }
        }
        5 => {
            let per_row = usize::try_from(d.get_int(b"VerticesPerRow")?).ok()?;
            if per_row < 2 {
                return None;
            }
            let mut rows: Vec<Vec<Vertex>> = Vec::new();
            'outer: while rows.len() * per_row < MAX_TRIANGLES {
                let mut row = Vec::with_capacity(per_row);
                for _ in 0..per_row {
                    match read_vertex(&mut r) {
                        Some(v) => row.push(v),
                        None => break 'outer,
                    }
                }
                rows.push(row);
            }
            for pair in rows.windows(2) {
                let (top, bot) = (&pair[0], &pair[1]);
                for i in 0..per_row - 1 {
                    let (Some(a), Some(b), Some(c), Some(e)) =
                        (top.get(i), top.get(i + 1), bot.get(i), bot.get(i + 1))
                    else {
                        continue;
                    };
                    tris.push([a.clone(), b.clone(), c.clone()]);
                    tris.push([b.clone(), e.clone(), c.clone()]);
                }
            }
        }
        6 | 7 => {
            let n_pts = if ty == 6 { 12 } else { 16 };
            let mut prev: Option<Patch> = None;
            while tris.len() < MAX_TRIANGLES {
                let Some(flag) = r.read(bpf) else { break };
                let (new_pts, new_cols) = if flag == 0 {
                    (n_pts, 4)
                } else {
                    (n_pts - 4, 2)
                };
                let mut pts = Vec::with_capacity(n_pts);
                let mut ok = true;
                for _ in 0..new_pts {
                    match (r.read(bpc), r.read(bpc)) {
                        (Some(x), Some(y)) => {
                            pts.push((scale(x, bpc, dec[0], dec[1]), scale(y, bpc, dec[2], dec[3])))
                        }
                        _ => ok = false,
                    }
                }
                let mut cols = Vec::with_capacity(4);
                for _ in 0..new_cols {
                    let mut c = Vec::with_capacity(n_out);
                    for k in 0..n_out {
                        match (r.read(bpcomp), dec.get(4 + 2 * k), dec.get(5 + 2 * k)) {
                            (Some(v), Some(&lo), Some(&hi)) => c.push(scale(v, bpcomp, lo, hi)),
                            _ => ok = false,
                        }
                    }
                    cols.push(c);
                }
                r.align();
                if !ok {
                    break;
                }
                // Patches after the first share an edge with the previous one.
                let (pts, cols) = match (flag, &prev) {
                    (0, _) => (pts, cols),
                    (f, Some((pp, pc))) => {
                        let edge: [usize; 4] = match f {
                            1 => [3, 4, 5, 6],
                            2 => [6, 7, 8, 9],
                            _ => [9, 10, 11, 0],
                        };
                        let ce: [usize; 2] = match f {
                            1 => [1, 2],
                            2 => [2, 3],
                            _ => [3, 0],
                        };
                        let mut full: Vec<(f64, f64)> =
                            edge.iter().filter_map(|&i| pp.get(i).copied()).collect();
                        full.extend(pts);
                        let mut c: Vec<Vec<f64>> =
                            ce.iter().filter_map(|&i| pc.get(i).cloned()).collect();
                        c.extend(cols);
                        (full, c)
                    }
                    _ => break,
                };
                if pts.len() < 12 || cols.len() < 4 {
                    break;
                }
                patch_triangles(&pts, &cols, &mut tris);
                prev = Some((pts, cols));
            }
        }
        _ => return None,
    }
    Some(tris)
}

fn cubic(p0: (f64, f64), p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (
        a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
        a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
    )
}

/// A Coons patch (the boundary of a tensor patch) split into a grid of
/// triangles with bilinear colours.
fn patch_triangles(p: &[(f64, f64)], c: &[Vec<f64>], out: &mut Vec<[Vertex; 3]>) {
    const N: usize = 10;
    let g = |i: usize| p.get(i).copied().unwrap_or((0.0, 0.0));
    // Boundary curves in the order the specification lists the points.
    let d1 = [g(0), g(1), g(2), g(3)]; // u = 0 side (v from 0 to 1)
    let c2 = [g(3), g(4), g(5), g(6)]; // v = 1
    let d2 = [g(9), g(8), g(7), g(6)]; // u = 1
    let c1 = [g(0), g(11), g(10), g(9)]; // v = 0
    let col = |i: usize| c.get(i).cloned().unwrap_or_default();
    let (c00, c01, c11, c10) = (col(0), col(1), col(2), col(3));
    let point = |u: f64, v: f64| -> Vertex {
        let lc1 = cubic(c1[0], c1[1], c1[2], c1[3], u);
        let lc2 = cubic(c2[0], c2[1], c2[2], c2[3], u);
        let ld1 = cubic(d1[0], d1[1], d1[2], d1[3], v);
        let ld2 = cubic(d2[0], d2[1], d2[2], d2[3], v);
        let (p00, p01, p10, p11) = (g(0), g(3), g(9), g(6));
        let x = (1.0 - v) * lc1.0 + v * lc2.0 + (1.0 - u) * ld1.0 + u * ld2.0
            - ((1.0 - u) * (1.0 - v) * p00.0
                + (1.0 - u) * v * p01.0
                + u * (1.0 - v) * p10.0
                + u * v * p11.0);
        let y = (1.0 - v) * lc1.1 + v * lc2.1 + (1.0 - u) * ld1.1 + u * ld2.1
            - ((1.0 - u) * (1.0 - v) * p00.1
                + (1.0 - u) * v * p01.1
                + u * (1.0 - v) * p10.1
                + u * v * p11.1);
        let n = c00.len();
        let comps = (0..n)
            .map(|k| {
                let q = |v: &Vec<f64>| v.get(k).copied().unwrap_or(0.0);
                (1.0 - u) * (1.0 - v) * q(&c00)
                    + (1.0 - u) * v * q(&c01)
                    + u * v * q(&c11)
                    + u * (1.0 - v) * q(&c10)
            })
            .collect();
        Vertex { x, y, c: comps }
    };
    for i in 0..N {
        for j in 0..N {
            if out.len() + 2 > MAX_TRIANGLES {
                return;
            }
            let (u0, u1) = (i as f64 / N as f64, (i + 1) as f64 / N as f64);
            let (v0, v1) = (j as f64 / N as f64, (j + 1) as f64 / N as f64);
            let (a, b, cc, d) = (point(u0, v0), point(u1, v0), point(u0, v1), point(u1, v1));
            out.push([a, b.clone(), cc.clone()]);
            out.push([b, d, cc]);
        }
    }
}
