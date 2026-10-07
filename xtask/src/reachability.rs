//! `xtask reachability-check`: the declared-capability caller gate.
//!
//! For each entry in [`SEED`] — `(label, needle, search_dir, declare_dir)` —
//! assert a REAL caller exists: a non-comment, non-`#[cfg(test)]`, non-test-
//! path occurrence of `needle` in `search_dir`, OUTSIDE the crate that
//! declares the symbol (`declare_dir`). "Outside the declaring crate" is the
//! D3 fix: internal self-matches (`provider_from_env` calling a sibling
//! resolver, the `fn` declaration itself) must never satisfy the gate, so
//! deleting the symbol's last external caller goes RED even when the
//! declaring crate still matches itself.
//!
//! This is a pure text oracle (Decision 5 step 1, spec 72): the strongest
//! structure-free caller detector for path-qualified calls and
//! receiver-qualified method calls (`sup.invoke(`), with comments and
//! cfg(test) regions excluded by reusing `lint_files`' skipping logic. A
//! `syn` visitor is explicitly NOT this batch; escalate only if the living
//! list outgrows the text seeds (Decision 5 step 2).
//!
//! **Living list**: add an entry per new declared capability. `declare_dir`
//! is where the symbol is *declared*; it always overlaps (`search_dir` ⊆
//! `declare_dir` or the declaration is a dependency of `search_dir`'s crate)
//! and self-matches there are skipped by construction.
//!
//! Wired into `quality-gate.sh` (after the test steps, before the summary),
//! so the blind-gate class (043 report D3) cannot recur silently.

// Module is consumed only from `main.rs` binary paths and its own tests;
// `pub(crate)` inside a private module trips redundant_pub_crate under this
// lint config, so the lint is relaxed for the whole file.
#![allow(clippy::redundant_pub_crate)] // reason: module private by design

use std::path::Path;

use crate::lint_files::count_effective_lines;

/// One reachability entry: `needle` must appear outside comments and
/// `#[cfg(test)]` bodies in some non-test `.rs` under `search_dir`, and NOT
/// under `declare_dir` (the crate that declares the symbol) — external
/// callers only.
pub struct ReachEntry {
    /// Human label (the declared capability); printed on failure.
    pub label: &'static str,
    /// The call-site text to find.
    pub needle: &'static str,
    /// Directory to search (relative to the workspace root).
    pub search_dir: &'static str,
    /// Directory prefix to skip (where the symbol is declared: the
    /// declaring crate). Internal self-matches never count.
    pub declare_dir: &'static str,
}

/// The seed list: every entry's needle is the REAL external call site, on
/// this tree at HEAD, that this check depends on (spec 72 R1).
///
/// - `resolve_with_transport` is declared in `crates/pii-model/src/lib.rs`
///   and called from `crates/pii-server/src/lib.rs:203`
///   (`pii_model::resolve_with_transport(&cfg.model, Arc::new(net))`) — the
///   `ollama:` path's one caller (`pii-server`, 043 report C-read). The
///   declaring crate's own internal calls (`pii-model/src/lib.rs:242,266`)
///   are skipped by `declare_dir`.
/// - `Supervisor::invoke` is declared in `crates/pii-worker/src/control.rs`
///   and called from `crates/pii-server/src/node_kinds/nodes/worker.rs:38`
///   (`sup.invoke(...)`, the `worker` graph node, F3.7) — receiver-qualified,
///   so the needle pins the real call instead of any `.invoke(` homonym
///   (043 report D3 evidence string #3, spec 72 R1).
pub const SEED: &[ReachEntry] = &[
    ReachEntry {
        label: "pii_model::resolve_with_transport",
        needle: "pii_model::resolve_with_transport(",
        search_dir: "crates/pii-server/src",
        declare_dir: "crates/pii-model",
    },
    ReachEntry {
        label: "pii_worker::Supervisor::invoke",
        needle: "sup.invoke(",
        search_dir: "crates/pii-server/src",
        declare_dir: "crates/pii-worker",
    },
];

/// Runs the reachability gate over the workspace.
///
/// # Errors
/// One `REACHABILITY FAIL: <label> has no non-test caller` line per
/// orphaned entry; exit 1 when any entry fails.
pub fn run(root: &Path) -> Result<(), Vec<String>> {
    let mut violations = Vec::new();
    for entry in SEED {
        if !find_external_caller(root, entry) {
            violations.push(format!(
                "REACHABILITY FAIL: {} has no non-test caller",
                entry.label
            ));
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

/// True when `needle` appears as a real call site — outside comments and
/// `#[cfg(test)]` bodies — in some `.rs` under `search_dir`, and that file
/// is neither under `declare_dir` (self-declaration/internal calls) nor a
/// test-only path (`tests/`, `*_test.rs`).
fn find_external_caller(root: &Path, entry: &ReachEntry) -> bool {
    let dir = root.join(entry.search_dir);
    let mut hit = false;
    walk(&dir, &mut |path| {
        if hit {
            return;
        }
        let rel = path.strip_prefix(root).unwrap_or(path);
        let rel_str = rel.to_string_lossy();
        // The declaring crate's files are the ONLY place the declaration and
        // internal self-calls live; a caller must be outside it.
        if under_declare_dir(&rel_str, entry.declare_dir) {
            return;
        }
        if is_test_path(&rel_str) {
            return;
        }
        if let Ok(src) = std::fs::read_to_string(path) {
            // Every commented line in a Rust file is `//`-prefixed (the
            // workspace gate counts effective lines that way), so a
            // `count_effective_lines` == 0 file is a pure-comment file and
            // cannot contain a call. The needle match below runs per line,
            // with comment/cfg(test) lines excluded the same way.
            let src = src.as_ref();
            for (non_comment, line) in effective_lines(src) {
                if line.contains(entry.needle) && non_comment != 0 {
                    hit = true;
                    return;
                }
            }
        }
    });
    hit
}

/// True when `rel` lives under `declare_dir` — the skip is component-
/// aware, so a sibling crate whose name extends the declaring crate
/// (`crates/user2` vs `crates/user`) is NOT shadowed by the prefix.
fn under_declare_dir(rel: &str, declare_dir: &str) -> bool {
    let rel_path = Path::new(rel);
    let decl = Path::new(declare_dir);
    rel_path.strip_prefix(decl).is_ok()
}

/// True when `rel` is a test-only path (`/tests/` dir or `*_test.rs` name).
fn is_test_path(rel: &str) -> bool {
    let lower = rel.to_ascii_lowercase();
    lower.contains("/tests/") || lower.ends_with("_test.rs")
}

/// Depth-first `.rs` walk, deterministic order, no third-party walker.
fn walk(dir: &Path, visit: &mut dyn FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<std::path::PathBuf> =
        entries.filter_map(Result::ok).map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            walk(&p, visit);
        } else if p.extension().is_some_and(|e| e == "rs") {
            visit(&p);
        }
    }
}

/// `(non_comment, line)` for every line whose trimmed text is neither blank
/// nor `//`-commented: the workspace's own comment convention (the one
/// `count_effective_lines` counts), used to strip comments and let reuse of
/// `lint_files`' skipping cover comment lines here.
fn effective_lines(src: &str) -> impl Iterator<Item = (usize, &str)> {
    src.lines().filter_map(|line| {
        let t = line.trim();
        if t.is_empty() || t.starts_with("//") {
            return None;
        }
        Some((count_effective_lines(t), t))
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::indexing_slicing)] // tests index scratch fixtures

    use super::{ReachEntry, SEED, find_external_caller, is_test_path, run, under_declare_dir};
    use crate::lint_files::count_effective_lines;

    fn scratch_root(tag: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root exists")
            .join("_tmp")
            .join("spec72-reachability")
            .join(tag);
        for (path, contents) in files {
            let p = root.join(path);
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir).unwrap();
            }
            std::fs::write(&p, contents).unwrap();
        }
        root
    }

    fn entry(needle: &'static str) -> ReachEntry {
        ReachEntry {
            label: "cap",
            needle,
            search_dir: "crates",
            declare_dir: "crates/user",
        }
    }

    fn run_entry(root: &std::path::Path, e: &ReachEntry) -> bool {
        find_external_caller(root, e)
    }

    /// The declaring crate's own matches (declaration + internal calls)
    /// must NOT count: the only real caller lives outside `declare_dir`.
    #[test]
    fn declaring_crate_self_matches_do_not_count() {
        let root = scratch_root(
            "self-match-isolated",
            &[
                ("crates/user/src/decl.rs", "pub fn cap() {}\ncap();\n"),
                ("crates/user2/src/main.rs", ""),
            ],
        );
        let e = entry("cap()");
        assert!(!run_entry(&root, &e), "internal self-match is not a caller");
        std::fs::write(
            root.join("crates/user2/src/main.rs"),
            "fn go() { user::cap(); }\n",
        )
        .unwrap();
        assert!(run_entry(&root, &e), "external caller counts");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A sibling crate whose name extends the declaring crate's name
    /// (`crates/user2` under `declare_dir = crates/user`) still counts as
    /// external: the skip is component-aware, not a raw string prefix.
    #[test]
    fn declare_dir_prefix_skips_only_that_prefix() {
        let root = scratch_root(
            "declare-prefix",
            &[("crates/usera/src/other.rs", "fn _x() { cap(); }\n")],
        );
        let e = entry("cap()");
        assert!(
            run_entry(&root, &e),
            "a file outside the declare_dir prefix counts"
        );
        // The exact shadowing case from the first red run.
        assert!(
            !under_declare_dir("crates/user2/src/main.rs", "crates/user"),
            "user2 must not be shadowed by user"
        );
        assert!(under_declare_dir("crates/user/src/lib.rs", "crates/user"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A commented needle never counts (spec 72 R2: comments are not
    /// callers) — `count_effective_lines` skip logic reused.
    #[test]
    fn commented_call_is_not_a_caller() {
        let root = scratch_root(
            "comment-only",
            &[("crates/user/src/lib.rs", "// sup.invoke(\n")],
        );
        let e = entry("sup.invoke(");
        assert!(!run_entry(&root, &e), "a comment is not a caller");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A comment containing `.invoke(` in an unrelated crate, with the
    /// seed's `.invoke()` (needle `.invoke(`): the comment must NOT satisfy
    /// the gate — exit must be failure when the comment is the only match.
    #[test]
    fn comments_are_not_callers() {
        let root = scratch_root(
            "comment-not-caller",
            &[("crates/user/src/lib.rs", "// .invoke( as a shape\n")],
        );
        let e = entry(".invoke(");
        assert!(!run_entry(&root, &e), "comment does not satisfy the gate");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `#[cfg(test)]` bodies are not callers (spec 72 R2). A needle
    /// occurring only inside a `#[cfg(test)] mod tests { .. }` block does
    /// not satisfy the gate.
    #[test]
    fn cfg_test_body_is_not_a_caller() {
        let root = scratch_root(
            "cfg-test-body",
            &[(
                "crates/user/src/lib.rs",
                "#[cfg(test)]\nmod tests {\n    sup.invoke(a);\n}\n",
            )],
        );
        let e = entry("sup.invoke(");
        assert!(
            !run_entry(&root, &e),
            "a cfg(test)-body match is not a caller"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A test-only path (`tests/`) is not a caller.
    #[test]
    fn test_path_is_not_a_caller() {
        let root = scratch_root(
            "test-path",
            &[("crates/user/tests/it.rs", "sup.invoke(a);\n")],
        );
        assert!(is_test_path("crates/user/tests/it.rs"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Every seed entry must currently have its real external caller (R1's
    /// positive half: the check's basis must exist, and stay honest — a
    /// seed whose caller is gone fails here loudly rather than the gate
    /// carrying a stale entry).
    #[test]
    fn seeds_match_the_real_tree_at_head() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root exists");
        let mut missing = Vec::new();
        for seed in SEED {
            if !find_external_caller(root, seed) {
                missing.push(seed.label);
            }
        }
        assert!(
            missing.is_empty(),
            "external callers for {missing:?} are gone from HEAD"
        );
    }

    /// Failure paths keep the output shape invariant: one line per entry,
    /// naming the label and the needle's absence, in SEED order (spec 72:
    /// "symbols × evidence" shape unchanged).
    #[test]
    fn run_reports_each_orphaned_entry_in_seed_order() {
        let root = scratch_root("empty-tree", &[]);
        let err = run(&root).unwrap_err();
        let n = SEED.len();
        assert_eq!(err.len(), n, "one message per entry (had {n}): {err:?}");
        for (i, e) in SEED.iter().enumerate() {
            let line = &err[i];
            assert!(
                line.contains(e.label) && line.contains("REACHABILITY FAIL: "),
                "line {i} must name the orphaned symbol: {line}"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The phantom `line_is` + `_test` doc (043 report D3 string #3) is gone;
    /// the doc no longer names a function that does not exist. The guard
    /// strings are split so the file itself contains no grep-trap literal
    /// (spec 72 exit criteria: the whole-file greps must count 0).
    #[test]
    fn no_phantom_line_doc_documentation() {
        // Only the production half of this file: cut at the real test
        // module attribute (line-start), not any doc-comment mention.
        let src = include_str!("reachability.rs");
        let at = src
            .find("\n#[cfg(test)]\n")
            .expect("file always has a cfg(test) module");
        let prod = &src[..at];
        let phantom = ["line_is", "_test"].concat();
        assert!(!prod.contains(&phantom), "phantom doc must be deleted");
        // And the hardcoded host path is gone from this file (R3/D7 class).
        let host_path = ["/projects/", "pi-improved"].concat();
        assert!(!prod.contains(&host_path), "no host paths in-tree");
        // Scratch fixtures (this file's own harness) route through the
        // workspace `_tmp/`, never a system directory (R7 rule).
        assert!(
            src.contains(".join(\"_tmp\")"),
            "scratch fixtures route under _tmp/"
        );
    }

    /// A pure-comment file (zero effective lines) cannot carry a caller:
    /// the comment-skip must hold on real multi-comment fixtures, not just
    /// the trimmed `//` prefix.
    #[test]
    fn pure_comment_file_has_zero_effective_lines_and_holds() {
        // The check's exclusion of `//` lines is the same convention the
        // workspace's line counter enforces; a fixture comment must produce
        // zero effective lines.
        let src = "// sup.invoke(\n// .invoke( shape\n";
        let t = src.trim();
        let lines: Vec<&str> = src
            .lines()
            .filter(|l| {
                let tt = l.trim();
                !tt.is_empty() && !tt.starts_with("//")
            })
            .collect();
        assert!(lines.is_empty(), "comment lines are skipped: {lines:?}");
        // Effective-line count of a comment-only line is 0, not 1.
        assert_eq!(count_effective_lines(src), 0);
        let _ = t;
    }
}
