//! `pii backup <dest> [--dir D]` and `pii restore <src> [--dir D] [--force]`
//! (spec 105 R3). Offline: both need the daemon stopped, because the stores
//! are copied while holding redb's exclusive lock (see `pii_store::backup`).
//! `--dir` defaults to the runtime directory the daemon keeps its stores in.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use pii_store::backup::{Manifest, backup, restore};

use crate::cmd_ctl::next_str;

struct Flags {
    path: PathBuf,
    dir: PathBuf,
    force: bool,
}

fn parse(args: &mut std::env::ArgsOs, allow_force: bool) -> Option<Flags> {
    let mut path = None;
    let mut dir = None;
    let mut force = false;
    while let Some(a) = next_str(args) {
        match a.as_str() {
            "--dir" => dir = Some(PathBuf::from(next_str(args)?)),
            "--force" if allow_force => force = true,
            p if !p.starts_with("--") && path.is_none() => path = Some(PathBuf::from(p)),
            _ => return None,
        }
    }
    let dir = dir.or_else(|| pii_uds::runtime_dir("default"))?;
    Some(Flags {
        path: path?,
        dir,
        force,
    })
}

fn report(verb: &str, m: &Manifest) {
    let mut out = std::io::stdout();
    for e in &m.entries {
        let _ = writeln!(
            out,
            "{verb} {} ({} bytes, format_version={}, blake3={})",
            e.name, e.bytes, e.format_version, e.blake3
        );
    }
}

fn finish(r: Result<Manifest, pii_abi::PiiError>, verb: &str) -> ExitCode {
    match r {
        Ok(m) => {
            report(verb, &m);
            ExitCode::SUCCESS
        }
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "error: {e}");
            ExitCode::from(2)
        }
    }
}

/// `pii backup <dest> [--dir D]`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_backup(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(f) = parse(args, false) else {
        let _ = writeln!(
            std::io::stderr(),
            "usage: pii backup <dest> [--dir <store dir>]"
        );
        return ExitCode::from(2);
    };
    finish(backup(&f.dir, &f.path), "backed up")
}

/// `pii restore <src> [--dir D] [--force]`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_restore(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(f) = parse(args, true) else {
        let _ = writeln!(
            std::io::stderr(),
            "usage: pii restore <backup dir> [--dir <store dir>] [--force]"
        );
        return ExitCode::from(2);
    };
    finish(restore(&f.path, &f.dir, f.force), "restored")
}
