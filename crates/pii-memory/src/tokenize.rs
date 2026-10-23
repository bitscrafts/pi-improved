//! Tokenisation (F4.4): lowercase, split on non-alphanumeric, drop tokens
//! shorter than 2 chars, drop a fixed 40-word stoplist. No stemming.

/// The stoplist (F4.4: exactly these words, never a heuristic).
pub const STOPLIST: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "if", "then", "of", "to", "in", "on", "for", "with",
    "at", "by", "from", "as", "is", "are", "was", "were", "be", "been", "it", "this", "that",
    "these", "those", "we", "you", "they", "he", "she", "his", "her", "items", "not", "no", "do",
];

/// Lowercases, splits on non-alphanumeric, drops short tokens and the
/// stoplist. Total: never panics on any input.
#[must_use]
pub fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| {
            t.chars().count() >= 2 && !STOPLIST.contains(t) && !t.chars().all(char::is_numeric)
        })
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::tokenize;

    #[test]
    fn lowercases_splits_drops() {
        assert_eq!(
            tokenize("The Redb-Store; holds! 42 items"),
            vec![
                String::from("redb"),
                String::from("store"),
                String::from("holds"),
            ]
        );
        assert_eq!(tokenize("a I x"), [] as [String; 0]);
        assert_eq!(tokenize(""), Vec::<String>::new());
    }
}
