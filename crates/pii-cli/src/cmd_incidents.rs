//! `pii incidents scan --root <dir> [--new]` (spec 150): open incidents from
//! the system's own records. Offline, no daemon, no model. Prints one line per
//! incident (`--new`: only those opened by this scan, so a caller can send one
//! notification each and stay silent otherwise).

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use pii_learn::distance::{measure_distance, render};
use pii_learn::incident::{IncidentConfig, iso, parse_iso, scan_incidents};
use pii_learn::incident_runtime::scan_runtime_log;

use crate::cmd_ctl::next_str;

/// `pii incidents scan --root <dir> [--new]`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_incidents(args: &mut std::env::ArgsOs) -> ExitCode {
    let mut root = None;
    let mut only_new = false;
    let mut sub = None;
    let mut source = None;
    let mut log = None;
    let mut since = None;
    while let Some(a) = next_str(args) {
        match a.as_str() {
            "--root" => root = next_str(args).map(PathBuf::from),
            "--new" => only_new = true,
            "--source" => source = next_str(args),
            "--since" => since = next_str(args),
            "--log" => log = next_str(args).map(PathBuf::from),
            s if sub.is_none() && !s.starts_with("--") => sub = Some(s.to_owned()),
            _ => sub = None,
        }
    }
    if sub.as_deref() == Some("distance") {
        return distance(root, since.as_deref());
    }
    if sub.as_deref() == Some("scan") && source.as_deref() == Some("runtime") {
        return log.map_or_else(usage, |log| scan_runtime(&log));
    }
    let (Some("scan"), Some(root)) = (sub.as_deref(), root) else {
        return usage();
    };
    match scan_incidents(&root, &IncidentConfig::load(&root)) {
        Ok(found) => {
            let mut out = std::io::stdout();
            for (i, is_new) in found.iter().filter(|(_, n)| *n || !only_new) {
                let _ = writeln!(
                    out,
                    "incident {} count={} first_seen={} opened_at={} evidence={}{}",
                    i.fingerprint,
                    i.count,
                    i.first_seen,
                    i.opened_at,
                    i.evidence,
                    if *is_new { " NEW" } else { "" }
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "error: {e}");
            ExitCode::from(2)
        }
    }
}

/// `pii incidents distance --since <date> [--root <dir>]` (spec 159): the
/// operator distance from recorded timestamps, read-only.
fn distance(root: Option<PathBuf>, since: Option<&str>) -> ExitCode {
    let Some(since) = since.and_then(|s| parse_iso(&format!("{s}T00:00:00Z"))) else {
        return usage();
    };
    let root = root.unwrap_or_else(|| PathBuf::from("_tmp/autonomy/logs"));
    let _ = write!(
        std::io::stdout(),
        "{}",
        render(&measure_distance(&root, since))
    );
    ExitCode::SUCCESS
}

fn usage() -> ExitCode {
    let _ = writeln!(
        std::io::stderr(),
        "usage: pii incidents scan --root <dir> [--new] | pii incidents scan --source runtime --log <file> | pii incidents distance --since <date> [--root <dir>]"
    );
    ExitCode::from(2)
}

/// `--source runtime --log <file>` (spec 156): the daemon's own runtime log,
/// read-only, with the detection latency against the configured window.
fn scan_runtime(log: &std::path::Path) -> ExitCode {
    let cfg = IncidentConfig::default();
    match scan_runtime_log(log, &cfg) {
        Ok(found) => {
            let mut out = std::io::stdout();
            for i in &found {
                let _ = writeln!(
                    out,
                    "incident {} count={} first_seen={} last_seen={} latency_secs={} window_secs={} source={}",
                    i.fingerprint,
                    i.count,
                    iso(i.first_seen),
                    iso(i.last_seen),
                    i.latency_secs(),
                    cfg.window_secs,
                    i.source
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "error: {e}");
            ExitCode::from(2)
        }
    }
}
