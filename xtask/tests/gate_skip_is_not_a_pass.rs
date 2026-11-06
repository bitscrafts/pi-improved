//! Spec 395: a skipped gate leaves the row `pending` (attempts unchanged), and a missing write dir is WIRING
//! (`disk-floor.sh` exit 4), not a disk shortage. Driven through the real scripts.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn floor(args: &[&str]) -> (i32, String) {
    let out = Command::new("bash")
        .arg("scripts/autonomy/disk-floor.sh")
        .args(args)
        .current_dir(root())
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn skip_branch() -> String {
    let src = std::fs::read_to_string(root().join("scripts/autonomy/cycle.sh")).unwrap();
    let mut on = false;
    let mut out = String::new();
    for l in src.lines() {
        if l.contains("if [ \"$DRC\" -ne 0 ]; then") {
            on = true;
        }
        if on {
            out.push_str(l);
            out.push('\n');
            if l.contains("exit 13") {
                break;
            }
        }
    }
    out
}

#[test]
fn a_skipped_gate_leaves_the_row_pending() {
    let b = skip_branch();
    assert!(
        b.contains("status=pending"),
        "skip must leave the row pending:\n{b}"
    );
    assert!(
        !b.contains("needs-review"),
        "skip must not mark needs-review:\n{b}"
    );
    assert!(
        !b.contains("attempts="),
        "skip must not touch attempts:\n{b}"
    );
    assert!(
        b.contains("last=gate-skipped:"),
        "reason must be last=gate-skipped:<why>:\n{b}"
    );
}

#[test]
fn a_missing_write_dir_is_wiring() {
    let dir = root()
        .join("_tmp")
        .join(format!("gate-skip-395-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let absent = dir.join("absent-dir");
    let (rc, out) = floor(&["1", dir.to_str().unwrap(), absent.to_str().unwrap()]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(rc, 4, "MISSING must exit 4: {out}");
    assert!(
        out.contains(absent.to_str().unwrap()),
        "message must name the path: {out}"
    );
}

#[test]
fn control_a_green_gate_still_advances() {
    let (rc, out) = floor(&["1", "/projects"]);
    assert_eq!(rc, 0, "room on disk must pass the floor: {out}");
    let (rc, _) = floor(&["999999999", "/projects"]);
    assert_eq!(rc, 1, "a real shortage stays exit 1");
}
