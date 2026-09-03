//! Contract tests for the `pii-store` journal/index/snapshot API (spec F3.3).
//!
//! Each test owns its database under `/projects/pi-improved/_tmp` (N8).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;

use pii_abi::{RunEvent, RunHeader, RunId, RunStatus, Terminal};
use pii_store::Store;

fn tmp_store(name: &str) -> (PathBuf, Store) {
    let dir = PathBuf::from("/projects/pi-improved/_tmp/pi-store-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!(
        "{}-{}-{}.redb",
        name,
        std::process::id(),
        std::time::Instant::now().elapsed().as_nanos()
    ));
    let _ = std::fs::remove_file(&path);
    let store = Store::open(&path).unwrap();
    (path, store)
}

fn header(run: &RunId) -> RunHeader {
    RunHeader {
        run: run.clone(),
        graph: "g".to_string(),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 0,
    }
}

fn started(graph: &str) -> RunEvent {
    RunEvent::Started {
        graph: graph.to_string(),
        input_hash: "h".to_string(),
    }
}

#[test]
fn append_event_advances_header_seq_atomically() {
    let (_p, store) = tmp_store("seq-atomic");
    let run = RunId::from("r000000000421");
    let mut h = header(&run);
    let ev = started("g");
    let s0 = store.append_event(&mut h, &ev).unwrap();
    let s1 = store.append_event(&mut h, &ev).unwrap();
    let s2 = store.append_event(&mut h, &ev).unwrap();
    assert_eq!((s0, s1, s2), (0, 1, 2));
    assert_eq!(h.seq, 3);
    let loaded = store.get_run(&run).unwrap().unwrap();
    assert_eq!(loaded.seq, 3);
    let journal = store.read_journal(&run).unwrap();
    assert_eq!(journal.len(), 3);
    assert_eq!(journal[0].0, 0);
    assert_eq!(journal[1].0, 1);
    assert_eq!(journal[2].0, 2);
}

#[test]
fn run_by_status_index_is_repointed() {
    let (_p, store) = tmp_store("status-repoint");
    let run = RunId::from("r000000000422");
    let mut h = header(&run);
    store.append_event(&mut h, &started("g")).unwrap();
    assert!(
        store
            .runs_by_status(RunStatus::Running)
            .unwrap()
            .contains(&run)
    );

    h.status = RunStatus::Succeeded;
    store
        .append_event(
            &mut h,
            &RunEvent::Finished {
                terminal: Terminal::Success {
                    output: serde_json::json!(1),
                },
                output_hash: "h".to_string(),
            },
        )
        .unwrap();

    assert!(
        !store
            .runs_by_status(RunStatus::Running)
            .unwrap()
            .contains(&run),
        "stale Running row must be gone"
    );
    assert!(
        store
            .runs_by_status(RunStatus::Succeeded)
            .unwrap()
            .contains(&run),
        "run must be listed under Succeeded"
    );
}

#[test]
fn next_run_id_is_monotonic() {
    let (_p, store) = tmp_store("monotonic");
    let a = store.next_run_id().unwrap();
    let b = store.next_run_id().unwrap();
    assert!(b > a, "second must sort after first lexicographically");
    for id in [&a, &b] {
        let s = id.as_str();
        assert_eq!(s.len(), 13, "want r + 12 digits, got {s}");
        assert!(s.starts_with('r'));
        assert!(s[1..].chars().all(|c| c.is_ascii_digit()), "got {s}");
    }
}

#[test]
fn snapshot_roundtrips() {
    let (_p, store) = tmp_store("snap-rt");
    let run = RunId::from("r000000000423");
    store.put_snapshot(&run, 32, b"{}").unwrap();
    let (seq, bytes) = store.latest_snapshot(&run).unwrap().unwrap();
    assert_eq!(seq, 32);
    assert_eq!(bytes, b"{}");
}

#[test]
fn graph_body_is_content_addressed() {
    let (_p, store) = tmp_store("content-addr");
    let body = br#"{ "name": "content-addressed" }"#;
    let k1 = store.put_graph(body).unwrap();
    let k2 = store.put_graph(body).unwrap();
    assert_eq!(k1, k2, "same bytes must hash to the same key");
    let got = store.get_graph(&k1).unwrap().unwrap();
    assert_eq!(got, body);
}
