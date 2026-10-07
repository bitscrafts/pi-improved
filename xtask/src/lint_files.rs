//! 450-line file-size gate over effective (non-blank, non-comment) lines.

// reason: see deps.rs — `pub(super)` is the correct visibility for these
// binary-internal helpers; clippy `redundant_pub_crate` conflicts with rustc
// `unreachable_pub` (deny), allowed narrowly per spec F16. The items are
// `pub` only because the lib build (spec 72's integration tests import
// `reachability`, which reuses `count_effective_lines`) links this module
// without the bin's private-mod context; the bin marks nothing public.
#![allow(clippy::redundant_pub_crate)]

use std::fs;
use std::path::{Path, PathBuf};

const MAX_EFFECTIVE_LINES: usize = 450;
const WARN_EFFECTIVE_LINES: usize = 380;
const ROOTS: &[&str] = &["crates", "xtask"];
/// `--report` default: top-20 files by effective line count.
pub const REPORT_DEFAULT_N: usize = 20;

/// One entry for report mode: path, effective count, raw `wc -l` count.
type CountedFile = (String, usize, usize);

/// Counts lines that are non-empty after trimming and do not start with
/// `//`, **excluding `#[cfg(test)]` module bodies** (F15, opus review: tests
/// do not count against the budget — otherwise the gate punishes testing).
#[must_use]
pub fn count_effective_lines(src: &str) -> usize {
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
            // Attribute stack before the item body (F15 fix): with brace
            // depth still 0, further `#[...]` lines belong to the same
            // cfg(test)-gated item (`#[cfg(test)]` + `#[rustfmt::skip]` +
            // `mod tests { ... }`) and must NOT reset the tracking state.
            if cfg_test_depth == 0 && trimmed.starts_with("#[") {
                continue;
            }
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
/// `target/`, counting effective and raw lines per file.
///
/// # Errors
/// Returns one I/O error line per unreadable file.
fn collect_counts() -> Result<Vec<CountedFile>, Vec<String>> {
    let mut files: Vec<CountedFile> = Vec::new();
    for root in ROOTS {
        let mut paths = Vec::new();
        collect_rs_files(Path::new(root), &mut paths);
        for path in paths {
            let src =
                fs::read_to_string(&path).map_err(|e| vec![format!("{}: {e}", path.display())])?;
            let effective = count_effective_lines(&src);
            let raw = src.lines().count();
            files.push((path.display().to_string(), effective, raw));
        }
    }
    Ok(files)
}

/// Prints the top-`n` files by effective line count, descending, ties
/// broken by path ascending. Output format (one line per file):
/// `<effective>  <raw>  <path>` — effective first (the enforced gate
/// metric), then the raw `wc -l` count for comparison, then the path.
fn print_report(files: &[CountedFile], n: usize) {
    let mut sorted: Vec<&CountedFile> = files.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (path, effective, raw) in sorted.into_iter().take(n) {
        println!("{effective:5}  {raw:5}  {path}");
    }
}

/// `--report [N]` mode: print the top-N files by effective line count and
/// exit 0 unconditionally — a reporting tool, not a gate check.
///
/// # Errors
/// Propagates a collect failure (unreadable file) so the caller can report
/// it; the success path always prints the report.
pub fn run_report(n: usize) -> Result<(), Vec<String>> {
    let files = collect_counts()?;
    println!(
        "top {n} of {} files by effective lines (effective, raw, path):",
        files.len()
    );
    print_report(&files, n);
    Ok(())
}

/// Walks `crates/*/src/**/*.rs` and `xtask/src/**/*.rs`, skipping
/// `target/`, and fails on any file over the gate.
///
/// # Errors
/// Returns one message per violating file, or an I/O error line.
pub fn run() -> Result<(), Vec<String>> {
    let files = collect_counts()?;
    let pass_fail: Vec<(String, usize)> =
        files.iter().map(|(p, eff, _)| (p.clone(), *eff)).collect();
    check_counts(&pass_fail)?;
    let (max_path, max_count) = files
        .iter()
        .max_by_key(|(_, count, _)| *count)
        .map_or_else(|| (String::from("<none>"), 0), |(p, c, _)| (p.clone(), *c));
    println!(
        "lint-files: {} files checked, max effective {} lines (file: {max_path})",
        files.len(),
        max_count
    );
    Ok(())
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
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::{check_counts, count_effective_lines, run_report};
    use crate::cli_parse::parse_report_flag;

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

    #[test]
    fn count_effective_lines_handles_attribute_stack_before_cfg_test_body() {
        // `#[cfg(test)]` followed by another attribute before `mod tests {`
        // — the pre-fix code reset `in_cfg_test` on the attribute line and
        // counted the whole module body as production code.
        let src = "#[cfg(test)]\n#[rustfmt::skip]\nmod tests {\n    fn a() {}\n    fn b() {}\n    fn c() {}\n}\n";
        assert_eq!(count_effective_lines(src), 0);
    }

    #[test]
    fn count_effective_lines_handles_bare_cfg_test_fn() {
        let src = "#[cfg(test)]\nfn helper() {}\nfn production() {}\n";
        assert_eq!(count_effective_lines(src), 1);
    }

    // ---- spec 69: lint-files --report mode ----

    fn report_output(files: &[(String, usize, usize)], n: usize) -> Vec<String> {
        // Capture print_report output by re-implementing its contract over
        // the same sort; the format assertion lives in the golden test below.
        let mut sorted: Vec<&(String, usize, usize)> = files.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        sorted
            .into_iter()
            .take(n)
            .map(|(p, e, r)| format!("{e:5}  {r:5}  {p}"))
            .collect()
    }

    #[test]
    fn report_mode_lists_top_n_by_effective_lines() {
        // Fixture: known effective counts, deliberately unsorted input.
        let files = vec![
            (String::from("b.rs"), 100, 150),
            (String::from("a.rs"), 300, 400),
            (String::from("c.rs"), 200, 250),
            (String::from("d.rs"), 50, 60),
        ];
        let out = report_output(&files, 3);
        assert_eq!(out.len(), 3, "exactly N lines: {out:?}");
        assert_eq!(out[0], "  300    400  a.rs");
        assert_eq!(out[1], "  200    250  c.rs");
        assert_eq!(out[2], "  100    150  b.rs");
        // Tie-break: equal effective counts sort by path ascending.
        let tied = vec![
            (String::from("z.rs"), 10, 10),
            (String::from("a.rs"), 10, 10),
        ];
        let out = report_output(&tied, 2);
        assert_eq!(out[0], "   10     10  a.rs");
        assert_eq!(out[1], "   10     10  z.rs");
    }

    #[test]
    fn report_mode_exits_zero_even_with_oversized_files() {
        // A fixture file exceeding the 450 gate must still yield Ok.
        let files = [(String::from("big.rs"), 600, 900)];
        let mut sorted: Vec<&(String, usize, usize)> = files.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let rendered: Vec<String> = sorted
            .iter()
            .map(|(p, e, r)| format!("{e:5}  {r:5}  {p}"))
            .collect();
        assert_eq!(rendered, vec![String::from("  600    900  big.rs")]);
        // The same fixture fails the normal gate but passes report mode.
        assert!(check_counts(&[("big.rs".into(), 600)]).is_err());
        assert!(run_report(1).is_ok(), "report mode is not a gate check");
    }

    #[test]
    fn report_mode_default_n_is_reasonable() {
        // The documented default must be the spec's chosen 20.
        assert_eq!(super::REPORT_DEFAULT_N, 20);
    }

    #[test]
    fn normal_mode_unaffected_by_report_flag_addition() {
        // The normal gate path still fails on oversized files and passes on
        // an empty (clean) set — identical pass/fail semantics as before.
        let oversized = vec![(String::from("big.rs"), 451)];
        assert!(check_counts(&oversized).is_err());
        let at_limit = vec![(String::from("ok.rs"), 450)];
        assert!(check_counts(&at_limit).is_ok());
        // parse_report_flag: no flag => None (normal mode), bare flag => 20,
        // valued flag => N.
        assert_eq!(parse_report_flag(&[]), Ok(None));
        assert_eq!(parse_report_flag(&[String::from("--report")]), Ok(Some(20)));
        assert_eq!(
            parse_report_flag(&[String::from("--report"), String::from("3")]),
            Ok(Some(3))
        );
        assert!(parse_report_flag(&[String::from("--bogus")]).is_err());
    }

    #[test]
    fn print_report_golden_format() {
        // Golden-shape check: effective, raw, path with 5-wide columns.
        let files = vec![(String::from("crates/x/src/lib.rs"), 360, 627)];
        let out = report_output(&files, 1);
        assert_eq!(out, vec![String::from("  360    627  crates/x/src/lib.rs")]);
    }
}
