//! Spec 159: the operator distance from recorded timestamps alone.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_learn::distance::{measure_distance, render};

fn store(lines: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("incidents.jsonl"), lines).unwrap();
    d
}

#[test]
fn an_incident_the_operator_acted_on_late_is_exact_to_the_second() {
    let d = store(concat!(
        r#"{"fingerprint":"a","first_seen":"2026-10-10T10:00:00Z","opened_at":"2026-10-10T10:12:34Z","count":3,"evidence":"x:1","operator_acts":["2026-10-10T11:00:00Z","2026-10-10T12:00:00Z"]}"#,
        "\n"
    ));
    let r = measure_distance(d.path(), 0);
    let i = &r.incidents[0];
    assert_eq!(i.h_pre, Some(false));
    assert_eq!(i.l_secs, Some(754));
    assert_eq!(i.acts, Some(2));
    assert!(render(&r).contains("median=754s"));
}

#[test]
fn an_act_before_the_card_is_h_pre() {
    let d = store(concat!(
        r#"{"fingerprint":"a","first_seen":"2026-10-10T10:00:00Z","opened_at":"2026-10-10T10:10:00Z","operator_acts":["2026-10-10T10:05:00Z"]}"#,
        "\n"
    ));
    assert_eq!(measure_distance(d.path(), 0).incidents[0].h_pre, Some(true));
}

#[test]
fn an_untouched_incident_has_no_act_before_the_card() {
    let d = store(concat!(
        r#"{"fingerprint":"a","first_seen":"2026-10-10T10:00:00Z","opened_at":"2026-10-10T10:10:00Z"}"#,
        "\n"
    ));
    let i = &measure_distance(d.path(), 0).incidents[0];
    assert_eq!(i.h_pre, Some(false));
    assert_eq!(i.acts, Some(0));
}

#[test]
fn an_incident_without_timestamps_is_named_and_kept_out_of_the_median() {
    let d = store(concat!(
        r#"{"fingerprint":"ok","first_seen":"2026-10-10T10:00:00Z","opened_at":"2026-10-10T10:01:00Z"}"#,
        "\n",
        r#"{"fingerprint":"bare"}"#,
        "\n"
    ));
    let r = measure_distance(d.path(), 0);
    let i = &r.incidents[1];
    assert_eq!((i.h_pre, i.l_secs, i.acts), (None, None, Some(0)));
    let out = render(&r);
    assert!(out.contains("incidents in the window: 2"));
    assert!(out.contains("L (seconds, first evidence to opened): computable 1/2 median=60s"));
    assert!(out.contains("NOT COMPUTABLE bare"));
}

#[test]
fn an_empty_store_says_so() {
    let d = store("");
    assert_eq!(
        render(&measure_distance(d.path(), 0)),
        "no incidents in the window\n"
    );
}

#[test]
fn since_excludes_older_incidents() {
    let d = store(concat!(
        r#"{"fingerprint":"old","first_seen":"2026-09-01T00:00:00Z","opened_at":"2026-09-01T00:01:00Z"}"#,
        "\n"
    ));
    let since = pii_learn::incident::parse_iso("2026-10-01T00:00:00Z").unwrap();
    assert!(measure_distance(d.path(), since).incidents.is_empty());
}

#[test]
fn an_act_recorded_in_the_operator_log_counts_and_an_untouched_incident_stays_zero() {
    let d = store(concat!(
        r#"{"fingerprint":"hot","first_seen":"2026-10-10T10:00:00Z","opened_at":"2026-10-10T10:10:00Z"}"#,
        "\n",
        r#"{"fingerprint":"cold","first_seen":"2026-10-10T10:00:00Z","opened_at":"2026-10-10T10:10:00Z"}"#,
        "\n"
    ));
    std::fs::write(
        d.path().join("operator-acts.log"),
        "2026-10-10T10:05:00Z | human | deploy-card | incident=hot rev=r1 approved\n\
         2026-10-10T11:00:00Z | human | pause | -\n",
    )
    .unwrap();
    let r = measure_distance(d.path(), 0);
    assert_eq!(
        (r.incidents[0].acts, r.incidents[0].h_pre),
        (Some(1), Some(true))
    );
    assert_eq!(
        (r.incidents[1].acts, r.incidents[1].h_pre),
        (Some(0), Some(false))
    );
}

#[test]
fn an_act_line_with_an_unreadable_timestamp_is_named_not_imputed() {
    let d = store(concat!(
        r#"{"fingerprint":"hot","first_seen":"2026-10-10T10:00:00Z","opened_at":"2026-10-10T10:10:00Z"}"#,
        "\n"
    ));
    std::fs::write(
        d.path().join("operator-acts.log"),
        "garbage | human | resume | hot\n",
    )
    .unwrap();
    let r = measure_distance(d.path(), 0);
    assert_eq!(r.incidents[0].acts, None);
    assert!(render(&r).contains("NOT COMPUTABLE hot"));
}
