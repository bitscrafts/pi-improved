//! Spec 96: static check that fails on a bare `_tmp` scratch site.
//!
//! A bare site is a `tempdir_in(` call whose argument names a `_tmp`
//! directory (or whose enclosing function mentions one) where nothing earlier
//! in that function calls `create_dir_all` or `tmp_scratch`. Such a test
//! passes in a developer tree (the directory is left over from an earlier
//! run) and panics `NotFound` in a fresh clone.

// reason: see lint_files.rs — binary-internal helpers; clippy
// `redundant_pub_crate` conflicts with rustc `unreachable_pub`.
#![allow(clippy::redundant_pub_crate)]

use std::fs;
use std::path::{Path, PathBuf};

const ROOTS: &[&str] = &["crates", "xtask"];
const CALL: &str = "tempdir_in(";

/// Returns `path:line` messages for every bare site in `src`.
fn check_source(path: &str, src: &str) -> Vec<String> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some((_, rest)) = line.split_once(CALL) else {
            continue;
        };
        // The argument may continue on later lines until its closing paren.
        let mut arg = String::from(rest);
        for next in lines.iter().skip(i + 1) {
            if arg.contains(')') {
                break;
            }
            arg.push_str(next);
        }
        // Everything from the enclosing `fn` line up to (not including) the call.
        let mut before: Vec<&str> = lines
            .iter()
            .take(i)
            .rev()
            .scan(false, |done, l| {
                if *done {
                    return None;
                }
                *done = l.contains("fn ");
                Some(*l)
            })
            .collect();
        before.reverse();
        let before = before.join("\n");
        let guarded = before.contains("create_dir_all") || before.contains("tmp_scratch");
        let names_tmp = names_tmp_dir(&arg) || names_tmp_dir(&before);
        if names_tmp && !guarded {
            out.push(format!(
                "{path}:{}: tempdir_in on a `_tmp` directory with no create_dir_all/tmp_scratch before it in the same fn",
                i + 1
            ));
        }
    }
    out
}

/// True when `text` holds a `_tmp` directory literal (`"_tmp"`, `"/_tmp"`,
/// `"_tmp/..."`), not merely an identifier such as `mcp_tmp_root`.
fn names_tmp_dir(text: &str) -> bool {
    text.contains("/_tmp") || text.contains("\"_tmp")
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name != "target" && name != "_tmp" {
                collect(&path, out);
            }
        } else if name.ends_with(".rs") && name != "lint_scratch.rs" {
            out.push(path);
        }
    }
}

/// Scans `crates/` and `xtask/` for bare `_tmp` scratch sites.
///
/// # Errors
/// One message per bare site, or per unreadable file.
pub fn run() -> Result<(), Vec<String>> {
    let mut files = Vec::new();
    for root in ROOTS {
        collect(Path::new(root), &mut files);
    }
    let mut violations = Vec::new();
    for path in &files {
        match fs::read_to_string(path) {
            Ok(src) => violations.extend(check_source(&path.display().to_string(), &src)),
            Err(e) => violations.push(format!("{}: {e}", path.display())),
        }
    }
    if violations.is_empty() {
        println!("lint-scratch: {} files checked, 0 bare sites", files.len());
        Ok(())
    } else {
        Err(violations)
    }
}

#[cfg(test)]
mod tests {
    use super::check_source;

    const BARE: &str = "fn t() {\n    let d = tempfile::Builder::new()\n        .prefix(\"x\")\n        .tempdir_in(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/_tmp\"))\n        .unwrap();\n}\n";

    #[test]
    fn flags_bare_site() {
        assert_eq!(check_source("a.rs", BARE).len(), 1);
    }

    #[test]
    fn accepts_create_dir_all_before() {
        let s = "fn t() {\n    let root = std::path::Path::new(\"/x\").join(\"_tmp\");\n    std::fs::create_dir_all(&root).unwrap();\n    let d = b.tempdir_in(&root).unwrap();\n}\n";
        assert!(check_source("a.rs", s).is_empty());
    }

    #[test]
    fn accepts_tmp_scratch_and_unrelated() {
        let s = "fn t() {\n    let d = pii_store::testing::tmp_scratch(m, \"x\");\n}\nfn u() {\n    b.tempdir_in(\"/tmp\").unwrap();\n}\n";
        assert!(check_source("a.rs", s).is_empty());
    }

    #[test]
    fn guard_in_a_different_fn_does_not_count() {
        let s = "fn a() {\n    std::fs::create_dir_all(\"_tmp\").unwrap();\n}\nfn b() {\n    x.tempdir_in(\"/p/_tmp\").unwrap();\n}\n";
        assert_eq!(check_source("a.rs", s).len(), 1);
    }
}
