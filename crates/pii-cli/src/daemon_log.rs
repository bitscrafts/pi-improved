//! `pii daemon`'s log and audit-trail plumbing: where the log file is, which
//! directories the sandbox must leave writable for it, the always-on audit
//! file, and the `daemon.sandbox.in_force` event.

use crate::cli_util::CliObserver;
use std::path::PathBuf;
use std::sync::Arc;

use pii_observe::{AUDIT_OFF_KINDS, AuditFilter, Event, Fanout, FileLog, Level, Observer, names};
use pii_server::Engine;
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

/// Parses an `on`/`off` switch value (spec 116). Anything else is an error that
/// names the switch: a typo must not silently leave a security surface off.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn parse_switch(name: &str, value: &str) -> Result<bool, String> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        other => Err(format!("{name} must be 'on' or 'off', got '{other}'")),
    }
}

/// A switch's effective state: flag > `[core]` > on (the default is ALWAYS on).
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn resolve_switch(
    name: &str,
    flag: Option<bool>,
    cfg: Option<&str>,
) -> Result<bool, String> {
    match (flag, cfg) {
        (Some(f), _) => Ok(f),
        (None, Some(v)) => parse_switch(name, v),
        (None, None) => Ok(true),
    }
}

/// The measurement switches in force (spec 116); both default on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) struct Switches {
    pub(crate) observer: bool,
    pub(crate) audit: bool,
}

/// Resolves both switches from the flags and `[core]`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn resolve_switches(
    observer: Option<bool>,
    audit: Option<bool>,
    cfg: &Config,
) -> Result<Switches, String> {
    Ok(Switches {
        observer: resolve_switch("observer", observer, cfg.core.observer.as_deref())?,
        audit: resolve_switch("audit", audit, cfg.core.audit.as_deref())?,
    })
}

/// One warn event per switched-off surface, stated at every start.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn switch_off_events(s: Switches) -> Vec<Event> {
    let off = |kind: &'static str, effect: &'static str| {
        Event::new("daemon", Level::Warn, kind)
            .field("effect", effect)
            .field("purpose", "measurement only; the default is on")
    };
    let mut out = Vec::new();
    if !s.audit {
        out.push(off(
            names::DAEMON_AUDIT_DISABLED,
            "denials are NOT recorded in the audit trail",
        ));
    }
    if !s.observer {
        out.push(off(
            names::DAEMON_OBSERVER_DETACHED,
            "tool calls, store/provider/net events and persist errors are NOT logged",
        ));
    }
    out
}

/// Attaches the daemon's observers to `engine` (specs 107, 108, 116): the
/// contract observer and the late-bound log unless the observer is switched
/// off, and the denial feed to the audit trail unless the audit is.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn wire_observers(engine: &Engine, log: &Arc<dyn Observer>, s: Switches) {
    if s.observer {
        engine.attach_observer(Arc::new(pii_core::ContractObserver::new(Arc::clone(log))));
        engine.attach_log(Arc::clone(log));
    } else {
        // Measurement only: runs carry no tool-call observer and the late-bound
        // log stays unattached (it drops, it does not queue).
        engine.detach_observers();
    }
    if s.audit {
        engine.attach_audit(Arc::new(pii_core::DenialObserver::new(Arc::clone(log))));
    }
}

/// The audit trail's file name, next to the store (durable state dir).
const AUDIT_FILE: &str = "audit.log";

/// The observer the daemon reports through: the rotating log file when one is
/// configured (flag > `[core]` > off), plus the audit trail — an always-on
/// `audit.log` next to the store that receives only the security-relevant
/// events ([`pii_observe::AUDIT_KINDS`]: denials, the sandbox profile in
/// force). With `audit_on == false` (spec 116, measurement only) the file
/// records only that the trail was switched off. A file that cannot be
/// written is reported once and never stops the daemon.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn open_log(
    flags: &LogFlags,
    cfg: &Config,
    audit_dir: &std::path::Path,
    audit_on: bool,
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
    let audit_file = file(audit_dir.join(AUDIT_FILE));
    let audit: Arc<dyn Observer> = Arc::new(if audit_on {
        AuditFilter::new(audit_file)
    } else {
        AuditFilter::with_kinds(audit_file, AUDIT_OFF_KINDS)
    });
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn scratch() -> tempfile::TempDir {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
        std::fs::create_dir_all(&root).unwrap();
        tempfile::Builder::new()
            .prefix("audit-")
            .tempdir_in(root)
            .unwrap()
    }

    #[test]
    fn a_switch_defaults_on_and_only_on_or_off_parse() {
        assert_eq!(resolve_switch("audit", None, None), Ok(true));
        assert_eq!(resolve_switch("audit", None, Some("off")), Ok(false));
        assert_eq!(resolve_switch("audit", Some(true), Some("off")), Ok(true));
        assert_eq!(resolve_switch("audit", Some(false), None), Ok(false));
        let e = resolve_switch("audit", None, Some("0")).unwrap_err();
        assert!(e.contains("audit") && e.contains("'0'"), "{e}");
    }

    #[test]
    fn switched_off_surfaces_are_each_announced_and_on_announces_nothing() {
        let on = Switches {
            observer: true,
            audit: true,
        };
        assert_eq!(switch_off_events(on), [] as [Event; 0]);
        let both = Switches {
            observer: false,
            audit: false,
        };
        let kinds: Vec<_> = switch_off_events(both).iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            [
                names::DAEMON_AUDIT_DISABLED,
                names::DAEMON_OBSERVER_DETACHED
            ]
        );
        let cfg: Config = Config::default();
        assert_eq!(resolve_switches(None, None, &cfg), Ok(on));
        assert_eq!(
            resolve_switches(Some(false), None, &cfg).map(|s| s.observer),
            Ok(false)
        );
    }

    #[test]
    fn audit_off_leaves_a_trail_that_says_so_and_nothing_else() {
        let dir = scratch();
        let log = open_log(&LogFlags::default(), &Config::default(), dir.path(), false);
        log.event(&Event::new("core", Level::Warn, names::CORE_CAP_DENIED));
        log.event(&Event::new(
            "daemon",
            Level::Warn,
            names::DAEMON_AUDIT_DISABLED,
        ));
        log.flush();
        let trail = std::fs::read_to_string(dir.path().join(AUDIT_FILE)).unwrap();
        assert!(trail.contains(names::DAEMON_AUDIT_DISABLED), "{trail}");
        assert!(!trail.contains(names::CORE_CAP_DENIED), "{trail}");
    }

    #[test]
    fn audit_on_records_denials() {
        let dir = scratch();
        let log = open_log(&LogFlags::default(), &Config::default(), dir.path(), true);
        log.event(&Event::new("core", Level::Warn, names::CORE_CAP_DENIED));
        log.flush();
        let trail = std::fs::read_to_string(dir.path().join(AUDIT_FILE)).unwrap();
        assert!(trail.contains(names::CORE_CAP_DENIED), "{trail}");
    }
}
