//! `xtask doc-drift-check` (Slice -1, S(-1).2): catch ONE concrete class of
//! doc/code mismatch mechanically — a path mentioned in `HANDOFF.md` inside
//! backticks that does not exist on disk.
//!
//! Scope is deliberately narrow (v1): every backtick-quoted token that looks
//! like a repo-relative path (`crates/…`, `specs/…`, `xtask/…`, `graphs/…`,
//! `scripts/…`, `fixtures/…`, or a bare filename ending in a known
//! extension) must resolve to an existing file or directory under the repo
//! root. Semantic claim verification is out of scope — this is the first
//! rung the future checks build on.

// Module is consumed only from `main.rs` binary paths and its own tests;
// `pub(crate)` inside a private module trips redundant_pub_crate under this
// lint config, so the lint is relaxed for the whole file.
#![allow(clippy::redundant_pub_crate)] // reason: module private by design

use std::path::{Path, PathBuf};

/// Prefixes that make a token look like a repo-relative path.
const PATH_PREFIXES: &[&str] = &[
    "crates/",
    "specs/",
    "xtask/",
    "graphs/",
    "scripts/",
    "fixtures/",
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
    // Spec 118: `_tmp/` is gitignored scratch (logs, exports) that HANDOFF
    // cites as history, so its absence is not drift; globs, ellipses and
    // `<N>`/argument forms are patterns, not paths.
    if token.contains("://")
        || token.contains(['{', '}', '*', '…', '<', ' '])
        || token.contains("...")
        || token.starts_with("_tmp/")
    {
        return false;
    }
    // An `_tmp` component anywhere in the path is the same scratch rule: a
    // test that writes crate-local `crates/<x>/_tmp/**` creates it on demand
    // (spec 96's tmp_scratch contract), so a history paragraph citing one is
    // not drift. Measured 2026-10-20: `crates/pii-memory/_tmp/phase3` and
    // `crates/pii-server/_tmp` in HANDOFF history failed a green tree.
    if token.split('/').any(|comp| comp == "_tmp") {
        return false;
    }
    PATH_PREFIXES.iter().any(|p| token.starts_with(p))
}

/// True when `token` exists under root, tolerating a trailing `:line`
/// (e.g. `` `crates/pii-store/src/lib.rs:41` `` or a `:5-9` range).
///
/// Forward-referenced future specs are exempt: HANDOFF routinely points at
/// spec files the planner has not authored yet (e.g. `06-phase3b-…`), so a
/// missing `specs/…` name is a plan, not a lie — the gate's teeth are on the
/// code/doc tree (`crates/`, `scripts/`, `graphs/`, `xtask/`, `_tmp/`, …).
fn token_exists(root: &Path, token: &str) -> bool {
    let stripped = token
        .rsplit_once(':')
        .filter(|(_, n)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit() || c == '-'))
        .map_or(token, |(path, _)| path);
    if root.join(stripped).exists() {
        return true;
    }
    if stripped.starts_with("specs/") {
        return true; // forward reference to a planner-owned, not-yet-authored spec
    }
    false
}

/// Spec 374 R3: the module headers (`//!`) of every Rust source in the tree must name only paths that exist and
/// functions that are defined. Path tokens reuse the HANDOFF rules above; a backtick token `name()` must be a
/// function the tree defines. Other symbols are not checked (a bare word is too often prose or a std item).
///
/// # Errors
/// One `DOC-DRIFT FAIL` line per dangling path or undefined function, naming the module that makes the claim.
pub(crate) fn run_modules(root: &Path) -> Result<(), Vec<String>> {
    let sources = rust_sources(root);
    let texts: Vec<String> = sources
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    let mut violations = Vec::new();
    for (path, src) in sources.iter().zip(&texts) {
        let Some(doc) = module_doc(src) else { continue };
        let module = relative(root, path);
        for token in backtick_tokens(&doc) {
            if anchored_path(&token) {
                if !token_exists(root, &token) {
                    violations.push(format!(
                        "DOC-DRIFT FAIL: `{token}` ({module}) does not exist"
                    ));
                }
            } else if let Some(item) = token.strip_suffix("()") {
                let name = item.rsplit("::").next().unwrap_or(item);
                if is_ident(name) && !defines_fn(&texts, name) {
                    violations.push(format!(
                        "DOC-DRIFT FAIL: `{token}` ({module}) names no function in the tree"
                    ));
                }
            }
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

/// Spec 374 R1: the Rust modules whose source has no `//!` header, one repo-relative path per line. An empty list
/// means every module is documented, which is the `nothing-to-do` control of `graphs/doc-code.toml`.
pub(crate) fn inventory(root: &Path) -> Vec<String> {
    rust_sources(root)
        .iter()
        .filter(|path| !std::fs::read_to_string(path).is_ok_and(|src| module_doc(&src).is_some()))
        .map(|path| relative(root, path))
        .collect()
}

/// Every Rust source under `xtask/src` and `crates/*/src`, sorted. The one tree walk behind both checks.
fn rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![root.join("xtask/src")];
    if let Ok(entries) = std::fs::read_dir(root.join("crates")) {
        dirs.extend(entries.flatten().map(|e| e.path().join("src")));
    }
    let mut out = Vec::new();
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// The module's `//!` header: the inner doc lines before the first code line, or `None` when there are none.
fn module_doc(src: &str) -> Option<String> {
    let lines: Vec<&str> = src
        .lines()
        .take_while(|l| {
            let t = l.trim_start();
            t.is_empty() || t.starts_with("//")
        })
        .filter_map(|l| l.trim_start().strip_prefix("//!"))
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn is_ident(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// True when some source defines `fn name` (followed by its parameters or generics).
fn defines_fn(texts: &[String], name: &str) -> bool {
    let decl = format!("fn {name}");
    texts.iter().any(|src| {
        src.match_indices(&decl).any(|(at, _)| {
            let after = src[at + decl.len()..].chars().next();
            matches!(after, Some('(' | '<'))
        })
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{anchored_path, backtick_tokens, defines_fn, module_doc, token_exists};

    /// Spec 374 R3: a module header that names a function nobody defines is refused, one that names a real one passes.
    #[test]
    fn symbols_in_a_header_must_be_defined() {
        let texts = vec!["pub fn real_one(x: u8) {}\n".to_owned()];
        assert!(defines_fn(&texts, "real_one"));
        assert!(!defines_fn(&texts, "real_two"), "an undefined name");
        assert!(!defines_fn(&texts, "real"), "a prefix is not the name");
    }

    #[test]
    fn module_header_is_the_leading_inner_doc() {
        assert_eq!(
            module_doc("//! one\n//! two\n\nuse x;\n//! later\n").as_deref(),
            Some(" one\n two")
        );
        assert!(module_doc("use x;\n").is_none(), "no header");
    }

    /// Spec 118 R2: scratch, placeholders and patterns are not durable targets.
    #[test]
    fn scratch_and_placeholders_are_not_checked() {
        assert!(!anchored_path("_tmp/sidecar.log"), "scratch is not durable");
        assert!(!anchored_path("_tmp/_trash/"));
        assert!(!anchored_path("_docs/06-research/032-*.md"), "glob");
        assert!(!anchored_path("_docs/06-research/046-…md"), "ellipsis");
        assert!(!anchored_path("xtask/..."), "ellipsis");
        assert!(!anchored_path("scripts/run.sh <N>"), "placeholder/args");
        assert!(
            anchored_path("crates/pii-x/src/y.rs"),
            "real paths still checked"
        );
        // An `_tmp` component anywhere is the scratch rule, not only a
        // leading one (the two crate-cited scratch paths in HANDOFF history).
        assert!(!anchored_path("crates/pii-memory/_tmp/phase3"));
        assert!(!anchored_path("crates/pii-server/_tmp"));
        assert!(anchored_path("_docs/02-guides/x.md"));
    }

    #[test]
    fn trailing_line_range_is_stripped() {
        let root_buf = crate::workspace_root();
        assert!(token_exists(root_buf.as_path(), "xtask/src/lib.rs:5-9"));
        assert!(!token_exists(
            root_buf.as_path(),
            "xtask/src/nope_zz.rs:5-9"
        ));
    }

    #[test]
    fn tokens_and_path_heuristics() {
        let src = "see `crates/pii-x/src/y.rs` and `dispatch.rs` and `~/.x` and `_tmp/a`";
        let toks = backtick_tokens(src);
        assert!(toks.contains(&"crates/pii-x/src/y.rs".to_owned()));
        assert!(toks.contains(&"dispatch.rs".to_owned()));
        assert!(anchored_path("crates/pii-x/src/y.rs"));
        assert!(!anchored_path("_tmp/a"), "spec 118: scratch is not durable");
        assert!(!anchored_path("dispatch.rs"), "bare names unchecked in v1");
        assert!(!anchored_path("https://example.com/x"));
        assert!(!anchored_path("~/.pi/x"));
        assert!(!anchored_path("04-phase2-{a}"));
    }

    #[test]
    fn trailing_line_number_is_stripped() {
        let root_buf = crate::workspace_root();
        let root = root_buf.as_path();
        assert!(token_exists(root, "crates/pii-store/src/lib.rs:41"));
        assert!(token_exists(root, "crates/pii-store/src/lib.rs"));
    }
}
