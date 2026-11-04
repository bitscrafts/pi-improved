//! Spec 371 R2: the promotion sweep is skipped while the promotions directory is unchanged, the skip is
//! counted in an event, and a changed directory is still swept (the near-miss control).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use pii_learn::{PromoteConfig, Promoter, SweepGate};
use pii_observe::{Recorder, names};

#[test]
fn an_unchanged_promotions_dir_is_not_swept_again() {
    let dir = tempfile::tempdir().unwrap();
    let rec = Arc::new(Recorder::new());
    let promoter = Promoter::new(dir.path(), PromoteConfig::default(), rec.clone());
    let mut gate = SweepGate::default();

    assert!(
        promoter.sweep_gated(&mut gate, 0).is_some(),
        "the first sweep is owed"
    );
    assert!(
        promoter.sweep_gated(&mut gate, 0).is_none(),
        "an unchanged directory is skipped"
    );
    assert!(promoter.sweep_gated(&mut gate, 0).is_none());

    let skips = rec.events_named(names::LEARN_SWEEP_SKIPPED);
    assert_eq!(skips.len(), 2, "every skip is counted: {:?}", rec.events());
}

#[test]
fn a_changed_promotions_dir_is_swept() {
    let dir = tempfile::tempdir().unwrap();
    let promotions = dir.path().join("learn").join("promotions");
    let promoter = Promoter::new(
        dir.path(),
        PromoteConfig::default(),
        Arc::new(Recorder::new()),
    );
    let mut gate = SweepGate::default();
    assert!(promoter.sweep_gated(&mut gate, 0).is_some());
    assert!(promoter.sweep_gated(&mut gate, 0).is_none());

    // A new entry changes the directory's mtime; the coarse filesystem clock needs a gap to show it.
    std::thread::sleep(Duration::from_millis(20));
    std::fs::create_dir_all(&promotions).unwrap();
    std::fs::write(promotions.join("p-0001.json"), b"{}").unwrap();

    assert!(
        promoter.sweep_gated(&mut gate, 0).is_some(),
        "a changed directory must be swept"
    );
}
