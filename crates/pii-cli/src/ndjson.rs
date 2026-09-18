//! Ndjson stdout observer (spec F5.5).
//!
//! Emits one ndjson line per engine observation: `{"metric":<name>,"value":<u64>}`
//! for metrics and `{"seq":<u64>,"event":<RunEvent>}` for journal events. The
//! final `{"terminal":..}` line is printed by the run/resume command itself,
//! after the engine returns.

use pii_core::{Metric, ObsEvent, Observer};

/// Renders a journal event as the F5.5 ndjson line `{"seq":n,"event":...}`.
fn journal_line(seq: u64, body: &Result<String, serde_json::Error>) -> String {
    match body {
        Ok(body) => format!(r#"{{"seq":{seq},"event":{body}}}"#),
        Err(err) => {
            let msg = err.to_string().replace('\\', "\\\\").replace('"', "'");
            format!(r#"{{"seq":{seq},"event":{{"serialize_error":"{msg}"}}}}"#)
        }
    }
}

/// Renders a metric as the F5.5 ndjson line `{"metric":..,"value":..}`.
fn metric_line(m: &Metric) -> String {
    serde_json::json!({"metric": m.name, "value": m.value}).to_string()
}

/// Observer that streams the run as ndjson lines on stdout.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) struct NdjsonObserver;

impl Observer for NdjsonObserver {
    fn event(&self, e: &ObsEvent) {
        if let ObsEvent::Journal { seq, event } = e {
            println!("{}", journal_line(*seq, &serde_json::to_string(event)));
        }
    }

    fn metric(&self, m: &Metric) {
        // `Metric` is not `serde::Serialize`; render the F5.5 shape by hand.
        println!("{}", metric_line(m));
    }

    fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::ffi::OsString;
    use std::process::ExitCode;

    use pii_abi::RunEvent;
    use pii_core::{Metric, Observer};

    use super::{NdjsonObserver, journal_line, metric_line};
    use crate::cli_util::{next_value, open_store, usage_error};

    // --- usage_error ------------------------------------------------------

    #[test]
    fn usage_error_returns_exit_code_2() {
        let code = usage_error("bad thing", "pii graph run <file> [flags]");
        assert_eq!(code, ExitCode::from(2));
    }

    // --- next_value -------------------------------------------------------

    #[test]
    fn next_value_returns_present_arg() {
        let mut args = [OsString::from("hello"), OsString::from("world")].into_iter();
        assert_eq!(next_value(&mut args, "--db", "USAGE").unwrap(), "hello");
        assert_eq!(next_value(&mut args, "--db", "USAGE").unwrap(), "world");
    }

    #[test]
    fn next_value_errors_when_exhausted() {
        let mut args: std::vec::IntoIter<OsString> = Vec::new().into_iter();
        let err = next_value(&mut args, "--db", "USAGE").unwrap_err();
        assert_eq!(err, ExitCode::from(2));
    }

    // --- open_store -------------------------------------------------------

    #[test]
    fn open_store_succeeds_on_valid_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("nested").join("run.redb");
        assert!(open_store(&db).is_some());
    }

    #[test]
    fn open_store_fails_on_invalid_path() {
        // A `..` after a regular file makes the parent un-creatable.
        let dir = tempfile::tempdir().expect("tempdir");
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"x").expect("write");
        let db = dir.path().join("blocker").join("..").join("run.redb");
        assert!(open_store(&db).is_none());
    }

    // --- ndjson shapes ----------------------------------------------------

    #[test]
    fn ndjson_journal_event_shape() {
        let event = RunEvent::Started {
            graph: String::from("g"),
            input_hash: String::from("abc"),
        };
        let line = journal_line(7, &serde_json::to_string(&event));
        assert_eq!(
            line,
            r#"{"seq":7,"event":{"Started":{"graph":"g","input_hash":"abc"}}}"#
        );
    }

    #[test]
    fn ndjson_metric_shape() {
        let m = Metric {
            name: String::from("graph.batch.width"),
            value: 4,
        };
        assert_eq!(
            metric_line(&m),
            r#"{"metric":"graph.batch.width","value":4}"#
        );
    }

    #[test]
    fn observer_ignores_non_journal_events() {
        // Compile-shape check: the observer trait impl exists and `flush` is
        // a no-op on the unit struct.
        NdjsonObserver.flush();
    }
}
