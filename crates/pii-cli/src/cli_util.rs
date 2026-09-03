//! Small helpers shared by the `pii graph` subcommands (spec F5).

use std::path::Path;
use std::process::ExitCode;

use pii_store::Store;

/// Prints a usage error to stderr and returns the F5.3 usage exit code (2).
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn usage_error(msg: &str, usage: &str) -> ExitCode {
    eprintln!("error: {msg}");
    eprintln!("{usage}");
    ExitCode::from(2)
}

/// Reads the value of a flag that requires one (F5.2 flag grammar).
///
/// # Errors
/// Returns the usage exit code when the flag has no following value.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn next_value(
    args: &mut std::env::ArgsOs,
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
        eprintln!("error: mkdir {}: {e}", parent.display());
        return None;
    }
    match Store::open(db) {
        Ok(store) => Some(store),
        Err(e) => {
            eprintln!("error: open {}: {e}", db.display());
            None
        }
    }
}
