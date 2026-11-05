//! The spec lane's input JSON, byte-for-byte as Python's `json.dumps` writes it (spec 317 R7). Pure.

use std::fmt::Write as _;

/// Python `json.dumps` of a str with `ensure_ascii`.
pub(crate) fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            '\u{8}' => o.push_str("\\b"),
            '\u{c}' => o.push_str("\\f"),
            ' '..='~' => o.push(c),
            _ => {
                let mut units = [0u16; 2];
                for u in c.encode_utf16(&mut units) {
                    let _ = write!(o, "\\u{u:04x}");
                }
            }
        }
    }
    o.push('"');
    o
}

/// Python `repr` of a float: shortest round-trip, exponent form outside [1e-4, 1e16).
pub(crate) fn py_float(x: f64) -> String {
    if x == 0.0 || (1e-4..1e16).contains(&x.abs()) {
        return format!("{x:?}");
    }
    let e = format!("{x:e}");
    let (mant, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let (sign, digits) = exp.strip_prefix('-').map_or(("+", exp), |d| ("-", d));
    format!("{mant}e{sign}{digits:0>2}")
}

/// `min(float(cap), limit)`. None when the cap does not parse as a finite float.
pub(crate) fn budget(cap: &str, limit: f64) -> Option<f64> {
    let c: f64 = cap.trim().parse().ok()?;
    c.is_finite().then_some(if limit < c { limit } else { c })
}

/// The brief as Python's text-mode `open().read()` returns it: CRLF and CR become LF.
/// None when the bytes are not UTF-8.
pub(crate) fn universal_newlines(bytes: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(bytes).ok()?;
    Some(s.replace("\r\n", "\n").replace('\r', "\n"))
}

/// The fields of the input, in the order the shell writes them.
pub(crate) struct Input<'a> {
    pub(crate) method: &'a str,
    pub(crate) agent: &'a str,
    pub(crate) model: &'a str,
    pub(crate) provider: &'a str,
    pub(crate) spec: &'a str,
    pub(crate) base: &'a str,
    pub(crate) workdir: &'a str,
    pub(crate) brief: &'a str,
    pub(crate) docs_brief: &'a str,
    pub(crate) budget: f64,
    pub(crate) sid: Option<&'a str>,
}

/// `json.dumps` with its default separators, one line, no trailing newline.
pub(crate) fn render(i: &Input<'_>) -> String {
    let sid = i.sid.map_or_else(|| "null".to_owned(), json_str);
    format!(
        "{{\"method\": {}, \"agent\": {}, \"model\": {}, \"provider\": {}, \"spec\": {}, \"base\": {}, \"workdir\": {}, \"brief\": {}, \"docs_brief\": {}, \"max_budget_usd\": {}, \"sid\": {sid}}}",
        json_str(i.method),
        json_str(i.agent),
        json_str(i.model),
        json_str(i.provider),
        json_str(i.spec),
        json_str(i.base),
        json_str(i.workdir),
        json_str(i.brief),
        json_str(i.docs_brief),
        py_float(i.budget),
    )
}
