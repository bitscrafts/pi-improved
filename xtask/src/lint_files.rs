//! 450-line file-size gate over effective (non-blank, non-comment) lines.

// reason: see deps.rs — `pub(super)` is the correct visibility for these
// binary-internal helpers; clippy `redundant_pub_crate` conflicts with rustc
// `unreachable_pub` (deny), allowed narrowly per spec F16.
#![allow(clippy::redundant_pub_crate)]

use std::fs;
use std::path::{Path, PathBuf};

const MAX_EFFECTIVE_LINES: usize = 450;
const WARN_EFFECTIVE_LINES: usize = 380;
const ROOTS: &[&str] = &["crates", "xtask"];

/// Counts lines that are non-empty after trimming and do not start with
/// `//`, **excluding `#[cfg(test)]` module bodies** (F15, opus review: tests
/// do not count against the budget — otherwise the gate punishes testing).
#[must_use]
pub(super) fn count_effective_lines(src: &str) -> usize {
    let mut count = 0usize;
    let mut in_cfg_test = false;
    let mut cfg_test_depth: isize = 0;
    for line in src.lines() {
        let trimmed = line.trim();
        // Detect a `#[cfg(test)]` attribute line.
        if trimmed.starts_with("#[cfg(test)]") {
            in_cfg_test = true;
            continue;
        }
        // Whole-file test module (F15): an inner `#![cfg(test)]` attribute
        // means the entire remaining file is test-only code and counts as
        // zero effective lines.
        if trimmed == "#![cfg(test)]" {
            return 0;
        }
        // Track brace depth to know when the cfg(test) module body ends.
        let opens = trimmed.matches('{').count().cast_signed();
        let closes = trimmed.matches('}').count().cast_signed();
        if in_cfg_test {
            cfg_test_depth += opens - closes;
            if cfg_test_depth <= 0 {
                in_cfg_test = false;
                cfg_test_depth = 0;
            }
            continue; // every line inside a cfg(test) body is excluded
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        count += 1;
    }
    count
}

/// Pure check over pre-counted files; fails on any file over the gate.
fn check_counts(files: &[(String, usize)]) -> Result<(), Vec<String>> {
    let mut violations = Vec::new();
    for (path, count) in files {
        if *count > MAX_EFFECTIVE_LINES {
            violations.push(format!(
                "{path}: {count} effective lines exceeds {MAX_EFFECTIVE_LINES}"
            ));
        } else if *count > WARN_EFFECTIVE_LINES {
            println!("warning: {path}: {count} effective lines (warn at {WARN_EFFECTIVE_LINES})");
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

/// Walks `crates/*/src/**/*.rs` and `xtask/src/**/*.rs`, skipping
/// `target/`, and fails on any file over the gate.
///
/// # Errors
/// Returns one message per violating file, or an I/O error line.
pub(super) fn run() -> Result<(), Vec<String>> {
    let mut files: Vec<(String, usize)> = Vec::new();
    for root in ROOTS {
        let mut paths = Vec::new();
        collect_rs_files(Path::new(root), &mut paths);
        for path in paths {
            let src =
                fs::read_to_string(&path).map_err(|e| vec![format!("{}: {e}", path.display())])?;
            files.push((path.display().to_string(), count_effective_lines(&src)));
        }
    }
    check_counts(&files)
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name == "target" || name == "tests" {
                continue;
            }
            collect_rs_files(&path, out);
        } else if name.ends_with(".rs") {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{check_counts, count_effective_lines};

    fn code_lines(n: usize) -> String {
        (0..n)
            .map(|i| format!("pub fn generated_{i}() {{}}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn lint_files_accepts_450_lines() {
        // 450 code + 60 blank + 40 comment lines == 450 effective.
        let mut src = code_lines(450);
        src.push('\n');
        src.push_str(&"\n".repeat(60));
        src.push_str(&"// comment\n".repeat(40));
        assert_eq!(count_effective_lines(&src), 450);
        let counted = vec![(
            String::from("crates/x/src/f.rs"),
            count_effective_lines(&src),
        )];
        assert!(check_counts(&counted).is_ok());
    }

    #[test]
    fn lint_files_rejects_451_lines() {
        let src = code_lines(451);
        assert_eq!(count_effective_lines(&src), 451);
        let counted = vec![(String::from("crates/x/src/big.rs"), 451)];
        let errs = check_counts(&counted).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("big.rs") && e.contains("451"))
        );
    }

    #[test]
    fn lint_files_excludes_cfg_test_bodies() {
        // 10 code lines + a cfg(test) module with 200 lines of tests.
        let mut src = code_lines(10);
        src.push_str("\n#[cfg(test)]\nmod tests {\n");
        src.push_str(&code_lines(200));
        src.push_str("\n}\n");
        assert_eq!(count_effective_lines(&src), 10);
    }
}
