//! Page ranges as people type them: `1-3,5,8-`, `last`, `odd`, `even`, `all`.
//! Pages are numbered from 1 in the syntax and from 0 in the result.

use std::collections::BTreeSet;

use folio_core::{Error, Result};

/// Pages in the order written. Duplicates are kept (`1,1` = page 1 twice).
/// `5-2` counts down. An empty spec means every page.
pub fn parse(spec: &str, total: usize) -> Result<Vec<usize>> {
    if total == 0 {
        return Err(Error::Invalid("the document has no pages".into()));
    }
    let mut out = Vec::new();
    let spec = spec.trim();
    if spec.is_empty() {
        return Ok((0..total).collect());
    }
    for part in spec.split(',') {
        let part = part.trim().to_ascii_lowercase();
        match part.as_str() {
            "" => continue,
            "all" => out.extend(0..total),
            "odd" => out.extend((0..total).step_by(2)),
            "even" => out.extend((1..total).step_by(2)),
            _ => {
                if let Some((a, b)) = part.split_once('-') {
                    let a = if a.trim().is_empty() {
                        1
                    } else {
                        page_number(a, total)?
                    };
                    let b = if b.trim().is_empty() {
                        total
                    } else {
                        page_number(b, total)?
                    };
                    if a <= b {
                        out.extend(a - 1..b);
                    } else {
                        out.extend((b - 1..a).rev());
                    }
                } else {
                    out.push(page_number(&part, total)? - 1);
                }
            }
        }
        if out.len() > 10_000_000 {
            return Err(Error::Limit("page range lists too many pages".into()));
        }
    }
    if out.is_empty() {
        return Err(Error::Invalid(format!("`{spec}` selects no pages")));
    }
    Ok(out)
}

/// The same pages, sorted and without duplicates.
pub fn parse_set(spec: &str, total: usize) -> Result<BTreeSet<usize>> {
    Ok(parse(spec, total)?.into_iter().collect())
}

fn page_number(s: &str, total: usize) -> Result<usize> {
    let s = s.trim();
    if s == "last" || s == "end" {
        return Ok(total);
    }
    let n: usize = s.parse().map_err(|_| {
        Error::Invalid(format!(
            "`{s}` is not a page number (use numbers, ranges like 2-5, `last`, `odd`, `even`)"
        ))
    })?;
    if n == 0 || n > total {
        return Err(Error::Invalid(format!(
            "page {n} does not exist; the document has {total} page{}",
            if total == 1 { "" } else { "s" }
        )));
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_range_syntax() {
        assert_eq!(parse("1-3,5", 6).unwrap(), vec![0, 1, 2, 4]);
        assert_eq!(parse("5-", 6).unwrap(), vec![4, 5]);
        assert_eq!(parse("-2", 6).unwrap(), vec![0, 1]);
        assert_eq!(parse("last,1", 6).unwrap(), vec![5, 0]);
        assert_eq!(parse("3-1", 6).unwrap(), vec![2, 1, 0]);
        assert_eq!(parse("odd", 5).unwrap(), vec![0, 2, 4]);
        assert_eq!(parse("even", 5).unwrap(), vec![1, 3]);
        assert_eq!(parse("", 3).unwrap(), vec![0, 1, 2]);
    }

    #[test]
    fn page_range_errors_say_what_is_allowed() {
        let e = parse("9", 5).unwrap_err().to_string();
        assert!(e.contains("5 pages"), "{e}");
        assert!(parse("0", 5).is_err());
        assert!(parse("x", 5).unwrap_err().to_string().contains("ranges"));
        assert!(parse("1", 0).is_err());
    }
}
