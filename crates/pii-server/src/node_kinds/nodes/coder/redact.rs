//! Scrubs secrets out of an agent's captured output before it is journalled or stored.
//!
//! The coder journals the head of the agent's stdout and stderr, and the journal is also written to the store. Two
//! sources are removed: the exact VALUES of the credentials pii itself passed the agent, and token-shaped strings the
//! agent produced on its own.
//!
//! A scrub that is too eager mangles every path, so each rule has a floor: a credential value must be long enough to
//! be unique, and a token must carry a prefix AND a long enough body (`sk-learn` is a library name, not a key).

use std::collections::BTreeMap;

/// What replaces a removed secret.
pub const REDACTED: &str = "[REDACTED]";

/// A credential value shorter than this is not scrubbed by value: it would match unrelated text everywhere.
const MIN_VALUE_LEN: usize = 8;

/// Env names that hold a credential. A name containing one of these (case-insensitive) is a credential.
const CREDENTIAL_MARKERS: [&str; 5] = ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"];

/// Token shapes the agent may print on its own: the prefix, and how many token characters must follow it.
const TOKEN_PREFIXES: [(&str, usize); 6] = [
    ("sk-", 16),
    ("ghp_", 16),
    ("gho_", 16),
    ("github_pat_", 16),
    ("xoxb-", 16),
    ("Bearer ", 16),
];

/// The values of the credential-named variables in `env`, long enough to scrub by value.
#[must_use]
pub fn credential_values(env: &BTreeMap<String, String>) -> Vec<String> {
    env.iter()
        .filter(|(name, value)| {
            let upper = name.to_ascii_uppercase();
            value.len() >= MIN_VALUE_LEN && CREDENTIAL_MARKERS.iter().any(|m| upper.contains(m))
        })
        .map(|(_, value)| value.clone())
        .collect()
}

/// `text` with every one of `secrets` replaced, then every token-shaped string.
#[must_use]
pub fn redact(text: &str, secrets: &[String]) -> String {
    let mut scrubbed = text.to_owned();
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        scrubbed = scrubbed.replace(secret.as_str(), REDACTED);
    }
    scrub_tokens(&scrubbed)
}

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'+' | b'/' | b'=')
}

/// The length of the token at the start of `rest`, when one starts there.
fn token_len(rest: &str) -> Option<usize> {
    TOKEN_PREFIXES.iter().find_map(|(prefix, min_body)| {
        let body = rest.strip_prefix(prefix)?;
        let n = body.bytes().take_while(|b| is_token_byte(*b)).count();
        (n >= *min_body).then_some(prefix.len() + n)
    })
}

fn scrub_tokens(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    let mut prev: Option<char> = None;
    while let Some(c) = rest.chars().next() {
        // A token starts at a boundary: `ask-...` inside a word is not a key.
        let at_boundary = prev.is_none_or(|p| !p.is_ascii_alphanumeric());
        if at_boundary && let Some(n) = token_len(rest) {
            out.push_str(REDACTED);
            rest = rest.get(n..).unwrap_or("");
            prev = Some(']');
            continue;
        }
        out.push(c);
        rest = rest.get(c.len_utf8()..).unwrap_or("");
        prev = Some(c);
    }
    out
}
