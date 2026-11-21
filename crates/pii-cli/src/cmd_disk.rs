//! `pii disk [--root <repo>]` (spec 367 R1): one table of the artefact roots this repository fills, with bytes,
//! age and the filesystem each lives on, then free space for `/` and for the volume that holds the repository.
//! The floor is judged where the artefacts land (R4), so both lines are printed. Read-only: removal is
//! `cargo xtask disk --gc` (R3), which owns the TTL.

// reason: a diagnostic report reads the wall clock for artefact ages and shells out to `du` and `df`; it is not
// replayed, so the replay-only bans on `SystemTime::now` and `Command::output` do not apply.
#![allow(clippy::disallowed_methods)]

use std::ffi::OsString;
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

/// Side caches of runs that already ended, removed whole (disk-ceilings.md: "their own command", ceiling 0).
const SIDE_CACHES: &[&str] = &[
    "_tmp/tools-gate",
    "_tmp/phase4.7",
    "_tmp/devbox-test",
    "_tmp/xtask-target",
    "_tmp/head-builds-target",
    "_tmp/target-280",
];

/// The warm gate cache: the clean never removes it or anything inside it (disk-ceilings.md).
const WARM_CACHE: &str = "_tmp/gate-target";

/// Windows of disk-ceilings.md: `_tmp/_trash` entries older than 7 days, `_tmp/claude-batchC` older than 14.
const TRASH_DAYS: u64 = 7;
const BATCH_DAYS: u64 = 14;

#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_disk(args: &mut std::env::ArgsOs) -> ExitCode {
    let rest: Vec<OsString> = args.collect();
    if rest.first().is_some_and(|a| a == "clean") {
        return cmd_clean(rest.get(1..).unwrap_or(&[]));
    }
    let mut root = None;
    let mut it = rest.into_iter();
    while let Some(arg) = it.next() {
        if arg == "--root" {
            root = it.next().map(PathBuf::from);
        } else {
            warn!(
                "usage: pii disk [--root <repo>] | pii disk clean [--dry-run|--apply] [--path <p>]"
            );
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

/// `pii disk clean [--dry-run|--apply] [--path <p>] [--root <repo>]` (spec 438 R4-R5). Dry-run unless `--apply`. Without
/// `--path` it removes what disk-ceilings.md says is past its window; with `--path` it removes that one path, which must
/// be inside `_tmp/` and must not be the warm cache.
fn cmd_clean(args: &[OsString]) -> ExitCode {
    let mut apply = false;
    let mut path = None;
    let mut root = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--apply" {
            apply = true;
        } else if arg == "--dry-run" {
            apply = false;
        } else if arg == "--path" {
            path = it.next().map(PathBuf::from);
        } else if arg == "--root" {
            root = it.next().map(PathBuf::from);
        } else {
            warn!("usage: pii disk clean [--dry-run|--apply] [--path <p>] [--root <repo>]");
            return ExitCode::from(2);
        }
    }
    let repo = match root.map_or_else(std::env::current_dir, Ok) {
        Ok(p) => p,
        Err(e) => {
            warn!("pii disk clean: cannot read the current directory: {e}");
            return ExitCode::from(2);
        }
    };
    let candidates = match path {
        Some(p) => match checked_path(&repo, &p) {
            Ok(real) => vec![real],
            Err(why) => {
                warn!("pii disk clean: refused: {why}");
                return ExitCode::from(2);
            }
        },
        None => windowed(&repo),
    };
    let now = SystemTime::now();
    let verb = if apply { "freed" } else { "would free" };
    let mut text = String::new();
    let mut total = 0_u64;
    for path in &candidates {
        let bytes = du_bytes(path).unwrap_or(0);
        total += bytes;
        let _ = writeln!(
            text,
            "{verb} {:>10}  {:>4}d  {}",
            gib(bytes),
            age_days(path, now),
            path.display()
        );
    }
    let _ = writeln!(text, "{verb} {} in {} paths", gib(total), candidates.len());
    if !apply {
        let _ = writeln!(text, "dry run: nothing removed; add --apply to remove");
    }
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    if apply {
        for path in &candidates {
            let removed = if path.is_dir() {
                std::fs::remove_dir_all(path)
            } else {
                std::fs::remove_file(path)
            };
            if let Err(e) = removed {
                warn!("pii disk clean: {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

/// `path` resolved, when it is strictly inside `_tmp/` and outside the warm cache; otherwise why it is refused.
fn checked_path(repo: &Path, path: &Path) -> Result<PathBuf, String> {
    let tmp = std::fs::canonicalize(repo.join("_tmp"))
        .map_err(|_| "there is no _tmp/ in this repository".to_owned())?;
    let real = std::fs::canonicalize(repo.join(path))
        .map_err(|_| format!("{} does not exist, or is outside _tmp/", path.display()))?;
    if real == tmp || !real.starts_with(&tmp) {
        return Err(format!("{} is outside _tmp/", path.display()));
    }
    if real.starts_with(repo.join(WARM_CACHE)) {
        return Err(format!(
            "{} is the warm gate cache ({WARM_CACHE}), kept",
            path.display()
        ));
    }
    Ok(real)
}

/// What the windows of disk-ceilings.md say to remove now.
fn windowed(repo: &Path) -> Vec<PathBuf> {
    let now = SystemTime::now();
    let mut found = Vec::new();
    for (dir, days) in [
        ("_tmp/_trash", TRASH_DAYS),
        ("_tmp/claude-batchC", BATCH_DAYS),
    ] {
        if let Ok(rd) = std::fs::read_dir(repo.join(dir)) {
            found.extend(
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| age_days(p, now) >= days),
            );
        }
    }
    found.extend(
        SIDE_CACHES
            .iter()
            .map(|c| repo.join(c))
            .filter(|p| p.exists()),
    );
    if let Ok(rd) = std::fs::read_dir(repo.join("_tmp")) {
        found.extend(rd.flatten().map(|e| e.path()).filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("verify-"))
        }));
    }
    if repo.join("target").exists() {
        found.push(repo.join("target"));
    }
    found
}

/// Bytes under `path` from `du -sb`, or `None` when it cannot be read.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn du_bytes(path: &Path) -> Option<u64> {
    let out = Command::new("du").arg("-sb").arg(path).output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Mount point and available MB of the filesystem holding `path`, from `df -Pm`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn df(path: &Path) -> Option<(String, u64)> {
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
