//! `pii daemon`'s log and audit-trail plumbing: where the log file is, which
//! directories the sandbox must leave writable for it, the always-on audit
//! file, and the `daemon.sandbox.in_force` event.

use crate::cli_util::CliObserver;
use std::path::PathBuf;
use std::sync::Arc;

use pii_observe::{AuditFilter, Event, Fanout, FileLog, Level, Observer, names};
use pii_server::daemon::config::Config;
use pii_tools::sandbox::SandboxProfileName;

/// Daemon log settings from the command line; `None` falls through to `[core]`.
#[derive(Debug, Default, PartialEq, Eq)]
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) struct LogFlags {
    pub(crate) file: Option<PathBuf>,
    pub(crate) rotate_bytes: Option<u64>,
    pub(crate) keep_files: Option<usize>,
}

/// The log file's path: flag > `[core]` > off.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn log_path(flags: &LogFlags, cfg: &Config) -> Option<PathBuf> {
    flags
        .file
        .clone()
        .or_else(|| cfg.core.log_file.as_deref().map(PathBuf::from))
        .filter(|p| !p.as_os_str().is_empty())
}

/// The directories outside the store/runtime dirs the daemon writes to once
/// running: the log file's parent and the trajectory root. Created here (the
/// sandbox can only grant a path that exists) and returned for the grant.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn writable_dirs(flags: &LogFlags, cfg: &Config) -> Vec<PathBuf> {
    let traj = pii_core::env_var(pii_core::TRAJECTORY_DIR_ENV)
        .filter(|d| !d.is_empty())
        .or_else(|| cfg.core.trajectory_dir.clone().filter(|d| !d.is_empty()))
        .map(PathBuf::from);
    let log = log_path(flags, cfg)
        .and_then(|p| p.parent().map(PathBuf::from))
        .filter(|p| !p.as_os_str().is_empty());
    let dirs: Vec<PathBuf> = traj.into_iter().chain(log).collect();
    for d in &dirs {
        // reason: best-effort; a dir that cannot be made simply is not
        // granted, and the later write fails visibly through its own sink.
        let _ = std::fs::create_dir_all(d);
    }
    dirs
}

/// The audit trail's file name, next to the store (durable state dir).
const AUDIT_FILE: &str = "audit.log";

/// The observer the daemon reports through: the rotating log file when one is
/// configured (flag > `[core]` > off), plus the audit trail — an always-on
/// `audit.log` next to the store that receives only the security-relevant
/// events ([`pii_observe::AUDIT_KINDS`]: denials, the sandbox profile in
/// force). A file that cannot be written is reported once and never stops
/// the daemon.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn open_log(
    flags: &LogFlags,
    cfg: &Config,
    audit_dir: &std::path::Path,
) -> Arc<dyn Observer> {
    use pii_server::daemon::config::{DEFAULT_LOG_KEEP_FILES, DEFAULT_LOG_ROTATE_BYTES};
    let rotate = flags
        .rotate_bytes
        .or(cfg.core.log_rotate_bytes)
        .unwrap_or(DEFAULT_LOG_ROTATE_BYTES);
    let keep = flags
        .keep_files
        .or(cfg.core.log_keep_files)
        .unwrap_or(DEFAULT_LOG_KEEP_FILES);
    let file = |path: PathBuf| -> Arc<dyn Observer> {
        Arc::new(
            FileLog::new(path, rotate, keep)
                .with_scrubber(pii_core::secret_scrubber())
                .with_reporter(Arc::new(CliObserver)),
        )
    };
    let audit: Arc<dyn Observer> = Arc::new(AuditFilter::new(file(audit_dir.join(AUDIT_FILE))));
    let mut sinks = vec![audit];
    sinks.extend(log_path(flags, cfg).map(file));
    Arc::new(Fanout::new(sinks))
}

/// The `daemon.sandbox.in_force` event: profile, the exact grant, and whether
/// confinement is actually applied.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn sandbox_event(sandbox: SandboxProfileName) -> Event {
    let enforced = !matches!(sandbox, SandboxProfileName::Off);
    Event::new(
        "daemon",
        if enforced { Level::Info } else { Level::Warn },
        names::DAEMON_SANDBOX_IN_FORCE,
    )
    .field("profile", pii_tools::sandbox::profile_name(sandbox))
    .field("grant", pii_tools::sandbox::grant_policy(sandbox))
    .field("enforced", enforced)
}
