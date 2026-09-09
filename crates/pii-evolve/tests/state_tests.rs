//! R2: `state.md` round-trips and summarizes to a compact fragment.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_core::{Budget, CapSet, Ctx};
use pii_evolve::state::StateDoc;

fn ctx() -> Ctx {
    Ctx::root(CapSet::empty(), Budget::unlimited())
}

#[test]
fn state_doc_round_trips() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut doc = StateDoc {
        goal: String::from("ship spec 23"),
        phase: String::from("patch"),
        last_step: String::from("localize"),
        next_step: String::from("patch"),
        ..StateDoc::default()
    };
    doc.properties.insert(
        String::from("model_learning"),
        String::from("gpt-x underperforms on rust"),
    );
    let ctx = ctx();
    doc.touch(&ctx);
    let first_updated_at = doc.updated_at;
    doc.save(dir.path()).expect("save");

    let reloaded = StateDoc::load(dir.path()).expect("load");
    assert_eq!(reloaded, doc, "round trip must preserve every field");

    let mut doc2 = reloaded;
    doc2.touch(&ctx);
    assert!(
        doc2.updated_at > first_updated_at,
        "updated_at must be strictly monotonic across touches"
    );
}

#[test]
fn state_doc_summarizes_to_fragment() {
    let doc = StateDoc {
        goal: String::from("ship spec 23"),
        phase: String::from("patch"),
        last_step: String::from("localize"),
        next_step: String::from("write patch.rs"),
        ..StateDoc::default()
    };
    let summary = doc.summarize();
    assert!(
        summary.starts_with("Goal: ship spec 23"),
        "summary must start with a goal line: {summary}"
    );
    assert!(
        summary.contains("write patch.rs"),
        "summary must include next_step: {summary}"
    );
}
