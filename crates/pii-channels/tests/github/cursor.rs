//! Spec 412: an issue is delivered once per `ready` label, not once per update. The cursor is the time the
//! owner applied `ready` (the event's `created_at`), not the issue's `updated_at`, which the channel's own
//! comment moves. The fixtures carry `created_at` as GitHub sends it; the landed tests are not edited.

#![allow(clippy::wildcard_imports)] // reason: the shared fake transport and fixtures live in the parent module

use super::*;

const T1_ISO: &str = "2026-11-10T10:00:00Z";
const T2_ISO: &str = "2026-11-10T11:00:00Z";
const T3_ISO: &str = "2026-11-10T12:00:00Z";
/// The same instants in epoch seconds.
const T1: u64 = 1_794_304_800;
const T2: u64 = 1_794_308_400;
const T3: u64 = 1_794_312_000;

fn labeled_at(actor: &str, label: &str, created_at: &str) -> Value {
    json!({
        "event": "labeled",
        "label": { "name": label },
        "actor": { "login": actor },
        "created_at": created_at,
    })
}

fn unlabeled_at(actor: &str, label: &str, created_at: &str) -> Value {
    json!({
        "event": "unlabeled",
        "label": { "name": label },
        "actor": { "login": actor },
        "created_at": created_at,
    })
}

#[test]
fn a_comment_on_a_delivered_issue_does_not_deliver_it_again() {
    // The owner applied `ready` at T1 and was delivered. The channel's own comment at T2 moved `updated_at`.
    let fake = github(
        vec![vec![issue(7, "ann", &["ready"], T2_ISO)]],
        vec![(7, vec![labeled_at(OWNER, "ready", T1_ISO)])],
    );
    let ch = channel(&fake, &[]);
    let got = ch.poll(&ctx(), Some(T1)).unwrap();
    assert!(
        got.is_empty(),
        "a comment must not deliver the issue again: {got:?}"
    );
    assert_eq!(ch.counters().unwrap().dropped, 0, "a skip is not a drop");
}

#[test]
fn CONTROL_the_owner_labelling_ready_again_delivers_it_again() {
    let fake = github(
        vec![vec![issue(7, "ann", &["ready"], T3_ISO)]],
        vec![(
            7,
            vec![
                labeled_at(OWNER, "ready", T1_ISO),
                unlabeled_at(OWNER, "ready", T2_ISO),
                labeled_at(OWNER, "ready", T3_ISO),
            ],
        )],
    );
    let got = channel(&fake, &[]).poll(&ctx(), Some(T1)).unwrap();
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].cursor, T3, "the cursor is the newest label time");
}

#[test]
fn the_cursor_is_the_label_time_not_updated_at() {
    let fake = github(
        vec![vec![issue(7, "ann", &["ready"], T2_ISO)]],
        vec![(7, vec![labeled_at(OWNER, "ready", T1_ISO)])],
    );
    let got = channel(&fake, &[]).poll(&ctx(), None).unwrap();
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(
        got[0].cursor, T1,
        "the cursor is the label time, not updated_at"
    );
}

#[test]
fn an_issue_not_updated_since_the_cursor_costs_no_events_call() {
    // `updated_at` equals the cursor: not newer, so the issue is skipped before its events are read.
    let fake = github(
        vec![vec![issue(7, "ann", &["ready"], T2_ISO)]],
        vec![(7, vec![labeled_at(OWNER, "ready", T1_ISO)])],
    );
    let got = channel(&fake, &[]).poll(&ctx(), Some(T2)).unwrap();
    assert!(got.is_empty(), "{got:?}");
    let events_asked = fake.seen().iter().any(|r| r.url.contains("/events"));
    assert!(
        !events_asked,
        "no events request for an issue not updated since the cursor"
    );
}

#[test]
fn issues_are_returned_oldest_label_first() {
    // Listed newest first: issue 2 was labelled at T2, issue 1 at T1.
    let fake = github(
        vec![vec![
            issue(2, "bob", &["ready"], T2_ISO),
            issue(1, "ann", &["ready"], T1_ISO),
        ]],
        vec![
            (2, vec![labeled_at(OWNER, "ready", T2_ISO)]),
            (1, vec![labeled_at(OWNER, "ready", T1_ISO)]),
        ],
    );
    let got = channel(&fake, &[]).poll(&ctx(), None).unwrap();
    let cursors: Vec<u64> = got.iter().map(|i| i.cursor).collect();
    assert_eq!(cursors, [T1, T2], "{got:?}");
}

#[test]
fn an_event_without_created_at_falls_back_to_updated_at() {
    // `labeled` with no `created_at` (the landed fixtures' shape): the issue's `updated_at` stands in.
    let fake = github(
        vec![vec![issue(7, "ann", &["ready"], T2_ISO)]],
        vec![(7, vec![labeled(OWNER, "ready")])],
    );
    let got = channel(&fake, &[]).poll(&ctx(), None).unwrap();
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].cursor, T2, "fallback: updated_at");
}
