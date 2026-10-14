//! Colour spaces (ISO 32000-2 §8.6) and their conversion to sRGB for display.
//! ICC profiles are not applied yet: an ICCBased space is drawn through its
//! component count, as Acrobat does with colour management turned off.

use folio_core::filters::decode_stream;
use folio_core::{Dict, Document, Object};

use crate::function::Function;

#[derive(Clone, Debug)]
pub enum ColorSpace {
    Gray,
    Rgb,
    Cmyk,
    Lab {
        white: [f64; 3],
        range: [f64; 4],
    },
    Indexed {
        base: Box<ColorSpace>,
        hival: usize,
        lookup: Vec<u8>,
    },
    /// Separation (n = 1) and DeviceN.
    Tint {
        n: usize,
        alt: Box<ColorSpace>,
        f: Option<Function>,
        /// `/None` separation: paints nothing.
        none: bool,
    },
    /// Patterns are drawn in stage 3d; until then their area is left unpainted.
    Pattern,
}

impl ColorSpace {
    pub fn components(&self) -> usize {
        match self {
            ColorSpace::Gray | ColorSpace::Indexed { .. } | ColorSpace::Pattern => 1,
            ColorSpace::Rgb | ColorSpace::Lab { .. } => 3,
            ColorSpace::Cmyk => 4,
            ColorSpace::Tint { n, .. } => *n,
        }
    }

    /// The initial colour after `cs`/`CS` (§8.6.5).
    pub fn initial(&self) -> Vec<f64> {
        match self {
            ColorSpace::Cmyk => vec![0.0, 0.0, 0.0, 1.0],
            ColorSpace::Lab { range, .. } => vec![0.0, range[0].max(0.0), range[2].max(0.0)],
            ColorSpace::Tint { n, .. } => vec![1.0; *n],
            other => vec![0.0; other.components()],
        }
    }

    /// Resolve a colour-space operand: a name (device, or a key in the
    /// resources' /ColorSpace) or an array.
    pub fn parse(doc: &Document, o: &Object, res: Option<&Dict>) -> Option<ColorSpace> {
        Self::parse_depth(doc, o, res, 0)
    }

    fn parse_depth(
        doc: &Document,
        o: &Object,
        res: Option<&Dict>,
        depth: usize,
    ) -> Option<ColorSpace> {
        if depth > 6 {
            return None;
        }
        match doc.resolve(o) {
            Object::Name(n) => match n.as_slice() {
                b"DeviceGray" | b"G" | b"CalGray" => Some(ColorSpace::Gray),
                b"DeviceRGB" | b"RGB" | b"CalRGB" => Some(ColorSpace::Rgb),
                b"DeviceCMYK" | b"CMYK" => Some(ColorSpace::Cmyk),
                b"Pattern" => Some(ColorSpace::Pattern),
                other => {
                    let named = res
                        .and_then(|r| doc.resolve_key(r, b"ColorSpace"))
                        .and_then(Object::as_dict)
                        .and_then(|cs| cs.get(other))?;
                    Self::parse_depth(doc, named, None, depth + 1)
                }
            },
            Object::Array(a) => {
                let family = a
                    .first()
                    .map(|f| doc.resolve(f))
                    .and_then(Object::as_name)?;
                let arg = |i: usize| a.get(i);
                match family {
                    b"DeviceGray" | b"CalGray" | b"G" => Some(ColorSpace::Gray),
                    b"DeviceRGB" | b"CalRGB" | b"RGB" => Some(ColorSpace::Rgb),
                    b"DeviceCMYK" | b"CMYK" => Some(ColorSpace::Cmyk),
                    b"ICCBased" => {
                        let s = doc.resolve(arg(1)?).as_stream()?;
                        if let Some(alt) = s.dict.get(b"Alternate")
                            && let Some(cs) = Self::parse_depth(doc, alt, None, depth + 1)
                        {
                            return Some(cs);
                        }
                        Some(match s.dict.get_int(b"N").unwrap_or(3) {
                            1 => ColorSpace::Gray,
                            4 => ColorSpace::Cmyk,
                            _ => ColorSpace::Rgb,
                        })
                    }
                    b"Lab" => {
                        let d = doc.resolve(arg(1)?).as_dict();
                        let get = |k: &[u8], n: usize, def: &[f64]| -> Vec<f64> {
                            let v: Vec<f64> = d
                                .and_then(|d| doc.resolve_key(d, k))
                                .and_then(Object::as_array)
                                .map(|a| a.iter().filter_map(|x| doc.resolve(x).as_f64()).collect())
                                .unwrap_or_default();
                            if v.len() >= n { v } else { def.to_vec() }
                        };
                        let w = get(b"WhitePoint", 3, &[0.9505, 1.0, 1.089]);
                        let r = get(b"Range", 4, &[-100.0, 100.0, -100.0, 100.0]);
                        Some(ColorSpace::Lab {
                            white: [w[0], w[1], w[2]],
                            range: [r[0], r[1], r[2], r[3]],
                        })
                    }
                    b"Indexed" | b"I" => {
                        let base = Self::parse_depth(doc, arg(1)?, res, depth + 1)?;
                        let hival =
                            doc.resolve(arg(2)?).as_int().unwrap_or(0).clamp(0, 255) as usize;
                        let lookup = match doc.resolve(arg(3)?) {
                            Object::Str(s) => s.clone(),
                            Object::Stream(s) => decode_stream(s).unwrap_or_default(),
                            _ => Vec::new(),
                        };
                        Some(ColorSpace::Indexed {
                            base: Box::new(base),
                            hival,
                            lookup,
                        })
                    }
                    b"Separation" => {
                        let name = doc.resolve(arg(1)?).as_name().unwrap_or_default();
                        let alt = Self::parse_depth(doc, arg(2)?, res, depth + 1)?;
                        Some(ColorSpace::Tint {
                            n: 1,
                            none: name == b"None",
                            alt: Box::new(alt),
                            f: Function::parse(doc, arg(3)?),
                        })
                    }
                    b"DeviceN" => {
                        let n = doc.resolve(arg(1)?).as_array()?.len().clamp(1, 32);
                        let alt = Self::parse_depth(doc, arg(2)?, res, depth + 1)?;
                        Some(ColorSpace::Tint {
                            n,
                            none: false,
                            alt: Box::new(alt),
                            f: Function::parse(doc, arg(3)?),
                        })
                    }
                    b"Pattern" => Some(ColorSpace::Pattern),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// sRGB in 0..=1, or `None` when nothing should be painted.
    pub fn to_rgb(&self, c: &[f64]) -> Option<[f32; 3]> {
        self.to_rgb_depth(c, 0)
    }

    fn to_rgb_depth(&self, c: &[f64], depth: usize) -> Option<[f32; 3]> {
        if depth > 6 {
            return None;
        }
        let at = |i: usize| {
            let v = c.get(i).copied().unwrap_or(0.0);
            if v.is_finite() {
                v.clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        Some(match self {
            ColorSpace::Gray => {
                let g = at(0) as f32;
                [g, g, g]
            }
            ColorSpace::Rgb => [at(0) as f32, at(1) as f32, at(2) as f32],
            ColorSpace::Cmyk => cmyk_to_rgb(at(0), at(1), at(2), at(3)),
            ColorSpace::Lab { white, .. } => {
                let raw = |i: usize| c.get(i).copied().filter(|v| v.is_finite()).unwrap_or(0.0);
                lab_to_rgb(raw(0), raw(1), raw(2), white)
            }
            ColorSpace::Indexed {
                base,
                hival,
                lookup,
            } => {
                let idx = c.first().copied().unwrap_or(0.0);
                let idx = if idx.is_finite() { idx.round() } else { 0.0 };
                let idx = (idx.max(0.0) as usize).min(*hival);
                let n = base.components();
                let comps: Vec<f64> = (0..n)
                    .map(|k| f64::from(lookup.get(idx * n + k).copied().unwrap_or(0)) / 255.0)
                    .collect();
                return base.to_rgb_depth(&comps, depth + 1);
            }
            ColorSpace::Tint { n, alt, f, none } => {
                if *none {
                    return None;
                }
                let input: Vec<f64> = (0..*n).map(at).collect();
                let out = match f {
                    Some(f) => f.eval(&input),
                    None => vec![at(0); alt.components()],
                };
                return alt.to_rgb_depth(&out, depth + 1);
            }
            ColorSpace::Pattern => return None,
        })
    }
}

/// The simple, profile-free conversion every viewer falls back to.
pub fn cmyk_to_rgb(c: f64, m: f64, y: f64, k: f64) -> [f32; 3] {
    [
        ((1.0 - c) * (1.0 - k)) as f32,
        ((1.0 - m) * (1.0 - k)) as f32,
        ((1.0 - y) * (1.0 - k)) as f32,
    ]
}

/// CIE L*a*b* → XYZ (with the space's white point) → linear sRGB → sRGB.
pub fn lab_to_rgb(l: f64, a: f64, b: f64, white: &[f64; 3]) -> [f32; 3] {
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let g = |t: f64| {
        if t > 6.0 / 29.0 {
            t * t * t
        } else {
            3.0 * (6.0f64 / 29.0).powi(2) * (t - 4.0 / 29.0)
        }
    };
    let (x, y, z) = (white[0] * g(fx), white[1] * g(fy), white[2] * g(fz));
    let r = 3.2406 * x - 1.5372 * y - 0.4986 * z;
    let gg = -0.9689 * x + 1.8758 * y + 0.0415 * z;
    let bb = 0.0557 * x - 0.2040 * y + 1.0570 * z;
    let enc = |v: f64| {
        let v = v.clamp(0.0, 1.0);
        let s = if v <= 0.0031308 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        s as f32
    };
    [enc(r), enc(gg), enc(bb)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_spaces() {
        assert_eq!(
            ColorSpace::Cmyk.to_rgb(&[0.0, 1.0, 1.0, 0.0]),
            Some([1.0, 0.0, 0.0])
        );
        assert_eq!(ColorSpace::Gray.to_rgb(&[2.0]), Some([1.0, 1.0, 1.0]));
        assert_eq!(
            ColorSpace::Rgb.to_rgb(&[f64::NAN, 0.5, 1.0]),
            Some([0.0, 0.5, 1.0])
        );
    }

    #[test]
    fn indexed_lookup_is_clamped() {
        let cs = ColorSpace::Indexed {
            base: Box::new(ColorSpace::Rgb),
            hival: 1,
            lookup: vec![255, 0, 0, 0, 0, 255],
        };
        assert_eq!(cs.to_rgb(&[1.0]), Some([0.0, 0.0, 1.0]));
        assert_eq!(cs.to_rgb(&[99.0]), Some([0.0, 0.0, 1.0]));
    }

    #[test]
    fn lab_white_is_white() {
        let [r, g, b] = lab_to_rgb(100.0, 0.0, 0.0, &[0.9505, 1.0, 1.089]);
        assert!(r > 0.99 && g > 0.99 && b > 0.99, "{r} {g} {b}");
    }
}
