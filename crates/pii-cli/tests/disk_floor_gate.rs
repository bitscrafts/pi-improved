//! Spec 438 (Exit Criteria): the gate refuses for disk before its first step, `pii disk clean` has a
//! dry-run that says what it would free, refuses a path outside `_tmp/`, and the free-space floor is
//! named in the source. Each test mirrors one exit-criterion command from the spec.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary and reads its output; it walks the source tree.
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The workspace root: the criteria are run from the repository root.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `pii <args>` run from the repository root with an isolated runtime dir, so no real daemon is reached.
fn pii(args: &[&str]) -> (bool, String, String) {
    let run = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_pii"))
        .current_dir(repo_root())
        .env("PII_RUNTIME_DIR", run.path())
        .env_remove("PII_CONFIG")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Stdout and stderr together, as `2>&1` joins them in a shell pipeline.
fn combined(stdout: &str, stderr: &str) -> String {
    format!("{stdout}{stderr}")
}

/// Every file under `dir`, recursively.
fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_under(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// `pii gate run --routine --dry-run 2>&1 | grep -qE 'disk|free'`
#[test]
fn gate_dry_run_names_the_disk_it_needs() {
    let (_, stdout, stderr) = pii(&["gate", "run", "--routine", "--dry-run"]);
    let text = combined(&stdout, &stderr);

    assert!(
        text.contains("disk") || text.contains("free"),
        "the gate's dry run does not mention disk or free space: {text}"
    );
}

/// `pii disk clean --dry-run | grep -q 'would'`
#[test]
fn disk_clean_dry_run_says_what_it_would_free() {
    let (_, stdout, _) = pii(&["disk", "clean", "--dry-run"]);

    assert!(
        stdout.contains("would"),
        "pii disk clean --dry-run does not say what it would free: {stdout}"
    );
}

/// `pii disk clean --path .pii 2>&1 | grep -qiE 'refus|outside'`
#[test]
fn disk_clean_refuses_a_path_outside_tmp() {
    let (_, stdout, stderr) = pii(&["disk", "clean", "--path", ".pii"]);
    let text = combined(&stdout, &stderr).to_lowercase();

    assert!(
        text.contains("refus") || text.contains("outside"),
        ".pii is outside _tmp/ but the clean did not refuse it: {text}"
    );
}

/// `grep -rqE 'free_gb|AVAIL_GB|disk_floor' crates/pii-cli/src/ crates/pii-server/src/`
#[test]
fn the_free_space_floor_is_named_in_the_source() {
    let root = repo_root();
    let mut files = Vec::new();
    files_under(&root.join("crates/pii-cli/src"), &mut files);
    files_under(&root.join("crates/pii-server/src"), &mut files);

    let named = files.iter().any(|f| {
        std::fs::read_to_string(f).is_ok_and(|s| {
            s.contains("free_gb") || s.contains("AVAIL_GB") || s.contains("disk_floor")
        })
    });

    assert!(
        named,
        "no free_gb, AVAIL_GB or disk_floor in crates/pii-cli/src or crates/pii-server/src"
    );
}
