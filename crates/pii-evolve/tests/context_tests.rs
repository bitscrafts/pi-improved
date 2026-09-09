//! R3: compaction threshold decisions and decision-only summaries.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_evolve::context::{CompactionPolicy, TranscriptEntry, summarize_for_compaction};

#[test]
fn compaction_small_window_does_not_fire() {
    let policy = CompactionPolicy {
        window_tokens: 100_000,
        threshold: 0.72,
    };
    assert!(
        !policy.should_compact(40_000),
        "40% of window must not trigger compaction"
    );
}

#[test]
fn compaction_large_window_fires() {
    let policy = CompactionPolicy {
        window_tokens: 100_000,
        threshold: 0.72,
    };
    assert!(
        policy.should_compact(80_000),
        "80% of window must trigger compaction (threshold ~0.70-0.75)"
    );
}

#[test]
fn summary_keeps_decisions_omits_dialogue() {
    let entries = vec![
        TranscriptEntry::Dialogue(String::from(
            "the model rambled about unrelated topics for a while",
        )),
        TranscriptEntry::Decision {
            decision: String::from("route review to a different model than patch"),
            outcome: String::from("caught a bug the patch model missed"),
        },
        TranscriptEntry::Dialogue(String::from("more back and forth dialogue")),
    ];
    let summary = summarize_for_compaction(&entries);
    assert!(
        summary.contains(
            "route review to a different model than patch -> caught a bug the patch model missed"
        ),
        "summary must keep the decision and its outcome: {summary}"
    );
    assert!(
        !summary.contains("rambled"),
        "summary must omit dialogue verbatim: {summary}"
    );
    assert!(
        !summary.contains("back and forth"),
        "summary must omit dialogue verbatim: {summary}"
    );
}
