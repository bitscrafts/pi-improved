//! Spec 101 R3: an attach failure reaches a `pii-observe` observer as
//! `tui.failure`, naming the stage.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_observe::{Recorder, names};

#[tokio::test]
async fn attach_failure_is_reported_by_name() {
    // The peer end is dropped at once: the hello handshake cannot complete.
    let (ours, theirs) = tokio::io::duplex(64);
    drop(theirs);
    let (r, w) = tokio::io::split(ours);
    let rec = Recorder::new();
    let res = pii_tui::run_tui_observed(r, w, &rec).await;
    assert!(res.is_err());
    let ev = rec.events_named(names::TUI_FAILURE);
    assert_eq!(ev.len(), 1, "{:?}", rec.events());
    assert_eq!(
        ev[0].fields.get("stage"),
        Some(&serde_json::json!("attach"))
    );
}
