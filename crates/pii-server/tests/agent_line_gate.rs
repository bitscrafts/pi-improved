//! TDD contract test (2026-09-17 split of `agent.rs` → `agent/`):
//! every file in `crates/pii-server/src/agent/` must stay under the
//! workspace's 450-line gate.

// reason: this is a source-shape test (file line counts), not agent-runtime
// code; the paths are compile-time-pinned to this crate's own `src/agent/`.
#![allow(clippy::expect_used)]

use std::path::Path;

/// `agent_module_has_no_single_file_over_450_lines`: run the effective
/// line count over `crates/pii-server/src/agent/*.rs`; zero files may
/// exceed 450 effective lines.
///
/// Counting semantics match the workspace gate (`xtask/src/lint_files.rs`):
/// non-blank, non-comment lines, with `#[cfg(test)]` module bodies
/// excluded (F15 — tests do not count against the budget). `tests.rs`
/// carries an inner `#![cfg(test)]` and is declared `#[cfg(test)] mod
/// tests;` in `mod.rs`, so its whole body is a test body and is excluded.
#[test]
fn agent_module_has_no_single_file_over_450_lines() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/node_kinds/agent");
    let mut offenders: Vec<(String, usize)> = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("agent/ directory exists") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read source");
        let effective = count_effective_lines(&text);
        if effective > 450 {
            offenders.push((path.display().to_string(), effective));
        }
    }
    assert!(
        offenders.is_empty(),
        "files over the 450-line gate: {offenders:?}"
    );
}

/// Effective-line counter mirroring `xtask/src/lint_files.rs`:
/// excludes blank lines, `//` comments, and `#[cfg(test)]` bodies
/// (brace-depth tracked). A file whose body is entirely a test module
/// (`#![cfg(test)]` inner attribute — the standalone `mod tests;` file
/// shape used by `agent/tests.rs`) is a test body in its entirety and
/// counts as zero effective lines, per the same F15 semantics.
fn count_effective_lines(src: &str) -> usize {
    let mut count = 0usize;
    let mut in_cfg_test = false;
    let mut cfg_test_depth: isize = 0;
    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[cfg(test)]") {
            in_cfg_test = true;
            continue;
        }
        // Whole-file test module (F15): an inner `#![cfg(test)]` attribute
        // means every subsequent line of this file is test-only code.
        if trimmed == "#![cfg(test)]" {
            return 0;
        }
        let opens = isize::try_from(trimmed.matches('{').count()).unwrap_or(isize::MAX);
        let closes = isize::try_from(trimmed.matches('}').count()).unwrap_or(isize::MAX);
        if in_cfg_test {
            cfg_test_depth += opens - closes;
            if cfg_test_depth <= 0 {
                in_cfg_test = false;
                cfg_test_depth = 0;
            }
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        count += 1;
    }
    count
}
