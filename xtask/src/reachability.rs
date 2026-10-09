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
            // Lexed over the whole file: comments, strings and cfg(test)
            // items never contribute candidate lines.
            if effective_lines(&src)
                .iter()
                .any(|line| line.contains(entry.needle))
            {
                hit = true;
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

/// Candidate caller lines: every line left after the whole file is lexed
/// (comments, strings, raw strings and char literals blanked, state carried
/// across lines) and `#[cfg(test)]` items are skipped (mod, fn, or an
/// attribute stack before one). Blank lines are dropped.
fn effective_lines(src: &str) -> Vec<String> {
    let code = blank_non_code(src);
    let mut out = Vec::new();
    let mut gate = CfgTestGate::default();
    for line in code.lines() {
        if gate.skips(line.trim()) {
            continue;
        }
        let t = line.trim();
        if !t.is_empty() {
            out.push(t.to_owned());
        }
    }
    out
}

/// Tracks a `#[cfg(test)]` item across lines: attribute stack, then the
/// braced body (or a `;`-terminated item).
#[derive(Default)]
struct CfgTestGate {
    active: bool,
    depth: isize,
    opened: bool,
}

impl CfgTestGate {
    /// True when `t` (a trimmed, lexed line) belongs to a cfg(test) item.
    fn skips(&mut self, t: &str) -> bool {
        let rest = if self.active {
            t
        } else {
            let Some(after) = t.strip_prefix("#[cfg(test)]") else {
                return false;
            };
            self.active = true;
            after.trim()
        };
        if !self.opened && (rest.is_empty() || rest.starts_with("#[")) {
            return true; // attribute stack before the item body
        }
        let opens = rest.matches('{').count().cast_signed();
        let closes = rest.matches('}').count().cast_signed();
        self.depth += opens - closes;
        self.opened |= opens > 0;
        let ended = if self.opened {
            self.depth <= 0
        } else {
            rest.contains(';')
        };
        if ended {
            *self = Self::default();
        }
        true
    }
}

/// Replaces comments with nothing and the contents of string, raw string and
/// char literals with nothing (quotes kept), preserving newlines so line
/// structure survives. Block comments nest, as in Rust.
fn blank_non_code(src: &str) -> String {
    let cs: Vec<char> = src.chars().collect();
    let at = |k: usize| cs.get(k).copied();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    while let Some(c) = at(i) {
        let prev = i.checked_sub(1).and_then(at);
        let ident_prev = prev.is_some_and(|p| p.is_alphanumeric() || p == '_');
        i = match c {
            '/' if at(i + 1) == Some('/') => skip_line_comment(&cs, i),
            '/' if at(i + 1) == Some('*') => skip_block_comment(&cs, i, &mut out),
            '"' => skip_string(&cs, i, &mut out),
            'r' if (!ident_prev || prev == Some('b')) && raw_hashes(&cs, i).is_some() => {
                skip_raw_string(&cs, i, &mut out)
            }
            '\'' => skip_char_literal(&cs, i, &mut out),
            _ => {
                out.push(c);
                i + 1
            }
        };
    }
    out
}

fn skip_line_comment(cs: &[char], mut i: usize) -> usize {
    while cs.get(i).is_some_and(|&d| d != '\n') {
        i += 1;
    }
    i
}

fn skip_block_comment(cs: &[char], mut i: usize, out: &mut String) -> usize {
    let at = |k: usize| cs.get(k).copied();
    let mut depth = 0usize;
    while let Some(c) = at(i) {
        if c == '/' && at(i + 1) == Some('*') {
            depth += 1;
            i += 2;
        } else if c == '*' && at(i + 1) == Some('/') {
            depth = depth.saturating_sub(1);
            i += 2;
            if depth == 0 {
                break;
            }
        } else {
            if c == '\n' {
                out.push('\n');
            }
            i += 1;
        }
    }
    i
}

fn skip_string(cs: &[char], mut i: usize, out: &mut String) -> usize {
    out.push('"');
    i += 1;
    while let Some(&c) = cs.get(i) {
        match c {
            '\\' => {
                if cs.get(i + 1) == Some(&'\n') {
                    out.push('\n');
                }
                i += 2;
            }
            '"' => {
                out.push('"');
                return i + 1;
            }
            _ => {
                if c == '\n' {
                    out.push('\n');
                }
                i += 1;
            }
        }
    }
    i
}

/// `Some(n)` when `cs[i..]` opens a raw string `r#..#"` with `n` hashes.
fn raw_hashes(cs: &[char], i: usize) -> Option<usize> {
    let mut j = i + 1;
    while cs.get(j) == Some(&'#') {
        j += 1;
    }
    (cs.get(j) == Some(&'"')).then(|| j - i - 1)
}

fn skip_raw_string(cs: &[char], i: usize, out: &mut String) -> usize {
    let hashes = raw_hashes(cs, i).unwrap_or(0);
    out.push('"');
    let mut j = i + hashes + 2;
    while let Some(&c) = cs.get(j) {
        if c == '"' && (1..=hashes).all(|k| cs.get(j + k) == Some(&'#')) {
            out.push('"');
            return j + hashes + 1;
        }
        if c == '\n' {
            out.push('\n');
        }
        j += 1;
    }
    j
}

/// A `'` opens a char literal (`'x'`, `'\n'`); otherwise it is a lifetime.
fn skip_char_literal(cs: &[char], i: usize, out: &mut String) -> usize {
    let close = match (cs.get(i + 1), cs.get(i + 2)) {
        (Some('\\'), _) => (i + 3..i + 12).find(|&k| cs.get(k) == Some(&'\'')),
        (Some(_), Some('\'')) => Some(i + 2),
        _ => None,
    };
    if let Some(end) = close {
        out.push_str("''");
        return end + 1;
    }
    out.push('\'');
    i + 1
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
                "crates/other/src/lib.rs",
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

    /// A test-only path (`tests/`) is not a caller. The fixture sits
    /// OUTSIDE `declare_dir`, so only the `is_test_path` skip can reject it.
    #[test]
    fn test_path_is_not_a_caller() {
        let root = scratch_root(
            "test-path",
            &[
                ("crates/other/tests/it.rs", "sup.invoke(a);\n"),
                ("crates/other/src/x_test.rs", "sup.invoke(a);\n"),
            ],
        );
        assert!(is_test_path("crates/other/tests/it.rs"));
        assert!(
            !run_entry(&root, &entry("sup.invoke(")),
            "a tests/ or *_test.rs match is not a caller"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Every non-caller shape from the 045 re-review (N2) must be rejected,
    /// outside `declare_dir`; a real call after each shape must still count.
    #[test]
    fn stateful_non_caller_shapes_are_rejected() {
        let call = "sup.invoke(a);";
        let shapes = [
            format!("#[cfg(test)]\nmod t {{\n    fn f() {{ {call} }}\n}}\n"),
            format!("#[cfg(test)]\nfn f() {{ {call} }}\n"),
            format!("#[cfg(test)]\n#[allow(dead_code)]\nmod t {{\n    {call}\n}}\n"),
            format!("fn f() {{ let _ = \"{call}\"; }}\n"),
            format!("/* {call} */\n"),
            format!("/* a\n   {call}\n*/\n"),
            format!("fn f() {{ let _ = r#\"x\n{call}\n\"#; }}\n"),
            format!("fn f() {{ let _ = '\\''; }} // {call}\n"),
        ];
        for (n, body) in shapes.iter().enumerate() {
            let root = scratch_root(
                &format!("shape-{n}"),
                &[("crates/other/src/lib.rs", body.as_str())],
            );
            assert!(
                !run_entry(&root, &entry("sup.invoke(")),
                "shape {n} is not a caller: {body}"
            );
            let _ = std::fs::remove_dir_all(&root);
        }
        let root = scratch_root(
            "shape-real",
            &[(
                "crates/other/src/lib.rs",
                "#[cfg(test)]\nmod t {}\n/* c */ fn f() { let _ = \"s\"; sup.invoke(a); }\n",
            )],
        );
        assert!(run_entry(&root, &entry("sup.invoke(")), "real call counts");
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
