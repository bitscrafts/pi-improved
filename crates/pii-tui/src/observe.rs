//! The observability seam of the TUI driver: failures by stage, and gaps in
//! the event stream.

use pii_observe::{Event, Level, Observer, names};
use tokio::sync::broadcast::error::RecvError;

use crate::state::TuiError;

/// Reports a failure at `stage` (`attach`, `render` or `input`) as
/// [`names::TUI_FAILURE`].
#[allow(clippy::redundant_pub_crate)] // reason: conflicts with rustc `unreachable_pub`.
pub(crate) fn failure(observer: &dyn Observer, stage: &'static str, e: &TuiError) {
    observer.event(
        &Event::new("tui", Level::Error, names::TUI_FAILURE)
            .field("stage", stage)
            .field("error", e.to_string()),
    );
}

/// Reports a broken event subscription as [`names::TUI_REATTACH_GAP`]:
/// `lagged` carries how many events were skipped (0 when the stream closed).
#[allow(clippy::redundant_pub_crate)] // reason: conflicts with rustc `unreachable_pub`.
pub(crate) fn gap(observer: &dyn Observer, e: &RecvError) {
    let (reason, missed) = match e {
        RecvError::Lagged(n) => ("lagged", *n),
        RecvError::Closed => ("closed", 0),
    };
    observer.event(
        &Event::new("tui", Level::Warn, names::TUI_REATTACH_GAP)
            .field("reason", reason)
            .field("missed", missed),
    );
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use pii_observe::{Recorder, names};

    use super::*;

    #[test]
    fn gap_reports_skipped_events_by_name() {
        let rec = Recorder::new();
        gap(&rec, &RecvError::Lagged(7));
        gap(&rec, &RecvError::Closed);
        let ev = rec.events_named(names::TUI_REATTACH_GAP);
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].fields.get("missed"), Some(&serde_json::json!(7)));
        assert_eq!(
            ev[1].fields.get("reason"),
            Some(&serde_json::json!("closed"))
        );
    }
}
