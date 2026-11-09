//! Spec 402 TDD contract: a file's size limit (450 physical lines) is enforced per file, first, and one counter decides it.
//!
//! The tests drive the real binary (`CARGO_BIN_EXE_xtask`) so they pin the command-line contract, not any library
//! surface. Fixtures are scratch workspaces under `_tmp/`, selected by the cwd: `workspace_root` walks up from the cwd
//! to the first directory holding both `Cargo.toml` and `xtask/Cargo.toml`.
//!
//! Baseline format assumed here (the spec's R2): `_docs/05-operations/crate-audit-baseline.tsv`, a `path<TAB>lines`
//! header line, then one `path<TAB>lines` row per over-budget file. The old per-crate format is `crate<TAB>over_budget`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BASELINE: &str = "_docs/05-operations/crate-audit-baseline.tsv";
const GATE_STEPS: &str = "../gate-steps.txt";

fn xtask(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("the xtask binary runs")
}

fn combined(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn put(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// `lines` lines of filler comment: physical lines and effective lines differ by design only in the dense fixture.
fn filler(lines: usize) -> String {
    "// filler\n".repeat(lines)
}

/// A fresh scratch workspace with the two markers `workspace_root` looks for, and one crate `pii-x`.
fn fixture(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../_tmp/file-size")
        .join(format!("{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    put(
        &dir,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/*\", \"xtask\"]\nresolver = \"2\"\n",
    );
    put(
        &dir,
        "xtask/Cargo.toml",
        "[package]\nname = \"xtask\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    );
    put(
        &dir,
        "crates/pii-x/Cargo.toml",
        "[package]\nname = \"pii-x\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    );
    dir
}

/// Writes the new-format baseline: a `path<TAB>lines` header, then one row per `(path, lines)`.
fn write_baseline(root: &Path, rows: &[(&str, usize)]) {
    let mut body = String::from("path\tlines\n");
    for (path, lines) in rows {
        writeln!(body, "{path}\t{lines}").unwrap();
    }
    put(root, BASELINE, &body);
}

fn baseline_text(root: &Path) -> String {
    std::fs::read_to_string(root.join(BASELINE)).unwrap()
}

#[test]
fn a_new_file_over_450_is_refused_naming_it() {
    let root = fixture("new-over");
    put(&root, "crates/pii-x/src/big.rs", &filler(451));
    write_baseline(&root, &[]);

    let out = xtask(&root, &["crate-audit", "--ratchet"]);
    let text = combined(&out);
    assert!(
        !out.status.success(),
        "a 451-line file not in the baseline must be refused: {text}"
    );
    assert!(
        text.contains("crates/pii-x/src/big.rs") && text.contains("451"),
        "the refusal must name the path and its 451 lines: {text}"
    );
}

#[test]
#[allow(non_snake_case)] // reason: the spec names this test with the CONTROL_ prefix
fn CONTROL_a_450_line_file_passes() {
    let root = fixture("new-at-limit");
    put(&root, "crates/pii-x/src/edge.rs", &filler(450));
    write_baseline(&root, &[]);

    let out = xtask(&root, &["crate-audit", "--ratchet"]);
    assert!(
        out.status.success(),
        "a 450-line file is at the limit, not over it: {}",
        combined(&out)
    );
}

#[test]
fn a_baselined_file_that_grew_is_refused() {
    let root = fixture("grew");
    put(&root, "crates/pii-x/src/big.rs", &filler(501));
    write_baseline(&root, &[("crates/pii-x/src/big.rs", 500)]);

    let out = xtask(&root, &["crate-audit", "--ratchet"]);
    let text = combined(&out);
    assert!(
        !out.status.success(),
        "a baselined file that grew past its recorded 500 lines must be refused: {text}"
    );
    assert!(
        text.contains("crates/pii-x/src/big.rs") && text.contains("501") && text.contains("500"),
        "the refusal must name the path, its 501 lines and the recorded 500: {text}"
    );
}

#[test]
#[allow(non_snake_case)] // reason: the spec names this test with the CONTROL_ prefix
fn CONTROL_a_baselined_file_at_its_recorded_count_passes() {
    let root = fixture("same-count");
    put(&root, "crates/pii-x/src/big.rs", &filler(500));
    write_baseline(&root, &[("crates/pii-x/src/big.rs", 500)]);

    let out = xtask(&root, &["crate-audit", "--ratchet"]);
    assert!(
        out.status.success(),
        "a baselined file at its recorded 500 lines is not growth: {}",
        combined(&out)
    );
}

#[test]
fn a_baselined_file_is_not_replaced_by_another_over_budget_file() {
    // The acceptance case: per-crate counts stay equal (one over-budget file before, one after), so a count-based
    // ratchet passes it. The per-file ratchet must refuse B, and A's stale entry must go.
    let root = fixture("swap");
    put(&root, "crates/pii-x/src/a.rs", &filler(440));
    put(&root, "crates/pii-x/src/b.rs", &filler(460));
    write_baseline(&root, &[("crates/pii-x/src/a.rs", 500)]);

    let out = xtask(&root, &["crate-audit", "--ratchet"]);
    let text = combined(&out);
    assert!(
        !out.status.success(),
        "a second over-budget file must not take the place of a baselined one: {text}"
    );
    assert!(
        text.contains("crates/pii-x/src/b.rs") && text.contains("460"),
        "the refusal must name the new over-budget file B and its 460 lines: {text}"
    );
    assert!(
        text.contains("crates/pii-x/src/a.rs"),
        "A is under the limit and still listed, so its stale entry must be named: {text}"
    );
}

#[test]
fn a_baselined_file_under_the_limit_must_leave_the_baseline() {
    let root = fixture("stale");
    put(&root, "crates/pii-x/src/a.rs", &filler(440));
    write_baseline(&root, &[("crates/pii-x/src/a.rs", 500)]);

    let out = xtask(&root, &["crate-audit", "--ratchet"]);
    let text = combined(&out);
    assert!(
        !out.status.success(),
        "a file at or under 450 lines that is still listed must be refused: {text}"
    );
    assert!(
        text.contains("crates/pii-x/src/a.rs"),
        "the refusal must name the stale entry: {text}"
    );
}

#[test]
#[allow(non_snake_case)] // reason: the spec names this test with the CONTROL_ prefix
fn CONTROL_a_baselined_file_that_shrank_but_is_still_over_passes() {
    let root = fixture("shrank");
    put(&root, "crates/pii-x/src/a.rs", &filler(451));
    write_baseline(&root, &[("crates/pii-x/src/a.rs", 500)]);

    let out = xtask(&root, &["crate-audit", "--ratchet"]);
    assert!(
        out.status.success(),
        "shrinking a baselined file while it stays over the limit is allowed: {}",
        combined(&out)
    );
}

#[test]
fn a_baselined_path_that_is_gone_must_leave_the_baseline() {
    let root = fixture("gone");
    put(&root, "crates/pii-x/src/ok.rs", &filler(10));
    write_baseline(&root, &[("crates/pii-x/src/gone.rs", 500)]);

    let out = xtask(&root, &["crate-audit", "--ratchet"]);
    let text = combined(&out);
    assert!(
        !out.status.success(),
        "a baselined path that no longer exists must be refused: {text}"
    );
    assert!(
        text.contains("crates/pii-x/src/gone.rs"),
        "the refusal must name the missing path: {text}"
    );
}

#[test]
fn update_baseline_refuses_to_add_or_raise() {
    // A new over-budget path.
    let root = fixture("update-add");
    put(&root, "crates/pii-x/src/big.rs", &filler(451));
    write_baseline(&root, &[]);
    let before = baseline_text(&root);

    let out = xtask(&root, &["crate-audit", "--update-baseline"]);
    assert!(
        !out.status.success(),
        "--update-baseline must refuse to add a new over-budget path: {}",
        combined(&out)
    );
    assert_eq!(
        baseline_text(&root),
        before,
        "a refused --update-baseline must leave the baseline file unchanged"
    );

    // A higher count for a path already listed.
    let root = fixture("update-raise");
    put(&root, "crates/pii-x/src/a.rs", &filler(501));
    write_baseline(&root, &[("crates/pii-x/src/a.rs", 500)]);
    let before = baseline_text(&root);

    let out = xtask(&root, &["crate-audit", "--update-baseline"]);
    assert!(
        !out.status.success(),
        "--update-baseline must refuse to raise a recorded count: {}",
        combined(&out)
    );
    assert_eq!(
        baseline_text(&root),
        before,
        "a refused --update-baseline must leave the baseline file unchanged"
    );
}

#[test]
#[allow(non_snake_case)] // reason: the spec names this test with the CONTROL_ prefix
fn CONTROL_update_baseline_may_lower_a_count() {
    let root = fixture("update-lower");
    put(&root, "crates/pii-x/src/a.rs", &filler(480));
    write_baseline(&root, &[("crates/pii-x/src/a.rs", 500)]);

    let out = xtask(&root, &["crate-audit", "--update-baseline"]);
    assert!(
        out.status.success(),
        "--update-baseline may lower a recorded count: {}",
        combined(&out)
    );
    assert!(
        baseline_text(&root).contains("crates/pii-x/src/a.rs\t480"),
        "the lowered count must be written: {}",
        baseline_text(&root)
    );
}

#[test]
fn both_commands_use_the_same_counter() {
    // Effective lines (non-blank, non-comment) = 20, physical lines = 465. `lint-files` used to judge the effective
    // count and passed it; `crate-audit` judged the physical count and refused it. One counter must refuse it in both.
    let root = fixture("same-counter");
    let mut dense = String::new();
    for i in 0..20 {
        writeln!(dense, "pub const K{i}: u32 = {i};").unwrap();
    }
    dense.push_str(&"// comment\n".repeat(445));
    put(&root, "crates/pii-x/src/dense.rs", &dense);
    write_baseline(&root, &[]);

    let lint = xtask(&root, &["lint-files"]);
    let lint_text = combined(&lint);
    assert!(
        !lint.status.success() && lint_text.contains("dense.rs"),
        "lint-files must refuse a 465-physical-line file, whatever its effective count: {lint_text}"
    );

    let audit = xtask(&root, &["crate-audit", "--ratchet"]);
    assert!(
        !audit.status.success(),
        "crate-audit must refuse the same 465-physical-line file: {}",
        combined(&audit)
    );
}

#[test]
fn size_checks_run_before_the_build() {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(GATE_STEPS))
        .expect("gate-steps.txt is readable");
    let steps: Vec<(&str, &str)> = text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('|'))
        .collect();

    let fmt = steps
        .iter()
        .position(|(label, _)| *label == "fmt")
        .expect("gate-steps.txt names an fmt step");
    let clippy = steps
        .iter()
        .position(|(label, _)| label.starts_with("clippy"))
        .expect("gate-steps.txt names a clippy step");
    let audit = steps
        .iter()
        .position(|(_, cmd)| cmd.contains("xtask -- crate-audit"))
        .expect("gate-steps.txt runs crate-audit");
    let lint = steps
        .iter()
        .position(|(_, cmd)| cmd.contains("xtask -- lint-files"))
        .expect("gate-steps.txt runs lint-files");

    assert!(
        audit < fmt && audit < clippy,
        "crate-audit must run before fmt and clippy, so a size failure costs seconds: step {audit} vs fmt {fmt}, clippy {clippy}"
    );
    assert!(
        lint < fmt && lint < clippy,
        "lint-files must run before fmt and clippy, so a size failure costs seconds: step {lint} vs fmt {fmt}, clippy {clippy}"
    );
}

#[test]
fn the_baseline_migration_refuses_when_a_file_exceeds_its_crate_count() {
    // The old per-crate baseline records pii-x as holding zero over-budget files; the tree holds one.
    let root = fixture("migrate-refuse");
    put(&root, "crates/pii-x/src/big.rs", &filler(451));
    put(&root, BASELINE, "crate\tover_budget\npii-x\t0\n");
    let before = baseline_text(&root);

    let out = xtask(&root, &["crate-audit", "--migrate-from-crate-counts"]);
    assert!(
        !out.status.success(),
        "a file over its old crate's recorded count must refuse the migration: {}",
        combined(&out)
    );
    assert_eq!(
        baseline_text(&root),
        before,
        "a refused migration must write nothing"
    );
}

#[test]
#[allow(non_snake_case)] // reason: the spec names this test with the CONTROL_ prefix
fn CONTROL_the_baseline_migration_writes_when_every_file_fits_its_crate_count() {
    let root = fixture("migrate-write");
    put(&root, "crates/pii-x/src/big.rs", &filler(451));
    put(&root, BASELINE, "crate\tover_budget\npii-x\t1\n");

    let out = xtask(&root, &["crate-audit", "--migrate-from-crate-counts"]);
    assert!(
        out.status.success(),
        "one over-budget file against a recorded count of 1 must migrate: {}",
        combined(&out)
    );
    let text = baseline_text(&root);
    assert!(
        text.starts_with("path\tlines\n") && text.contains("crates/pii-x/src/big.rs\t451"),
        "the migrated baseline must be the path<TAB>lines format, recording the file: {text}"
    );
}
