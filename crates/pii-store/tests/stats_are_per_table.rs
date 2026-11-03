//! Spec 358 R1: the per-table row counts move with the table a row is written to.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_abi::{RunEvent, RunHeader, RunId, RunStatus};
use pii_store::Store;

#[test]
fn a_written_row_changes_its_tables_count() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "stats-").unwrap();
    let store = Store::open(&dir.path().join("store.redb")).unwrap();
    let before = store.info().unwrap().tables;
    let mut header = RunHeader {
        run: RunId::from("r000000000358"),
        graph: "g".to_string(),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 0,
        created_ms: 0,
    };
    let started = RunEvent::Started {
        graph: "g".to_string(),
        input_hash: "h".to_string(),
    };
    store.append_event(&mut header, &started).unwrap();
    let after = store.info().unwrap().tables;
    assert_eq!(after["run"], before["run"] + 1);
    assert_eq!(after["session"], before["session"]);
}
