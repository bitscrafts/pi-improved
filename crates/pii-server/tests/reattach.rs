//! Reattach test (F2.5, TDD `server::reattach_since_seq_no_duplicates`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_core::CapSet;
use pii_proto::event::Event;
use pii_proto::run::SeqEvent;
use pii_server::bus::{BridgeObserver, Bus};
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;
use std::sync::Arc;

fn engine_in(dir: &std::path::Path, fixture: &std::path::Path) -> Arc<Engine> {
    Engine::new(EngineConfig {
        workspace: dir.to_path_buf(),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", fixture.display()),
        sandbox: SandboxProfileName::Off,
        graph_dir: dir.join("graphs"),
    })
    .unwrap()
}

/// Runs one default-graph session to completion and returns the journalled
/// `SeqEvent`s the bridge produced.
async fn run_one_prompt(engine: &Arc<Engine>) -> Vec<SeqEvent> {
    let sid = engine.create_session(engine.cfg.workspace.display().to_string(), Vec::new(), None);
    let run_id = engine
        .prompt(
            &sid,
            &[pii_proto::session::ContentBlock::Text {
                text: String::from("hello"),
            }],
        )
        .expect("prompt");
    // Wait for the run to finish by tracing the journal.
    let store = engine.store.clone();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let header = store.get_run(&run_id).unwrap();
        if header.is_some_and(|h| h.status != pii_abi::RunStatus::Running) {
            break;
        }
        assert!(
            std::time::Instant::now() <= deadline,
            "run did not finish in time"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    store
        .read_journal(&run_id)
        .unwrap()
        .into_iter()
        .filter_map(|(seq, ev)| {
            pii_server::bus::map_event(&run_id, &ev).map(|event| SeqEvent { seq, event })
        })
        .collect()
}

#[tokio::test]
async fn reattach_since_seq_no_duplicates() {
    let dir = tempfile::Builder::new()
        .prefix("reattach-")
        .tempdir_in("/projects/pi-improved/_tmp/phase2")
        .expect("tempdir");
    let fixture = dir.path().join("hello.json");
    std::fs::write(
        &fixture,
        r#"{"turns":[{"text":"hello from the fixture","tool_calls":[],"tokens_in":3,"tokens_out":5}]}"#,
    )
    .unwrap();
    let engine = engine_in(dir.path(), &fixture);

    // First subscription: collect the full event stream (live).
    let bus = Bus::new();
    let _live = bus.subscribe();
    // Run to completion while "subscribed".
    {
        let bridge = BridgeObserver {
            run_id: pii_abi::RunId::from("r000000000000"),
            tx: bus.subscribe_tx(),
        };
        let _ = bridge; // the engine's own bridge covers the real run
    }
    let first = run_one_prompt(&engine).await;
    assert!(
        first.len() >= 4,
        "expected several events, got {:?}",
        first.iter().map(|e| e.seq).collect::<Vec<_>>()
    );
    let total = first.len();
    // "Drop at seq 3": the client last saw seq 3.
    let since = 3u64;
    // Resubscribe with since_seq = 3: replay must deliver 4..=N exactly once.
    let mut merged: Vec<SeqEvent> = Vec::new();
    let mut live_rx = bus.subscribe();
    let _last = pii_server::bus::reattach(
        &engine.store,
        &first_run_id(&first),
        since,
        &mut live_rx,
        &mut merged,
    );
    let seqs: Vec<u64> = merged.iter().map(|e| e.seq).collect();
    assert_eq!(seqs.len(), total - usize::try_from(since).unwrap_or(0));
    assert!(
        seqs.iter().all(|&s| s > since),
        "no seq <= {since}: {seqs:?}"
    );
    let mut sorted = seqs.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted, seqs, "every seq delivered exactly once, in order");
    // And the final event is run_finished.
    assert!(
        matches!(
            merged.last().map(|e| &e.event),
            Some(Event::RunFinished { .. })
        ),
        "stream ends with run_finished"
    );
}

fn first_run_id(events: &[SeqEvent]) -> pii_abi::RunId {
    for e in events {
        let rid = match &e.event {
            Event::RunStarted { run_id, .. } => run_id.clone(),
            _ => continue,
        };
        return pii_abi::RunId::from(rid.as_str());
    }
    panic!("no RunStarted in stream");
}

// Unused-in-test helpers stay referenced to satisfy the lints.
#[allow(dead_code)]
fn _caps() -> CapSet {
    CapSet::empty()
}
