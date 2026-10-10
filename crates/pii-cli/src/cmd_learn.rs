//! `pii learn {scan|list|show|verify}` (specs 123 R8, 124 R8): offline
//! extraction of candidate lessons from the trajectory directory, and their
//! verification by isolated shadow runs. It runs in this process, never in the
//! daemon, and only proposes or decides: nothing here changes a config or a graph.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use pii_learn::{LearnConfig, Learner, Verdict, Verifier, VerifyConfig, get_verdict, store};
use pii_observe::StderrObserver;

use crate::cmd_ctl::next_str;

const USAGE: &str = "usage: pii learn scan [--traj <root>] [--dir <store dir>] [--min-runs N] \
[--outlier-factor N] [--max-files N] [--max-file-bytes N]\n       pii learn list [--dir <store dir>]\n       \
pii learn show <id> [--dir <store dir>]\n       \
pii learn verify {<id>|--all} [--traj <root>] [--dir <store dir>] [--graphs <dir>] [--max-runs N] [--run-ms N] \
[--input <json>] [--scratch <dir>] [--protect <path>]...\n--traj defaults to $PII_TRAJECTORY_DIR; --dir to the runtime dir; \
--graphs to <dir>/graphs; --max-runs 5; --run-ms 30000; --scratch to the system temp dir.";

struct Opts {
    traj: Option<PathBuf>,
    dir: Option<PathBuf>,
    cfg: LearnConfig,
    positional: Option<String>,
    all: bool,
    graphs: Option<PathBuf>,
    scratch: Option<PathBuf>,
    protect: Vec<PathBuf>,
    input: serde_json::Value,
    max_runs: u32,
    run_ms: u64,
}

fn parse(args: &mut std::env::ArgsOs) -> Option<Opts> {
    let mut o = Opts {
        traj: pii_core::env_var(pii_core::TRAJECTORY_DIR_ENV).map(PathBuf::from),
        dir: pii_uds::runtime_dir("default"),
        cfg: LearnConfig::default(),
        positional: None,
        all: false,
        graphs: None,
        scratch: None,
        protect: Vec::new(),
        input: serde_json::Value::Null,
        max_runs: pii_learn::DEFAULT_MAX_SHADOW_RUNS,
        run_ms: pii_learn::DEFAULT_SHADOW_RUN_MS,
    };
    while let Some(a) = next_str(args) {
        match a.as_str() {
            "--traj" => o.traj = Some(PathBuf::from(next_str(args)?)),
            "--dir" => o.dir = Some(PathBuf::from(next_str(args)?)),
            "--min-runs" => o.cfg.min_runs = next_str(args)?.parse().ok()?,
            "--outlier-factor" => o.cfg.outlier_factor = next_str(args)?.parse().ok()?,
            "--max-files" => o.cfg.max_files = next_str(args)?.parse().ok()?,
            "--max-file-bytes" => o.cfg.max_file_bytes = next_str(args)?.parse().ok()?,
            "--all" => o.all = true,
            "--graphs" => o.graphs = Some(PathBuf::from(next_str(args)?)),
            "--scratch" => o.scratch = Some(PathBuf::from(next_str(args)?)),
            "--protect" => o.protect.push(PathBuf::from(next_str(args)?)),
            "--max-runs" => o.max_runs = next_str(args)?.parse().ok()?,
            "--run-ms" => o.run_ms = next_str(args)?.parse().ok()?,
            "--input" => o.input = serde_json::from_str(&next_str(args)?).ok()?,
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
// reason: one dispatch arm per subcommand; splitting it would only hide the list.
#[allow(clippy::too_many_lines)]
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
                    if let Some(v) = get_verdict(&dir, &c.id) {
                        let why = v
                            .reason
                            .map_or(String::new(), |r| format!(" ({})", r.as_str()));
                        let _ = writeln!(
                            out,
                            "verdict: {}{why}\n  {}\n  shadow: {}\n  shadow runs: {:?}\n  numbers: {:?}",
                            v.verdict.as_str(),
                            v.detail,
                            v.shadow,
                            v.shadow_runs,
                            v.numbers
                        );
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e.to_string()),
            }
        }
        Some("verify") => verify(&o, &dir, &mut out),
        _ => {
            let _ = writeln!(std::io::stderr(), "{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// `pii learn verify <id>|--all`: one line per candidate, verdict then shadow run.
fn verify(o: &Opts, dir: &std::path::Path, out: &mut impl std::io::Write) -> ExitCode {
    let Some(traj) = o.traj.clone() else {
        return fail("no trajectory root (use --traj or set PII_TRAJECTORY_DIR)");
    };
    let cs = match (&o.positional, o.all) {
        (Some(id), false) => match store::show(dir, id) {
            Ok(c) => vec![c],
            Err(e) => return fail(&e.to_string()),
        },
        (None, true) => store::list(dir).0,
        _ => {
            let _ = writeln!(std::io::stderr(), "{USAGE}");
            return ExitCode::from(2);
        }
    };
    let graphs = o.graphs.clone().unwrap_or_else(|| dir.join("graphs"));
    let mut cfg = VerifyConfig::new(traj, graphs);
    cfg.max_shadow_runs = o.max_runs;
    cfg.shadow_run_ms = o.run_ms;
    cfg.input = o.input.clone();
    if let Some(s) = &o.scratch {
        cfg.scratch_dir.clone_from(s);
    }
    cfg.protected.clone_from(&o.protect);
    cfg.protected.push(dir.to_path_buf());
    let verifier = Verifier::new(cfg, Arc::new(StderrObserver));
    let rs = match verifier.verify_all(dir, &cs) {
        Ok(rs) => rs,
        Err(e) => return fail(&e),
    };
    for r in &rs {
        let why = r
            .reason
            .map_or(String::new(), |x| format!(" ({})", x.as_str()));
        let shadow = if r.shadow.is_empty() {
            "-"
        } else {
            r.shadow.as_str()
        };
        let _ = writeln!(
            out,
            "{}  {}{why}  shadow={shadow} runs={:?}  {}",
            r.candidate,
            r.verdict.as_str(),
            r.shadow_runs,
            r.detail
        );
    }
    let refuted = rs.iter().filter(|r| r.verdict == Verdict::Refuted).count();
    let _ = writeln!(out, "{} verified ({refuted} refuted)", rs.len());
    ExitCode::SUCCESS
}
