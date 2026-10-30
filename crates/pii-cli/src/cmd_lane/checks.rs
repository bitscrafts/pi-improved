//! The text scans the shell does with `sed`, `grep`, `sha256sum` and `tomllib` (spec 317 R6, R7). Pure.

use sha2::{Digest, Sha256};
use std::fmt::Write as _;

/// `sed -n 's/^max_wall_ms *= *\([0-9][0-9]*\).*/\1/p' | head -1`: the first matching line's digits.
pub(crate) fn max_wall_ms(text: &[u8]) -> Option<String> {
    text.split(|&b| b == b'\n').find_map(|line| {
        let rest = line.strip_prefix(b"max_wall_ms")?;
        let rest = rest.get(spaces(rest)..)?.strip_prefix(b"=")?;
        let rest = rest.get(spaces(rest)..)?;
        let n = rest.iter().take_while(|b| b.is_ascii_digit()).count();
        (n > 0).then(|| String::from_utf8_lossy(rest.get(..n).unwrap_or_default()).into_owned())
    })
}

/// Leading spaces (not tabs) in `s`.
fn spaces(s: &[u8]) -> usize {
    s.iter().take_while(|&&b| b == b' ').count()
}

/// `grep -o 'sha256 = "[0-9a-f]*"' | head -1 | cut -d'"' -f2`: the first match anywhere, comments included.
pub(crate) fn pin(text: &[u8]) -> String {
    const KEY: &[u8] = b"sha256 = \"";
    for line in text.split(|&b| b == b'\n') {
        let mut from = 0;
        while let Some(rel) = line.get(from..).and_then(|s| find(s, KEY)) {
            let start = from + rel + KEY.len();
            let hex = line
                .get(start..)
                .unwrap_or_default()
                .iter()
                .take_while(|&&b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                .count();
            if line.get(start + hex) == Some(&b'"') {
                return String::from_utf8_lossy(line.get(start..start + hex).unwrap_or_default())
                    .into_owned();
            }
            from += rel + 1;
        }
    }
    String::new()
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Lowercase hex SHA-256 of `bytes` (`sha256sum | cut -c1-64`).
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// `limits.max_micro_usd` of the working-tree graph, read with the TOML parser (`tomllib` in the shell).
#[allow(clippy::cast_precision_loss)] // reason: TOML integers are far below 2^53
pub(crate) fn limit_micro_usd(text: &[u8]) -> Option<f64> {
    let text = std::str::from_utf8(text).ok()?;
    let doc: toml::Table = toml::from_str(text).ok()?;
    let v = doc.get("limits")?.as_table()?.get("max_micro_usd")?;
    // The script's `max_micro_usd / 1e6`.
    let micro = v.as_integer().map(|i| i as f64).or_else(|| v.as_float())?;
    Some(micro / 1e6)
}
