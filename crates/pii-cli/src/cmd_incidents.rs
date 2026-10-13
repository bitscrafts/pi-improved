//! `pii incidents scan --root <dir> [--new]` (spec 150): open incidents from
//! the system's own records. Offline, no daemon, no model. Prints one line per
//! incident (`--new`: only those opened by this scan, so a caller can send one
//! notification each and stay silent otherwise).

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use pii_learn::incident::{IncidentConfig, scan_incidents};

use crate::cmd_ctl::next_str;

/// `pii incidents scan --root <dir> [--new]`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_incidents(args: &mut std::env::ArgsOs) -> ExitCode {
    let mut root = None;
    let mut only_new = false;
    let mut sub = None;
    while let Some(a) = next_str(args) {
        match a.as_str() {
            "--root" => root = next_str(args).map(PathBuf::from),
            "--new" => only_new = true,
            s if sub.is_none() && !s.starts_with("--") => sub = Some(s.to_owned()),
            _ => sub = None,
        }
    }
    let (Some("scan"), Some(root)) = (sub.as_deref(), root) else {
        let _ = writeln!(
            std::io::stderr(),
            "usage: pii incidents scan --root <dir> [--new]"
        );
        return ExitCode::from(2);
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
