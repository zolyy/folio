//! Minimal JSON output for `--json` (no dependency needed for writing).

/// A JSON string literal.
pub fn s(v: &str) -> String {
    let mut out = String::with_capacity(v.len() + 2);
    out.push('"');
    for c in v.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `"text"` or `null`.
pub fn opt(v: Option<&str>) -> String {
    v.map_or_else(|| "null".into(), s)
}

/// A JSON number; non-finite values become `null`.
pub fn n(v: f64) -> String {
    if v.is_finite() {
        let r = (v * 100.0).round() / 100.0;
        if r.fract() == 0.0 {
            format!("{}", r as i64)
        } else {
            format!("{r}")
        }
    } else {
        "null".into()
    }
}

/// Build an object from `(key, raw JSON value)` pairs.
pub fn obj(fields: &[(&str, String)]) -> String {
    let body: Vec<String> = fields
        .iter()
        .map(|(k, v)| format!("{}:{v}", s(k)))
        .collect();
    format!("{{{}}}", body.join(","))
}

pub fn arr(items: &[String]) -> String {
    format!("[{}]", items.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes() {
        assert_eq!(s("a\"b\\\n"), "\"a\\\"b\\\\\\n\"");
        assert_eq!(n(f64::NAN), "null");
        assert_eq!(n(612.0), "612");
        assert_eq!(obj(&[("a", n(1.5))]), "{\"a\":1.5}");
    }
}
