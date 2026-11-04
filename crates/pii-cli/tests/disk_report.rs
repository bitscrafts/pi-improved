//! Spec 367 R1: `pii disk` prints the artefact roots the repository fills, with bytes, age and the
//! filesystem each lives on, and names the filesystem that holds the repository. R3: a target created
//! outside the owned `_tmp/targets/` root is reported as unowned rather than ignored.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;

fn scratch() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("disk-report-")
        .tempdir_in(root)
        .unwrap()
}

fn pii_disk(repo: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_pii"))
        .arg("disk")
        .arg("--root")
        .arg(repo)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn disk_names_both_filesystems_and_the_gate_cache() {
    let repo = scratch();
    let cache = repo.path().join("_tmp/gate-target/debug");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("artefact"), vec![0u8; 4096]).unwrap();

    let (ok, text) = pii_disk(repo.path());

    assert!(ok, "pii disk exited non-zero: {text}");
    assert!(text.contains("filesystem"), "no filesystem line: {text}");
    assert!(
        text.contains("gate-target"),
        "gate cache not listed: {text}"
    );
}

#[test]
fn disk_reports_a_target_outside_the_owned_root_as_unowned() {
    let repo = scratch();
    std::fs::create_dir_all(repo.path().join("_tmp/verify-x-target")).unwrap();

    let (ok, text) = pii_disk(repo.path());

    assert!(ok, "pii disk exited non-zero: {text}");
    assert!(
        text.lines()
            .any(|l| l.contains("verify-x-target") && l.contains("unowned")),
        "stray target not reported as unowned: {text}"
    );
}
