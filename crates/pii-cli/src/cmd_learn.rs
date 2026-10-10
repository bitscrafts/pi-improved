//! `pii learn {scan|list|show}` (spec 123 R8): offline extraction of candidate
//! lessons from the trajectory directory. It runs in this process, never in
//! the daemon, and only proposes: nothing here changes a config or a graph.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use pii_learn::{LearnConfig, Learner, store};
use pii_observe::StderrObserver;

use crate::cmd_ctl::next_str;

const USAGE: &str = "usage: pii learn scan [--traj <root>] [--dir <store dir>] [--min-runs N] \
[--outlier-factor N] [--max-files N] [--max-file-bytes N]\n       pii learn list [--dir <store dir>]\n       \
pii learn show <id> [--dir <store dir>]\n--traj defaults to $PII_TRAJECTORY_DIR; --dir to the runtime dir.";

struct Opts {
    traj: Option<PathBuf>,
    dir: Option<PathBuf>,
    cfg: LearnConfig,
    positional: Option<String>,
}

fn parse(args: &mut std::env::ArgsOs) -> Option<Opts> {
    let mut o = Opts {
        traj: pii_core::env_var(pii_core::TRAJECTORY_DIR_ENV).map(PathBuf::from),
        dir: pii_uds::runtime_dir("default"),
        cfg: LearnConfig::default(),
        positional: None,
    };
    while let Some(a) = next_str(args) {
        match a.as_str() {
            "--traj" => o.traj = Some(PathBuf::from(next_str(args)?)),
            "--dir" => o.dir = Some(PathBuf::from(next_str(args)?)),
            "--min-runs" => o.cfg.min_runs = next_str(args)?.parse().ok()?,
            "--outlier-factor" => o.cfg.outlier_factor = next_str(args)?.parse().ok()?,
            "--max-files" => o.cfg.max_files = next_str(args)?.parse().ok()?,
            "--max-file-bytes" => o.cfg.max_file_bytes = next_str(args)?.parse().ok()?,
            s if !s.starts_with("--") && o.positional.is_none() => {
                o.positional = Some(s.to_owned());
            }
            _ => return None,
        }
    }
    Some(o)
}

fn fail(msg: &str) -> ExitCode {
    let _ = writeln!(std::io::stderr(), "error: {msg}");
    ExitCode::from(2)
}

/// `pii learn ...`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_learn(args: &mut std::env::ArgsOs) -> ExitCode {
    let sub = next_str(args);
    let Some(o) = parse(args) else {
        let _ = writeln!(std::io::stderr(), "{USAGE}");
        return ExitCode::from(2);
    };
    let Some(dir) = o.dir.clone() else {
        return fail("no store directory (use --dir)");
    };
    let mut out = std::io::stdout();
    match sub.as_deref() {
        Some("scan") => {
            let Some(traj) = o.traj else {
                return fail("no trajectory root (use --traj or set PII_TRAJECTORY_DIR)");
            };
            match Learner::new(o.cfg, Arc::new(StderrObserver)).run_scan(&traj, &dir) {
                Ok(s) => {
                    let _ = writeln!(
                        out,
                        "scanned {} of {} files ({} bytes, {} ms); candidates {:?}; refused {:?}; skipped {:?}",
                        s.scan.files_scanned,
                        s.scan.files_seen,
                        s.scan.bytes_read,
                        s.scan.duration_ms,
                        s.candidates_by_kind,
                        s.refused_by_reason,
                        s.scan.skipped
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e),
            }
        }
        Some("list") => {
            let (list, skipped) = store::list(&dir);
            for c in &list {
                let _ = writeln!(out, "{}  {}  {}", c.id, c.kind.as_str(), c.claim);
            }
            let _ = writeln!(
                out,
                "{} candidates ({skipped} unreadable entries skipped)",
                list.len()
            );
            ExitCode::SUCCESS
        }
        Some("show") => {
            let Some(id) = o.positional else {
                let _ = writeln!(std::io::stderr(), "{USAGE}");
                return ExitCode::from(2);
            };
            match store::show(&dir, &id) {
                Ok(c) => {
                    let root = store::read_status(&dir)
                        .ok()
                        .flatten()
                        .map_or_else(|| "?".to_owned(), |s| s.root);
                    let _ = writeln!(
                        out,
                        "id: {}\nkind: {}\nclaim: {}\ndedupe_key: {}\nroot: {root}",
                        c.id,
                        c.kind.as_str(),
                        c.claim,
                        c.dedupe_key
                    );
                    let _ = writeln!(out, "counts: {:?}\nevidence:", c.evidence.counts);
                    for (r, p) in c.evidence.runs.iter().zip(&c.evidence.paths) {
                        let _ = writeln!(out, "  run {r}  {root}/{p}");
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e.to_string()),
            }
        }
        _ => {
            let _ = writeln!(std::io::stderr(), "{USAGE}");
            ExitCode::from(2)
        }
    }
}
