//! 450-line file-size gate over effective (non-blank, non-comment) lines.

use std::fs;
use std::path::{Path, PathBuf};

const MAX_EFFECTIVE_LINES: usize = 450;
const WARN_EFFECTIVE_LINES: usize = 380;
const ROOTS: &[&str] = &["crates", "xtask"];

/// Counts lines that are non-empty after trimming and do not start
/// with `//`.
#[must_use]
pub(crate) fn count_effective_lines(src: &str) -> usize {
    src.lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with("//")
        })
        .count()
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
pub(crate) fn run() -> Result<(), Vec<String>> {
    let mut files: Vec<(String, usize)> = Vec::new();
    for root in ROOTS {
        let mut paths = Vec::new();
        collect_rs_files(Path::new(root), &mut paths);
        for path in paths {
            let src = fs::read_to_string(&path)
                .map_err(|e| vec![format!("{}: {e}", path.display())])?;
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
            if name == "target" {
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
        let counted = vec![(String::from("crates/x/src/f.rs"), count_effective_lines(&src))];
        assert!(check_counts(&counted).is_ok());
    }

    #[test]
    fn lint_files_rejects_451_lines() {
        let src = code_lines(451);
        assert_eq!(count_effective_lines(&src), 451);
        let counted = vec![(String::from("crates/x/src/big.rs"), 451)];
        let errs = check_counts(&counted).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("big.rs") && e.contains("451")));
    }
}