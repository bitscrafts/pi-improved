//! `pii store compact [--dir D]` (spec 115 R3b): offline compaction of the
//! daemon's redb files. Refuses while a daemon holds them. Prints the
//! MEASURED size of each file before and after; a store with nothing to
//! reclaim prints a (near-)equal pair, which is the truth.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::cmd_ctl::next_str;

/// `pii store compact [--dir D]`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_store(args: &mut std::env::ArgsOs) -> ExitCode {
    let mut dir = None;
    let mut sub = None;
    while let Some(a) = next_str(args) {
        match a.as_str() {
            "--dir" => dir = next_str(args).map(PathBuf::from),
            s if sub.is_none() && !s.starts_with("--") => sub = Some(s.to_owned()),
            _ => sub = None,
        }
    }
    let dir = Some(dir.unwrap_or_else(pii_core::state_dir));
    let (Some(sub), Some(dir)) = (sub.as_deref(), dir) else {
        let _ = writeln!(
            std::io::stderr(),
            "usage: pii store compact|stats [--dir <store dir>]   (compact: daemon must be stopped)"
        );
        return ExitCode::from(2);
    };
    if sub == "stats" {
        return print_stats(&dir);
    }
    if sub != "compact" {
        let _ = writeln!(
            std::io::stderr(),
            "usage: pii store compact|stats [--dir <store dir>]   (compact: daemon must be stopped)"
        );
        return ExitCode::from(2);
    }
    match pii_store::compact::compact_dir(&dir) {
        Ok(reports) => {
            let mut out = std::io::stdout();
            for (name, r) in reports {
                let _ = writeln!(
                    out,
                    "compacted {name} {} -> {} bytes",
                    r.bytes_before, r.bytes_after
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

/// `pii store stats [--dir D]` (spec 358 R1): one line per table with its row
/// count, then the store file's byte size.
fn print_stats(dir: &std::path::Path) -> ExitCode {
    let path = dir.join("store.redb");
    let info = match pii_store::Store::open(&path).and_then(|s| s.info()) {
        Ok(info) => info,
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "error: {e}");
            return ExitCode::from(2);
        }
    };
    let mut out = std::io::stdout();
    for (table, rows) in &info.tables {
        let _ = writeln!(out, "store.redb {table} rows={rows}");
    }
    let _ = writeln!(out, "store.redb bytes={}", info.bytes);
    ExitCode::SUCCESS
}
