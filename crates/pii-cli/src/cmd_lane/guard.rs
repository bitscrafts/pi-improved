//! The spec guard's set arithmetic (spec 319 R6): `git status --porcelain -- specs | sort`, then `comm -13`. Pure.

/// `sort` of the porcelain lines: each non-empty line, in bytewise order.
pub(crate) fn sorted(raw: &[u8]) -> Vec<String> {
    let mut lines: Vec<String> = String::from_utf8_lossy(raw)
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    lines.sort();
    lines
}

/// The sorted lines as a file: each line followed by a newline.
pub(crate) fn render(lines: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for line in lines {
        out.extend_from_slice(line.as_bytes());
        out.push(b'\n');
    }
    out
}

/// `comm -13 before after`: the lines only in `after`, both sorted.
pub(crate) fn touched(before: &[String], after: &[String]) -> Vec<String> {
    after
        .iter()
        .filter(|l| before.binary_search(l).is_err())
        .cloned()
        .collect()
}
