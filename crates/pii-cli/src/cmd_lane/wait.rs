//! The poll words and the run id, as the shell's `grep` lines read them (spec 319 R2-R4). Pure.

/// The terminal words of a run's status (`case … *succeeded*|*failed*|*refused*|*cancelled*`).
const TERMINAL: [&str; 4] = ["succeeded", "failed", "refused", "cancelled"];

/// True when a status text carries a terminal word.
pub(crate) fn is_terminal(status: &str) -> bool {
    TERMINAL.iter().any(|w| status.contains(w))
}

/// `grep -o '"status":"[a-z_]*"' | head -1`: the first well-formed status pair, quotes included.
pub(crate) fn status(text: &str) -> String {
    const KEY: &str = "\"status\":\"";
    for (at, _) in text.match_indices(KEY) {
        let rest = &text[at + KEY.len()..];
        let n = rest
            .bytes()
            .take_while(|b| b.is_ascii_lowercase() || *b == b'_')
            .count();
        if rest.as_bytes().get(n) == Some(&b'"') {
            return text[at..=at + KEY.len() + n].to_owned();
        }
    }
    String::new()
}

/// `grep -o 'r[0-9]\{12\}' | head -1`: the first `r` followed by twelve digits.
pub(crate) fn run_id(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    (0..bytes.len()).find_map(|i| {
        let digits = bytes.get(i + 1..i + 13)?;
        (bytes.get(i) == Some(&b'r') && digits.iter().all(u8::is_ascii_digit))
            .then(|| text[i..i + 13].to_owned())
    })
}

/// `grep -o '"session":"[^"]*"' | head -1 | cut -d'"' -f4`: the value of the first session pair (may be empty).
pub(crate) fn session(text: &str) -> String {
    const KEY: &str = "\"session\":\"";
    text.find(KEY)
        .and_then(|at| {
            let rest = &text[at + KEY.len()..];
            rest.find('"').map(|end| rest[..end].to_owned())
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_id_needs_exactly_the_shell_shape() {
        assert_eq!(
            run_id("x r123456789012 y").as_deref(),
            Some("r123456789012")
        );
        assert_eq!(run_id("r12345678901"), None);
        assert_eq!(run_id("r1234567890123").as_deref(), Some("r123456789012"));
    }
}
