//! Spec 387 TDD contract: `cargo xtask crate-audit` measures every crate against the 450-line budget and the
//! ratchet refuses any crate whose over-budget count grows past its checked-in baseline.
//!
//! The tests drive the real binary (`CARGO_BIN_EXE_xtask`) so they pin the command-line contract, not any
//! library surface. Growth fixtures are scratch workspaces under `_tmp/`, selected by the cwd: `workspace_root`
//! walks up from the cwd to the first directory holding both `Cargo.toml` and `xtask/Cargo.toml`.
//!
//! Baseline format assumed here (the spec leaves it open): `_docs/05-operations/crate-audit-baseline.tsv`, a
//! `crate<TAB>over_budget` header line, then one `crate<TAB>count` row per crate.
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

const BUDGET: usize = 450;
const BASELINE: &str = "_docs/05-operations/crate-audit-baseline.tsv";

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

/// A fresh scratch workspace with the two markers `workspace_root` looks for, and the given crates.
fn fixture(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../_tmp/crate-audit")
        .join(format!("{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("xtask")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/*\", \"xtask\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("xtask/Cargo.toml"),
        "[package]\nname = \"xtask\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    dir
}

/// Adds a crate `name` whose single source file `file` holds `lines` lines.
fn add_crate(root: &Path, name: &str, file: &str, lines: usize) {
    let src = root.join("crates").join(name).join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        root.join("crates").join(name).join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n"),
    )
    .unwrap();
    std::fs::write(src.join(file), "// filler\n".repeat(lines)).unwrap();
}

fn write_baseline(root: &Path, rows: &[(&str, usize)]) {
    let path = root.join(BASELINE);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut body = String::from("crate\tover_budget\n");
    for (name, count) in rows {
        writeln!(body, "{name}\t{count}").unwrap();
    }
    std::fs::write(path, body).unwrap();
}

/// Every `.rs` file under `dir`, recursively.
fn rs_files(dir: &Path, out: &mut usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            *out += 1;
        }
    }
}

#[test]
fn the_audit_measures_every_crate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut names: Vec<String> = std::fs::read_dir(root.join("crates"))
        .unwrap()
        .flatten()
        .filter(|e| e.path().join("Cargo.toml").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names.len(),
        22,
        "the workspace is expected to hold 22 crates: {names:?}"
    );

    let mut source_files = 0;
    for name in &names {
        rs_files(
            &root.join("crates").join(name).join("src"),
            &mut source_files,
        );
    }

    let out = xtask(&root, &["crate-audit"]);
    let text = combined(&out);
    assert!(
        out.status.success(),
        "crate-audit must succeed on the workspace: {text}"
    );

    for name in &names {
        let row = text
            .lines()
            .any(|l| l.split_whitespace().next() == Some(name.as_str()));
        assert!(row, "no row for crate {name} in:\n{text}");
    }

    let total = text
        .lines()
        .find(|l| {
            l.split_whitespace()
                .next()
                .is_some_and(|w| w.eq_ignore_ascii_case("total"))
        })
        .unwrap_or_else(|| panic!("no total row in:\n{text}"));
    let plain = source_files.to_string();
    let grouped = plain
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join(",");
    assert!(
        total.split_whitespace().any(|w| w == plain || w == grouped),
        "the total row {total:?} must carry the recounted source files ({source_files})"
    );
}

#[test]
fn the_audit_refuses_growth() {
    let root = fixture("growth");
    add_crate(&root, "pii-x", "big.rs", BUDGET + 1);
    write_baseline(&root, &[("pii-x", 0)]);

    let out = xtask(&root, &["crate-audit", "--ratchet"]);
    let text = combined(&out);
    assert!(
        !out.status.success(),
        "a crate over its baseline must fail the ratchet: {text}"
    );
    assert!(
        text.contains("crates/pii-x/src/big.rs"),
        "the refusal must name the file: {text}"
    );
}

#[test]
fn the_audit_allows_payment() {
    let root = fixture("payment");
    add_crate(&root, "pii-x", "a.rs", BUDGET + 1);
    add_crate(&root, "pii-x", "b.rs", BUDGET + 1);
    write_baseline(&root, &[("pii-x", 3)]);

    let out = xtask(&root, &["crate-audit", "--ratchet"]);
    assert!(
        out.status.success(),
        "2 over-budget files against a baseline of 3 is payment, not growth: {}",
        combined(&out)
    );
}

#[test]
#[allow(non_snake_case)] // reason: the spec names this test with the CONTROL_ prefix
fn CONTROL_a_new_file_under_the_budget_is_not_a_violation() {
    let root = fixture("under-budget");
    add_crate(&root, "pii-x", "fresh.rs", 400);
    write_baseline(&root, &[("pii-x", 0)]);

    let out = xtask(&root, &["crate-audit", "--ratchet"]);
    assert!(
        out.status.success(),
        "a 400-line file is under the 450-line budget: {}",
        combined(&out)
    );
}

#[test]
fn control_the_baseline_cannot_be_raised_by_the_audit_itself() {
    let root = fixture("no-self-baseline");
    add_crate(&root, "pii-x", "big.rs", BUDGET + 1);
    write_baseline(&root, &[("pii-x", 0)]);
    let before = std::fs::read(root.join(BASELINE)).unwrap();

    xtask(&root, &["crate-audit"]);
    let ratchet = xtask(&root, &["crate-audit", "--ratchet"]);

    let after = std::fs::read(root.join(BASELINE)).unwrap();
    assert_eq!(
        before, after,
        "the audit must not rewrite its baseline without an explicit --update-baseline"
    );
    // A bare non-zero exit is not enough: an unknown subcommand also exits non-zero and leaves the baseline alone.
    let text = combined(&ratchet);
    assert!(
        !ratchet.status.success() && text.contains("crates/pii-x/src/big.rs"),
        "the growth must still be refused, by name, after a plain run: {text}"
    );
}
