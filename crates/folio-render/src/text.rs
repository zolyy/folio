//! The text of a rendered page as the reader sees it: every character with
//! its quadrilateral on the page, in reading order, with spaces and line
//! breaks inferred from geometry. Search (Acrobat's Find), highlighting and
//! text selection are built on it.
//!
//! Coordinates are points on the displayed page (rotation applied), origin
//! top-left, so they do not depend on the zoom the page was rendered at.

/// A point on the displayed page, in points from the top-left corner.
pub type Pt = [f32; 2];

/// One shown glyph: its text and its box (bottom-left, bottom-right,
/// top-right, top-left of the glyph's em box, as drawn).
#[derive(Clone, Debug, PartialEq)]
pub struct Glyph {
    pub text: String,
    pub quad: [Pt; 4],
}

/// A page's text with one entry in `boxes` per character of `text`.
/// Spaces and line breaks Folio inferred have no box.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageText {
    pub text: String,
    pub boxes: Vec<Option<[Pt; 4]>>,
}

fn sub(a: Pt, b: Pt) -> Pt {
    [a[0] - b[0], a[1] - b[1]]
}

fn len(v: Pt) -> f32 {
    v[0].hypot(v[1])
}

fn lerp(a: Pt, b: Pt, t: f32) -> Pt {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// Arrange glyphs (in content order) into text.
pub fn layout(glyphs: &[Glyph]) -> PageText {
    let mut out = PageText::default();
    let mut prev: Option<&Glyph> = None;
    for g in glyphs {
        if g.text.is_empty() {
            continue;
        }
        let [bl, br, _tr, tl] = g.quad;
        let height = len(sub(tl, bl)).max(0.01);
        if let Some(p) = prev {
            let [pbl, pbr, ..] = p.quad;
            let dir = {
                let d = sub(pbr, pbl);
                let l = len(d);
                if l > 1e-4 {
                    [d[0] / l, d[1] / l]
                } else {
                    [1.0, 0.0]
                }
            };
            // Distance across the previous baseline, and along it past its end.
            let rel = sub(bl, pbl);
            let across = (rel[0] * dir[1] - rel[1] * dir[0]).abs();
            let gap = {
                let r = sub(bl, pbr);
                r[0] * dir[0] + r[1] * dir[1]
            };
            let ends_ws = out.text.ends_with(char::is_whitespace);
            let starts_ws = g.text.starts_with(char::is_whitespace);
            if across > 0.5 * height || gap < -2.0 * height {
                if !out.text.ends_with('\n') {
                    out.text.push('\n');
                    out.boxes.push(None);
                }
            } else if gap > 0.15 * height && !ends_ws && !starts_ws {
                out.text.push(' ');
                out.boxes.push(None);
            }
        }
        // Several characters from one glyph (ligatures) share its width.
        let n = g.text.chars().count().max(1) as f32;
        for (i, c) in g.text.chars().enumerate() {
            let (t0, t1) = (i as f32 / n, (i + 1) as f32 / n);
            let q = [
                lerp(bl, br, t0),
                lerp(bl, br, t1),
                lerp(tl, g.quad[2], t1),
                lerp(tl, g.quad[2], t0),
            ];
            out.text.push(c);
            out.boxes.push(Some(q));
        }
        prev = Some(g);
    }
    out
}

/// Acrobat's Find options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub whole_words: bool,
    pub case_sensitive: bool,
}

fn fold(c: char, case_sensitive: bool) -> char {
    if case_sensitive {
        c
    } else {
        c.to_lowercase().next().unwrap_or(c)
    }
}

/// Every match of `query`, as ranges of character indices into `text.text`.
/// Runs of white space match any run of white space, so a phrase is found
/// across a line break, as in Acrobat.
pub fn find(text: &PageText, query: &str, opts: FindOptions) -> Vec<std::ops::Range<usize>> {
    // Normalise both sides: collapse white space, fold case.
    let mut hay: Vec<(char, usize)> = Vec::new();
    for (i, c) in text.text.chars().enumerate() {
        if c.is_whitespace() {
            if hay.last().is_some_and(|(p, _)| *p == ' ') {
                continue;
            }
            hay.push((' ', i));
        } else {
            hay.push((fold(c, opts.case_sensitive), i));
        }
    }
    let mut needle: Vec<char> = Vec::new();
    for c in query.trim().chars() {
        if c.is_whitespace() {
            if needle.last() != Some(&' ') {
                needle.push(' ');
            }
        } else {
            needle.push(fold(c, opts.case_sensitive));
        }
    }
    let total = text.text.chars().count();
    let mut out = Vec::new();
    if needle.is_empty() || needle.len() > hay.len() {
        return out;
    }
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        let matched = hay
            .get(i..i + needle.len())
            .is_some_and(|w| w.iter().zip(&needle).all(|((h, _), n)| h == n));
        if matched {
            let start = hay.get(i).map_or(0, |(_, s)| *s);
            let last = hay.get(i + needle.len() - 1).map_or(start, |(_, s)| *s);
            let end = (last + 1).min(total);
            let ok = !opts.whole_words || {
                let before = i.checked_sub(1).and_then(|k| hay.get(k)).map(|(c, _)| *c);
                let after = hay.get(i + needle.len()).map(|(c, _)| *c);
                !word(before) && !word(after)
            };
            if ok {
                out.push(start..end);
                i += needle.len();
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Rectangles to highlight a range: one quad per run of characters on the
/// same line.
pub fn quads(text: &PageText, range: std::ops::Range<usize>) -> Vec<[Pt; 4]> {
    let mut out: Vec<[Pt; 4]> = Vec::new();
    let mut run: Option<[Pt; 4]> = None;
    // Never walk past the text, whatever range a caller passes.
    let end = range.end.min(text.boxes.len());
    let start = range.start.min(end);
    let chars: Vec<char> = text.text.chars().collect();
    for i in start..end {
        match text.boxes.get(i).copied().flatten() {
            Some(q) => {
                run = Some(match run {
                    // Same line: extend to this character's right edge.
                    Some(r) if (q[0][1] - r[1][1]).abs() < 0.5 * len(sub(r[3], r[0])).max(0.01) => {
                        [r[0], q[1], q[2], r[3]]
                    }
                    Some(r) => {
                        out.push(r);
                        q
                    }
                    None => q,
                });
            }
            None => {
                // A line break ends the run; an inferred space joins it.
                if text.text.chars().nth(i) == Some('\n')
                    && let Some(r) = run.take()
                {
                    out.push(r);
                }
            }
        }
    }
    if let Some(r) = run {
        out.push(r);
    }
    out
}

/// The character nearest to `p` (inside its box if possible), for selection.
pub fn char_at(text: &PageText, p: Pt) -> Option<usize> {
    let mut best: Option<(f32, usize)> = None;
    for (i, b) in text.boxes.iter().enumerate() {
        let Some(q) = b else { continue };
        let xs = q.iter().map(|v| v[0]);
        let ys = q.iter().map(|v| v[1]);
        let (x0, x1) = (
            xs.clone().fold(f32::MAX, f32::min),
            xs.fold(f32::MIN, f32::max),
        );
        let (y0, y1) = (
            ys.clone().fold(f32::MAX, f32::min),
            ys.fold(f32::MIN, f32::max),
        );
        let dx = if p[0] < x0 {
            x0 - p[0]
        } else if p[0] > x1 {
            p[0] - x1
        } else {
            0.0
        };
        let dy = if p[1] < y0 {
            y0 - p[1]
        } else if p[1] > y1 {
            p[1] - y1
        } else {
            0.0
        };
        // Being on the same line matters more than horizontal distance.
        let d = dx + 4.0 * dy;
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, i));
        }
    }
    best.map(|(_, i)| i)
}

/// The text of a character range (selection → clipboard).
pub fn slice(text: &PageText, range: std::ops::Range<usize>) -> String {
    text.text
        .chars()
        .skip(range.start)
        .take(range.end.saturating_sub(range.start))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(text: &str, x: f32, y: f32, w: f32) -> Glyph {
        // A 10-pt glyph whose baseline-left corner is at (x, y).
        Glyph {
            text: text.into(),
            quad: [[x, y], [x + w, y], [x + w, y - 10.0], [x, y - 10.0]],
        }
    }

    fn page() -> PageText {
        layout(&[
            g("H", 0.0, 20.0, 6.0),
            g("i", 6.0, 20.0, 3.0),
            g("t", 14.0, 20.0, 4.0), // gap -> space
            g("h", 18.0, 20.0, 5.0),
            g("e", 23.0, 20.0, 5.0),
            g("r", 28.0, 20.0, 4.0),
            g("e", 32.0, 20.0, 5.0),
            g("t", 0.0, 40.0, 4.0), // next line
            g("h", 4.0, 40.0, 5.0),
            g("e", 9.0, 40.0, 5.0),
            g("ﬁ", 14.0, 40.0, 6.0), // ligature: two characters
            g("n", 20.0, 40.0, 5.0),
        ])
    }

    #[test]
    fn layout_infers_spaces_lines_and_splits_ligatures() {
        let p = page();
        assert_eq!(p.text, "Hi there\ntheﬁn");
        assert_eq!(p.text.chars().count(), p.boxes.len());
        let lig = layout(&[g("fi", 0.0, 10.0, 6.0)]);
        assert_eq!(lig.text, "fi");
        assert_eq!(lig.boxes[1].unwrap()[0], [3.0, 10.0]);
    }

    #[test]
    fn find_with_acrobat_options() {
        let p = page();
        let any = FindOptions::default();
        assert_eq!(find(&p, "THERE", any).len(), 1);
        assert!(
            find(
                &p,
                "THERE",
                FindOptions {
                    case_sensitive: true,
                    ..any
                }
            )
            .is_empty()
        );
        // "the" occurs inside "there" and as its own word on line 2? No: "theﬁn".
        assert_eq!(find(&p, "the", any).len(), 2);
        let whole = FindOptions {
            whole_words: true,
            ..any
        };
        assert!(find(&p, "the", whole).is_empty());
        assert_eq!(find(&p, "there", whole).len(), 1);
        // Phrases match across a line break.
        let r = find(&p, "there the", any);
        assert_eq!(r.len(), 1);
        assert_eq!(slice(&p, r[0].clone()), "there\nthe");
    }

    #[test]
    fn highlight_quads_split_by_line() {
        let p = page();
        let r = find(&p, "there the", FindOptions::default())[0].clone();
        let q = quads(&p, r);
        assert_eq!(q.len(), 2);
        assert_eq!(q[0][0], [14.0, 20.0]);
        assert_eq!(q[0][1], [37.0, 20.0]);
    }

    #[test]
    fn selection_picks_the_nearest_character() {
        let p = page();
        assert_eq!(char_at(&p, [7.0, 15.0]), Some(1)); // "i"
        let a = char_at(&p, [1.0, 35.0]).unwrap();
        assert_eq!(p.text.chars().nth(a), Some('t'));
        assert_eq!(slice(&p, 0..2), "Hi");
    }
}
