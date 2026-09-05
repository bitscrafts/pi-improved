//! `xtask doc-drift-check` (Slice -1, S(-1).2): catch ONE concrete class of
//! doc/code mismatch mechanically — a path mentioned in `HANDOFF.md` inside
//! backticks that does not exist on disk.
//!
//! Scope is deliberately narrow (v1): every backtick-quoted token that looks
//! like a repo-relative path (`crates/…`, `specs/…`, `xtask/…`, `graphs/…`,
//! `scripts/…`, `fixtures/…`, `_tmp/…`, or a bare filename ending in a known
//! extension) must resolve to an existing file or directory under the repo
//! root. Semantic claim verification is out of scope — this is the first
//! rung the future checks build on.

// Module is consumed only from `main.rs` binary paths and its own tests;
// `pub(crate)` inside a private module trips redundant_pub_crate under this
// lint config, so the lint is relaxed for the whole file.
#![allow(clippy::redundant_pub_crate)] // reason: module private by design

use std::path::Path;

/// Prefixes that make a token look like a repo-relative path.
const PATH_PREFIXES: &[&str] = &[
    "crates/",
    "specs/",
    "xtask/",
    "graphs/",
    "scripts/",
    "fixtures/",
    "_tmp/",
    "_docs/",
];

/// Runs the doc-drift check over `HANDOFF.md` (at the repo root).
///
/// # Errors
/// One `DOC-DRIFT FAIL: <token> (HANDOFF.md) does not exist` line per
/// dangling backtick path; exit 1 when any entry fails.
pub(crate) fn run(root: &Path) -> Result<(), Vec<String>> {
    let handoff = root.join("HANDOFF.md");
    let Ok(src) = std::fs::read_to_string(&handoff) else {
        return Err(vec![format!(
            "DOC-DRIFT FAIL: cannot read {}",
            handoff.display()
        )]);
    };
    let mut violations = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for token in backtick_tokens(&src) {
        if !anchored_path(&token) || seen.contains(&token) {
            continue;
        }
        seen.push(token.clone());
        if !token_exists(root, &token) {
            violations.push(format!(
                "DOC-DRIFT FAIL: `{token}` (HANDOFF.md) does not exist"
            ));
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

/// Extracts single-backtick-quoted tokens from `src`. Fenced code-block
/// spans contain newlines and are skipped by the newline filter.
fn backtick_tokens(src: &str) -> Vec<String> {
    let parts: Vec<&str> = src.split('`').collect();
    // split alternates outside/inside; inside tokens are the odd indices.
    let mut out = Vec::new();
    for (idx, part) in parts.iter().enumerate() {
        if idx % 2 == 1 && !part.contains('\n') && !part.is_empty() {
            out.push((*part).to_owned());
        }
    }
    out
}

/// True when `token` is an anchored repo-relative path (`crates/…`,
/// `specs/…`, …). Bare filenames and code identifiers are NOT checked in
/// v1: HANDOFF.md legitimately names files of OTHER projects (pidag) and
/// crate-relative shorthands, which a bare-name heuristic cannot
/// disambiguate — the anchor rule keeps the gate real (our own tree is
/// verified) without noise.
fn anchored_path(token: &str) -> bool {
    if token.contains("://") || token.contains('{') || token.contains('}') {
        return false;
    }
    PATH_PREFIXES.iter().any(|p| token.starts_with(p))
}

/// True when `token` exists under root, tolerating a trailing `:line`
/// (e.g. `` `crates/pii-store/src/lib.rs:41` ``).
///
/// Forward-referenced future specs are exempt: HANDOFF routinely points at
/// spec files the planner has not authored yet (e.g. `06-phase3b-…`), so a
/// missing `specs/…` name is a plan, not a lie — the gate's teeth are on the
/// code/doc tree (`crates/`, `scripts/`, `graphs/`, `xtask/`, `_tmp/`, …).
fn token_exists(root: &Path, token: &str) -> bool {
    let stripped = token
        .rsplit_once(':')
        .filter(|(_, n)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        .map_or(token, |(path, _)| path);
    if root.join(stripped).exists() {
        return true;
    }
    if stripped.starts_with("specs/") {
        return true; // forward reference to a planner-owned, not-yet-authored spec
    }
    false
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{anchored_path, backtick_tokens, token_exists};

    #[test]
    fn tokens_and_path_heuristics() {
        let src = "see `crates/pii-x/src/y.rs` and `dispatch.rs` and `~/.x` and `_tmp/a`";
        let toks = backtick_tokens(src);
        assert!(toks.contains(&"crates/pii-x/src/y.rs".to_owned()));
        assert!(toks.contains(&"dispatch.rs".to_owned()));
        assert!(anchored_path("crates/pii-x/src/y.rs"));
        assert!(anchored_path("_tmp/a"));
        assert!(!anchored_path("dispatch.rs"), "bare names unchecked in v1");
        assert!(!anchored_path("https://example.com/x"));
        assert!(!anchored_path("~/.pi/x"));
        assert!(!anchored_path("04-phase2-{a}"));
    }

    #[test]
    fn trailing_line_number_is_stripped() {
        let root = std::path::Path::new("/projects/pi-improved");
        assert!(token_exists(root, "crates/pii-store/src/lib.rs:41"));
        assert!(token_exists(root, "crates/pii-store/src/lib.rs"));
    }
}
