//! Spec 408: the event-log check and the current-labels check are separate, and each alone must hold. The fixtures here make the
//! issue's labels say `ready` while its event log says the label was removed, or re-applied by a stranger, so only the event log
//! can tell the issue is not approved.

#![allow(clippy::wildcard_imports)] // reason: the shared fake transport and fixtures live in the parent module

use super::*;

#[test]
fn an_issue_still_labelled_ready_whose_latest_event_removed_it_is_not_returned() {
    let fake = github(
        vec![vec![issue(7, "ann", &["ready"], "2026-11-10T12:00:00Z")]],
        vec![(7, vec![labeled(OWNER, "ready"), unlabeled(OWNER, "ready")])],
    );
    let ch = channel(&fake, &[]);
    let got = ch.poll(&ctx(), None).unwrap();
    assert!(
        got.is_empty(),
        "a removed label must not reach the queue even while the labels still say ready: {got:?}"
    );
    assert_eq!(ch.counters().unwrap().dropped, 1);
}

#[test]
fn an_issue_still_labelled_ready_whose_latest_event_is_a_strangers_is_not_returned() {
    let fake = github(
        vec![vec![issue(7, "ann", &["ready"], "2026-11-10T12:00:00Z")]],
        vec![(
            7,
            vec![
                labeled(OWNER, "ready"),
                unlabeled(OWNER, "ready"),
                labeled(STRANGER, "ready"),
            ],
        )],
    );
    let ch = channel(&fake, &[]);
    let got = ch.poll(&ctx(), None).unwrap();
    assert!(
        got.is_empty(),
        "a stranger's re-label must not reach the queue even while the labels still say ready: {got:?}"
    );
    assert_eq!(ch.counters().unwrap().dropped, 1);
}
