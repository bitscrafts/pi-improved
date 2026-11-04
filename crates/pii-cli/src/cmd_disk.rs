//! `pii disk [--root <repo>]` (spec 367 R1): one table of the artefact roots this repository fills, with bytes,
//! age and the filesystem each lives on, then free space for `/` and for the volume that holds the repository.
//! The floor is judged where the artefacts land (R4), so both lines are printed. Read-only: removal is
//! `cargo xtask disk --gc` (R3), which owns the TTL.

// reason: a diagnostic report reads the wall clock for artefact ages and shells out to `du` and `df`; it is not
// replayed, so the replay-only bans on `SystemTime::now` and `Command::output` do not apply.
#![allow(clippy::disallowed_methods)]

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::SystemTime;

/// Artefact roots inside the repository (spec 367 R1).
const REPO_ROOTS: &[&str] = &[
    "_tmp/gate-target",
    "_tmp/gate-target-fresh",
    "_tmp/targets",
    "_tmp/_trash",
];

/// Artefact roots outside the repository: gate exports, trash, coverage caches.
const EXTERNAL_ROOTS: &[&str] = &[
    "/projects/_tmp",
    "/projects/_trash",
    "/tmp/_trash-podman",
    "/tmp/gate-cache",
];

/// One artefact root: where it is, its size, its age, and whether a known owner put it there.
struct Row {
    path: PathBuf,
    bytes: u64,
    age_days: u64,
    fs: String,
    owned: bool,
}

#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_disk(args: &mut std::env::ArgsOs) -> ExitCode {
    let mut root = None;
    while let Some(arg) = args.next() {
        if arg == "--root" {
            root = args.next().map(PathBuf::from);
        } else {
            warn!("usage: pii disk [--root <repo>]");
            return ExitCode::from(2);
        }
    }
    let repo = match root.map_or_else(std::env::current_dir, Ok) {
        Ok(p) => p,
        Err(e) => {
            warn!("pii disk: cannot read the current directory: {e}");
            return ExitCode::from(2);
        }
    };
    let text = render(&repo);
    let mut out = std::io::stdout().lock();
    if out.write_all(text.as_bytes()).is_err() {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn render(repo: &Path) -> String {
    let now = SystemTime::now();
    let mut text = String::new();
    let _ = writeln!(text, "artefacts (bytes, age, filesystem, owner, path)");
    let mut total = 0_u64;
    for (path, owned) in artefacts(repo) {
        let Some(bytes) = du_bytes(&path) else {
            continue;
        };
        let row = Row {
            age_days: age_days(&path, now),
            fs: df(&path).map_or_else(|| "?".to_owned(), |(mount, _)| mount),
            path,
            bytes,
            owned,
        };
        total += row.bytes;
        let owner = if row.owned { "owned" } else { "unowned" };
        let _ = writeln!(
            text,
            "{:>10}  {:>4}d  {:<10}  {:<8}  {}",
            gib(row.bytes),
            row.age_days,
            row.fs,
            owner,
            row.path.display()
        );
    }
    let _ = writeln!(text, "total {}", gib(total));
    let root_free = df(Path::new("/"));
    let repo_free = df(repo);
    let _ = writeln!(text, "{}", free_line("filesystem /", root_free));
    let _ = writeln!(text, "{}", free_line("filesystem (repository)", repo_free));
    text
}

/// Every artefact root that exists, and each `_tmp/*target*` directory no owner claims (spec 367 R3).
fn artefacts(repo: &Path) -> Vec<(PathBuf, bool)> {
    let mut found: Vec<(PathBuf, bool)> = REPO_ROOTS
        .iter()
        .map(|r| (repo.join(r), true))
        .chain(EXTERNAL_ROOTS.iter().map(|r| (PathBuf::from(r), true)))
        .filter(|(p, _)| p.exists())
        .collect();
    if let Ok(rd) = std::fs::read_dir(repo.join("_tmp")) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let claimed = name == "targets" || name.starts_with("gate-target");
            if name.contains("target") && !claimed && entry.path().is_dir() {
                found.push((entry.path(), false));
            }
        }
    }
    found
}

fn free_line(label: &str, free: Option<(String, u64)>) -> String {
    match free {
        Some((mount, mb)) => format!("{label}: {mb} MB free on {mount}"),
        None => format!("{label}: free space unknown"),
    }
}

/// Bytes under `path` from `du -sb`, or `None` when it cannot be read.
fn du_bytes(path: &Path) -> Option<u64> {
    let out = Command::new("du").arg("-sb").arg(path).output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Mount point and available MB of the filesystem holding `path`, from `df -Pm`.
fn df(path: &Path) -> Option<(String, u64)> {
    let out = Command::new("df").args(["-Pm"]).arg(path).output().ok()?;
    let row = String::from_utf8_lossy(&out.stdout)
        .lines()
        .nth(1)?
        .to_owned();
    let mut cols = row.split_whitespace();
    let avail = cols.nth(3)?.parse().ok()?;
    let mount = row.split_whitespace().last()?.to_owned();
    Some((mount, avail))
}

fn age_days(path: &Path, now: SystemTime) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| now.duration_since(t).ok())
        .map_or(0, |d| d.as_secs() / 86_400)
}

#[allow(clippy::cast_precision_loss)] // reason: a size in GiB for a human; the last bits do not matter.
fn gib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / 1_073_741_824.0)
}
