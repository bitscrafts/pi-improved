//! Spec 169: a parked/needs-review head must not freeze the autonomy queue — `state.py pick` skips it, names it, and
//! dispatches the next dispatchable row; when every row is parked it still answers NONE.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::{Command, Output};

fn pick(dir: &std::path::Path) -> Output {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    Command::new("python3")
        .arg(root.join("scripts/autonomy/state.py"))
        .arg("pick")
        .env("PII_PLAN", dir.join("plan.tsv"))
        .env("PII_STATE", dir.join("state.json"))
        .output()
        .expect("python3 runs")
}

fn fixture(states: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let mut plan = String::new();
    let mut st = serde_json::Map::new();
    for (id, status) in states {
        plan.push_str(&format!("{id}\tspec.md\t-\ttitle {id}\t-\t-\t0\n"));
        st.insert((*id).into(), serde_json::json!({ "status": status }));
    }
    std::fs::write(dir.path().join("plan.tsv"), plan).unwrap();
    std::fs::write(dir.path().join("state.json"), serde_json::Value::Object(st).to_string()).unwrap();
    dir
}

#[test]
fn a_parked_head_does_not_freeze_the_queue() {
    for head in ["needs-review", "parked"] {
        let dir = fixture(&[("H1", head), ("P2", "pending")]);
        let out = pick(dir.path());
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{stderr}");
        assert_eq!(stdout.split('\t').next().unwrap(), "P2", "head {head} must be skipped: {stdout}");
        assert!(stderr.contains("H1") && stderr.contains("skip"), "the skipped row is named: {stderr}");
    }
}

#[test]
fn all_parked_says_so_and_picks_none() {
    let dir = fixture(&[("H1", "parked"), ("H2", "needs-review")]);
    let out = pick(dir.path());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "NONE");
    assert!(String::from_utf8_lossy(&out.stderr).contains("parked"));
}
