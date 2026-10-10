//! Secret scrubbing: replaces the values of secret-looking environment
//! variables wherever they would be written by a sink (log file, trajectory,
//! ndjson stream).
//!
//! Secrets are delivered by environment only; this is the compensating
//! control for the day one of them is echoed into a tool output or a model
//! reply. It scrubs the exact values known at startup — it is not a pattern
//! detector, so a secret that is transformed (hashed, split, re-encoded) is
//! not caught.

use std::borrow::Cow;

/// The text a scrubbed secret is replaced with.
pub const REDACTED: &str = "[REDACTED]";

/// Values shorter than this are never scrubbed: replacing `1` or `true`
/// everywhere would corrupt the record without protecting anything.
const MIN_SECRET_LEN: usize = 8;

/// Name fragments (matched case-insensitively) that mark a variable secret.
const SECRET_NAME_PARTS: &[&str] = &[
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "API_KEY",
    "APIKEY",
    "PRIVATE_KEY",
    "CREDENTIAL",
];

/// Whether an environment variable name marks its value as a secret.
#[must_use]
pub fn is_secret_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    SECRET_NAME_PARTS.iter().any(|p| upper.contains(p)) || upper.ends_with("_KEY")
}

/// Replaces known secret values with [`REDACTED`].
#[derive(Debug, Clone, Default)]
pub struct SecretScrubber {
    secrets: Vec<String>,
}

impl SecretScrubber {
    /// A scrubber over the secret-named entries of `vars` (name, value).
    #[must_use]
    pub fn from_vars(vars: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut secrets: Vec<String> = vars
            .into_iter()
            .filter(|(n, v)| is_secret_name(n) && v.len() >= MIN_SECRET_LEN)
            .map(|(_, v)| v)
            .collect();
        // Longest first, so a secret that contains another is replaced whole.
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        secrets.dedup();
        Self { secrets }
    }

    /// Whether anything is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.secrets.is_empty()
    }

    /// `text` with every known secret replaced; borrowed when none occurs.
    #[must_use]
    pub fn scrub<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let mut out = Cow::Borrowed(text);
        for s in &self.secrets {
            if out.contains(s.as_str()) {
                out = Cow::Owned(out.replace(s.as_str(), REDACTED));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn vars(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect()
    }

    #[test]
    fn only_secret_named_long_values_are_scrubbed() {
        let s = SecretScrubber::from_vars(vars(&[
            ("OLLAMA_API_KEY", "key-aaaaaaaa"),
            ("PII_DISCORD_TOKEN", "tok-bbbbbbbb"),
            ("PATH", "/usr/bin:/bin:/usr/local/bin"),
            ("MY_TOKEN", "short"),
        ]));
        let t = s.scrub("a key-aaaaaaaa b tok-bbbbbbbb c /usr/bin:/bin:/usr/local/bin d short");
        assert_eq!(
            t,
            "a [REDACTED] b [REDACTED] c /usr/bin:/bin:/usr/local/bin d short"
        );
    }

    #[test]
    fn text_without_a_secret_is_borrowed_unchanged() {
        let s = SecretScrubber::from_vars(vars(&[("X_SECRET", "s3cr3t-value")]));
        assert!(matches!(s.scrub("nothing here"), Cow::Borrowed(_)));
        assert!(SecretScrubber::default().is_empty());
    }

    #[test]
    fn name_matching_is_case_insensitive_and_catches_key_suffix() {
        assert!(is_secret_name("github_token"));
        assert!(is_secret_name("SSH_KEY"));
        assert!(!is_secret_name("KEYBOARD"));
        assert!(!is_secret_name("HOME"));
    }
}
