//! Small helpers shared by the `pii graph` subcommands (spec F5).

use std::path::Path;
use std::process::ExitCode;

use pii_observe::{Event, Level, Observer, names};
use pii_store::Store;

/// The CLI's observer: renders each [`names::CLI_FAILURE`] as the historical
/// `error: <message>` line on stderr, so operator-visible text is unchanged.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) struct CliObserver;

impl Observer for CliObserver {
    fn event(&self, e: &Event) {
        use std::io::Write as _;
        if e.kind == names::CLI_FAILURE
            && let Some(serde_json::Value::String(m)) = e.fields.get("message")
        {
            // reason: nowhere to report a failed stderr write.
            let _ = writeln!(std::io::stderr(), "error: {m}");
        }
    }
}

/// Reports an operator-facing failure at `stage` to `observer` as
/// [`names::CLI_FAILURE`]. The message must not carry secrets or env values.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn fail_to(observer: &dyn Observer, stage: &'static str, message: &str) {
    observer.event(
        &Event::new("cli", Level::Error, names::CLI_FAILURE)
            .field("stage", stage)
            .field("message", message),
    );
}

/// [`fail_to`] with the process's [`CliObserver`].
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn fail(stage: &'static str, message: impl std::fmt::Display) {
    fail_to(&CliObserver, stage, &message.to_string());
}

/// Prints a usage error to stderr and returns the F5.3 usage exit code (2).
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn usage_error(msg: &str, usage: &str) -> ExitCode {
    fail("usage", msg);
    eprintln!("{usage}");
    ExitCode::from(2)
}

/// Reads the value of a flag that requires one (F5.2 flag grammar).
///
/// # Errors
/// Returns the usage exit code when the flag has no following value.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn next_value(
    args: &mut dyn Iterator<Item = std::ffi::OsString>,
    flag: &str,
    usage: &str,
) -> Result<String, ExitCode> {
    args.next()
        .and_then(|a| a.into_string().ok())
        .ok_or_else(|| usage_error(&format!("{flag} needs a value"), usage))
}

/// Opens the redb database at `db`, creating parent directories.
///
/// Returns `None` (after printing the error) when the store cannot open.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn open_store(db: &Path) -> Option<Store> {
    if let Some(parent) = db.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        fail("store", format!("mkdir {}: {e}", parent.display()));
        return None;
    }
    match Store::open(db) {
        Ok(store) => Some(store),
        Err(e) => {
            fail("store", format!("open {}: {e}", db.display()));
            None
        }
    }
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

    use super::fail_to;

    /// Spec 101 R3: an operator-facing failure reaches the observer as
    /// `cli.failure`, carrying its stage and message.
    #[test]
    fn failure_is_reported_as_cli_failure() {
        let rec = Recorder::new();
        fail_to(&rec, "graph", "read x.toml: gone");
        let ev = rec.events_named(names::CLI_FAILURE);
        assert_eq!(ev.len(), 1, "{:?}", rec.events());
        assert_eq!(ev[0].fields.get("stage"), Some(&serde_json::json!("graph")));
        assert_eq!(
            ev[0].fields.get("message"),
            Some(&serde_json::json!("read x.toml: gone"))
        );
    }
}
