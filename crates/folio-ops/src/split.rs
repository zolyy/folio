//! Split Document, with Acrobat Pro's three methods: number of pages, file
//! size, and top-level bookmarks.

use std::collections::{HashMap, HashSet};

use folio_core::{Document, Error, ObjRef, Result};

use crate::{outline, pages};

#[derive(Clone, Copy, Debug)]
pub enum SplitBy {
    /// At most this many pages per file.
    Pages(usize),
    /// At most this many bytes per file (a single larger page gets a file of its own).
    MaxBytes(u64),
    /// One file per top-level bookmark.
    Bookmarks,
}

pub struct Part {
    /// `None` for numbered parts (`_Part1`, ...); the bookmark title otherwise.
    pub title: Option<String>,
    pub pages: Vec<usize>,
    pub doc: Document,
}

pub fn split(doc: &Document, by: SplitBy) -> Result<Vec<Part>> {
    let n = doc.pages()?.len();
    if n == 0 {
        return Err(Error::Invalid("the document has no pages to split".into()));
    }
    let groups: Vec<(Option<String>, Vec<usize>)> = match by {
        SplitBy::Pages(k) => {
            if k == 0 {
                return Err(Error::Invalid("pages per file must be at least 1".into()));
            }
            (0..n)
                .collect::<Vec<_>>()
                .chunks(k)
                .map(|c| (None, c.to_vec()))
                .collect()
        }
        SplitBy::MaxBytes(max) => by_size(doc, max)?.into_iter().map(|g| (None, g)).collect(),
        SplitBy::Bookmarks => by_bookmarks(doc, n)?,
    };
    groups
        .into_iter()
        .map(|(title, idx)| {
            let mut part = doc.clone();
            pages::select(&mut part, &idx)?;
            Ok(Part {
                title,
                pages: idx,
                doc: part,
            })
        })
        .collect()
}

fn by_bookmarks(doc: &Document, n: usize) -> Result<Vec<(Option<String>, Vec<usize>)>> {
    let page_list = doc.pages()?;
    let index: HashMap<u32, usize> = page_list
        .iter()
        .enumerate()
        .map(|(i, r)| (r.num, i))
        .collect();
    let mut starts: Vec<(usize, String)> = outline::top_level(doc)
        .into_iter()
        .filter_map(|b| Some((*index.get(&b.page?.num)?, b.title)))
        .collect();
    starts.sort_by_key(|(i, _)| *i);
    starts.dedup_by_key(|(i, _)| *i);
    if starts.is_empty() {
        return Err(Error::Invalid(
            "the document has no top-level bookmarks that point to pages".into(),
        ));
    }
    let mut out = Vec::new();
    for (k, (start, title)) in starts.iter().enumerate() {
        // Pages before the first bookmark go with the first part (Folio choice).
        let from = if k == 0 { 0 } else { *start };
        let to = starts.get(k + 1).map(|(s, _)| *s).unwrap_or(n);
        if from < to {
            out.push((Some(title.clone()), (from..to).collect()));
        }
    }
    Ok(out)
}

/// Group pages so that each file stays under `max` bytes, counting shared
/// resources (fonts, images) once per file.
fn by_size(doc: &Document, max: u64) -> Result<Vec<Vec<usize>>> {
    if max == 0 {
        return Err(Error::Invalid(
            "the maximum file size must be above 0".into(),
        ));
    }
    let page_list = doc.pages()?;
    let mut sizes: HashMap<u32, u64> = HashMap::new();
    let mut size_of = |num: u32| -> u64 {
        *sizes.entry(num).or_insert_with(|| {
            doc.objects
                .get(&num)
                .map(|o| {
                    let dict = folio_core::writer::serialize(o).len();
                    let data = o.as_stream().map_or(0, |s| s.data.len());
                    (dict + data + 32) as u64
                })
                .unwrap_or(0)
        })
    };
    const OVERHEAD: u64 = 1024;
    let mut groups = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut cur_objs: HashSet<u32> = HashSet::new();
    let mut cur_size = OVERHEAD;
    for (i, &p) in page_list.iter().enumerate() {
        let objs = page_objects(doc, p);
        let extra: u64 = objs
            .iter()
            .filter(|o| !cur_objs.contains(o))
            .map(|&o| size_of(o))
            .sum();
        if !cur.is_empty() && cur_size.saturating_add(extra) > max {
            groups.push(std::mem::take(&mut cur));
            cur_objs.clear();
            cur_size = OVERHEAD;
            let fresh: u64 = objs.iter().map(|&o| size_of(o)).sum();
            cur_size = cur_size.saturating_add(fresh);
        } else {
            cur_size = cur_size.saturating_add(extra);
        }
        cur_objs.extend(objs);
        cur.push(i);
    }
    if !cur.is_empty() {
        groups.push(cur);
    }
    Ok(groups)
}

/// Objects a page needs, not following links back up the tree.
fn page_objects(doc: &Document, page: ObjRef) -> HashSet<u32> {
    let mut seen = HashSet::new();
    let mut stack = vec![page];
    while let Some(r) = stack.pop() {
        if !seen.insert(r.num) || seen.len() > 1_000_000 {
            continue;
        }
        let Some(o) = doc.get(r) else { continue };
        if let Some(d) = o.as_dict() {
            for (k, v) in d.iter() {
                if k == b"Parent" || k == b"P" {
                    continue;
                }
                let mut refs = Vec::new();
                v.collect_refs(&mut refs);
                stack.extend(refs);
            }
        } else {
            let mut refs = Vec::new();
            o.collect_refs(&mut refs);
            stack.extend(refs);
        }
    }
    seen
}

/// A file-system-safe name made from a bookmark title.
pub fn safe_file_name(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let s = s.trim().trim_end_matches('.').trim();
    let s: String = s.chars().take(120).collect();
    if s.is_empty() { "Untitled".into() } else { s }
}
