//! Small helpers shared by the `pii graph` subcommands (spec F5).

use std::path::Path;
use std::process::ExitCode;

use pii_observe::{Event, Level, Observer, names};
use pii_store::Store;

/// The transport-failure events the CLI shows on stderr (spec 109 R7).
const TRANSPORT_FAILURES: &[&str] = &[
    names::PROTO_HANDSHAKE_TIMEOUT,
    names::PROTO_HANDSHAKE_CLOSED,
    names::PROTO_HANDSHAKE_SKEW,
    names::PROTO_HANDSHAKE_FAILED,
    names::UDS_CONNECT_FAILED,
];

/// The handshake bound: `$PII_CONNECT_TIMEOUT_MS` (milliseconds) when set and
/// valid, else [`pii_proto::client::DEFAULT_CONNECT_TIMEOUT`] (spec 109 R1).
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn connect_bound() -> std::time::Duration {
    pii_core::env_var("PII_CONNECT_TIMEOUT_MS")
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(
            pii_proto::client::DEFAULT_CONNECT_TIMEOUT,
            std::time::Duration::from_millis,
        )
}

/// Applies the sandbox profile (exit 4 when it cannot be enforced). The
/// trajectory root (spec 103) is written by the run, so it is granted
/// read-write; it must exist before the grant. Shared by `pii -p` and
/// `pii tui` (spec 109 R5) so both confine alike.
///
/// # Errors
/// The exit code to return when confinement cannot be applied.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn apply_sandbox_profile(
    profile: pii_tools::sandbox::SandboxProfileName,
    workspace: &Path,
) -> Result<(), ExitCode> {
    if matches!(profile, pii_tools::sandbox::SandboxProfileName::Off) {
        return Ok(());
    }
    let extra_rw: Vec<std::path::PathBuf> = pii_core::env_var(pii_core::TRAJECTORY_DIR_ENV)
        .filter(|d| !d.is_empty())
        .map(std::path::PathBuf::from)
        .inspect(|d| {
            // reason: best-effort; a dir that cannot be made is not granted.
            let _ = std::fs::create_dir_all(d);
        })
        .into_iter()
        .collect();
    pii_tools::sandbox::apply_workspace(workspace, &extra_rw, Path::new("/etc")).map_err(|e| {
        use std::io::Write as _;
        // reason: nowhere to report a failed stderr write.
        let _ = writeln!(std::io::stderr(), "{e}");
        ExitCode::from(4)
    })
}

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
        // Spec 109 R7: a transport failure names its endpoint and bound on the
        // terminal, not only in a log.
        if TRANSPORT_FAILURES.contains(&e.kind) {
            // reason: nowhere to report a failed stderr write.
            let _ = writeln!(
                std::io::stderr(),
                "{}",
                pii_observe::StderrObserver::render(e)
            );
        }
        // Spec 107 R1: an unconfined daemon is never silent.
        if e.kind == names::DAEMON_SANDBOX_IN_FORCE
            && e.fields.get("enforced") == Some(&serde_json::Value::Bool(false))
        {
            // reason: nowhere to report a failed stderr write.
            let _ = writeln!(
                std::io::stderr(),
                "warning: daemon is running UNCONFINED (--sandbox off): no filesystem confinement in force"
            );
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

/// Output through the std handles, not the raw print macros the observability lint bans (spec 98).
///
/// The macro form keeps every call site's format string EXACTLY as it was: a mechanical rewrite that turned
/// `println!` into a function call silently stopped interpolating braces, and a test caught it (2026-11-02).
#[macro_export]
macro_rules! say {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, $($arg)*);
    }};
}

/// One line on stderr, same seam. Every refusal in this module goes through here.
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let mut err = std::io::stderr().lock();
        let _ = writeln!(err, $($arg)*);
    }};
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
