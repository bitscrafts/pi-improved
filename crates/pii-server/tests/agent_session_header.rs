//! Spec 185 (session): a `pi` dispatch gets a session file with a valid header before it spawns, because the
//! installed pi rejects a missing or empty session file ("Session not found" / "Empty session file") before
//! any model is called. Controls: an existing header is left untouched, and an unwritable path refuses.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_server::node_kinds::nodes::agent_worker::{
    WorkerSpec, pi_session_path, write_pi_session_header,
};

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    // Scratch lives under the workspace's _tmp, never the system temp dir (the scratch lint enforces this).
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_tmp/session-tests");
    let d = root.join(format!("{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn a_fresh_session_gets_a_header_pi_accepts() {
    let d = tmp_dir("fresh");
    let f = d.join("s.jsonl");
    write_pi_session_header(f.to_str().unwrap(), "/work", 1_798_761_599_123).unwrap();
    let text = std::fs::read_to_string(&f).unwrap();
    let v: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(v["type"], "session");
    assert_eq!(v["version"], 3);
    assert_eq!(v["cwd"], "/work");
    assert!(v["id"].as_str().unwrap().len() == 36, "a UUID id");
    assert_eq!(v["timestamp"], "2026-12-31T23:59:59.123Z");
}

#[test]
fn a_resumed_session_keeps_its_header() {
    let d = tmp_dir("resume");
    let f = d.join("s.jsonl");
    std::fs::write(&f, "{\"type\":\"session\",\"id\":\"keep-me\"}\n").unwrap();
    write_pi_session_header(f.to_str().unwrap(), "/work", 0).unwrap();
    let text = std::fs::read_to_string(&f).unwrap();
    assert!(
        text.contains("keep-me"),
        "an existing session is not rewritten"
    );
}

#[test]
fn an_unwritable_session_path_is_an_error_not_a_spawn() {
    let d = tmp_dir("bad");
    // A regular file where a directory must be: create_dir_all cannot succeed.
    let blocker = d.join("blocker");
    std::fs::write(&blocker, "x").unwrap();
    let f = blocker.join("s.jsonl");
    assert!(write_pi_session_header(f.to_str().unwrap(), "/work", 0).is_err());
}

#[test]
fn the_default_session_path_follows_the_spec_stem() {
    let cfg = serde_json::json!({
        "agent": "pi",
        "model": "deepseek-v4-flash",
        "spec": "specs/207-the-coder-node-spawns-an-agent.md",
        "workdir": "/projects/x",
        "brief": "b",
    });
    let spec = WorkerSpec::from_cfg(&cfg).unwrap();
    assert_eq!(
        pi_session_path(&spec),
        "/projects/x/.pii-sessions/207-the-coder-node-spawns-an-agent.jsonl"
    );
}
