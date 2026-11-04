//! `cargo xtask disk [--gc]` (spec 367 R1, R3, R5): the artefact table `pii disk` prints, plus the owner's removal.
//! `--gc` removes `_tmp/targets/<purpose>` older than `ttl_days` and fresh-gate exports older than
//! `export_ttl_hours` whose owner is dead. Nothing is removed while a cargo or rustc is live.

// reason: dev-tooling reads the wall clock and the environment and shells out to du and df; it is never replayed.
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use crate::fresh_gate::{BASE, owner_is_alive, safe_to_remove};

/// Repo-relative ad-hoc target root with a TTL (spec 367 R3).
const TARGETS_ROOT: &str = "_tmp/targets";
/// Repo-relative gate cache roots shown in the table (spec 367 R1).
const REPO_ROOTS: &[&str] = &[
    "_tmp/gate-target",
    "_tmp/gate-target-fresh",
    TARGETS_ROOT,
    "_tmp/_trash",
];
/// Roots outside the repository: gate exports, trash, coverage caches.
const EXTERNAL_ROOTS: &[&str] = &["/projects/_trash", "/tmp/_trash-podman", "/tmp/gate-cache"];

const DEFAULT_TTL_DAYS: u64 = 7;
const DEFAULT_EXPORT_TTL_HOURS: u64 = 24;

/// `DISK_TTL_DAYS`: how old an ad-hoc target may be before `--gc` removes it. Junk falls back to the default.
pub fn ttl_days(env: Option<&str>) -> u64 {
    env.and_then(|v| v.trim().parse().ok())
        .unwrap_or(DEFAULT_TTL_DAYS)
}

/// `DISK_EXPORT_TTL_HOURS`: how long a dead owner's failed export survives before `--gc` removes it.
pub fn export_ttl_hours(env: Option<&str>) -> u64 {
    env.and_then(|v| v.trim().parse().ok())
        .unwrap_or(DEFAULT_EXPORT_TTL_HOURS)
}

/// Strictly older than the TTL: a target at exactly its TTL is kept.
pub fn ttl_expired(age: u64, ttl: u64) -> bool {
    age > ttl
}

/// True when a `/proc/<pid>/comm` names a build process whose presence blocks removal.
pub fn is_build_comm(comm: &str) -> bool {
    matches!(comm.trim(), "cargo" | "rustc")
}

/// True when a cargo or rustc is running. An unreadable `/proc` counts as live: removal then refuses.
pub fn build_is_live() -> bool {
    let Ok(rd) = std::fs::read_dir("/proc") else {
        return true;
    };
    rd.flatten()
        .filter_map(|e| std::fs::read_to_string(e.path().join("comm")).ok())
        .any(|c| is_build_comm(&c))
}

fn age_days(path: &Path, now: SystemTime) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| now.duration_since(t).ok())
        .map_or(0, |d| d.as_secs() / 86_400)
}

pub fn du_bytes(path: &Path) -> Option<u64> {
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
    let avail = row.split_whitespace().nth(3)?.parse().ok()?;
    let mount = row.split_whitespace().last()?.to_owned();
    Some((mount, avail))
}

#[allow(clippy::cast_precision_loss)] // reason: a size in GiB for a human; the last bits do not matter.
fn gib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / 1_073_741_824.0)
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

fn report(repo: &Path) {
    let now = SystemTime::now();
    println!("artefacts (bytes, age, filesystem, owner, path)");
    let mut total = 0_u64;
    for (path, owned) in artefacts(repo) {
        let Some(bytes) = du_bytes(&path) else {
            continue;
        };
        total += bytes;
        let fs = df(&path).map_or_else(|| "?".to_owned(), |(mount, _)| mount);
        let owner = if owned { "owned" } else { "unowned" };
        println!(
            "{:>10}  {:>4}d  {:<10}  {:<8}  {}",
            gib(bytes),
            age_days(&path, now),
            fs,
            owner,
            path.display()
        );
    }
    println!("total {}", gib(total));
    for (label, free) in [
        ("filesystem /", df(Path::new("/"))),
        ("filesystem (repository)", df(repo)),
    ] {
        match free {
            Some((mount, mb)) => println!("{label}: {mb} MB free on {mount}"),
            None => println!("{label}: free space unknown"),
        }
    }
}

/// Removes what the TTLs name, and nothing while a build is live (spec 367 R3, R5, guardrails).
fn gc(repo: &Path) -> Result<(), String> {
    if build_is_live() {
        println!("gc: a cargo or rustc is live; nothing removed");
        return Ok(());
    }
    let now = SystemTime::now();
    let ttl = ttl_days(std::env::var("DISK_TTL_DAYS").ok().as_deref());
    let targets = repo.join(TARGETS_ROOT);
    if let Ok(rd) = std::fs::read_dir(&targets) {
        for entry in rd.flatten().map(|e| e.path()) {
            if entry.is_dir() && ttl_expired(age_days(&entry, now), ttl) {
                std::fs::remove_dir_all(&entry)
                    .map_err(|e| format!("gc {}: {e}", entry.display()))?;
                println!("gc: removed {} (older than {ttl} days)", entry.display());
            }
        }
    }
    let export_ttl = export_ttl_hours(std::env::var("DISK_EXPORT_TTL_HOURS").ok().as_deref());
    let base = Path::new(BASE);
    if let Ok(rd) = std::fs::read_dir(base) {
        for entry in rd.flatten().map(|e| e.path()) {
            let aged = age_days(&entry, now) * 24 >= export_ttl;
            let stale = entry.is_dir() && safe_to_remove(base, &entry) && !owner_is_alive(&entry);
            if stale && aged {
                std::fs::remove_dir_all(&entry)
                    .map_err(|e| format!("gc {}: {e}", entry.display()))?;
                println!("gc: removed failed export {}", entry.display());
            }
        }
    }
    Ok(())
}

/// `cargo xtask disk [--gc]`.
pub fn run(args: &[String]) -> Result<(), String> {
    let repo = std::env::current_dir().map_err(|e| format!("cwd: {e}"))?;
    if args.iter().any(|a| a == "--gc") {
        gc(&repo)?;
    }
    report(&repo);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_default_and_override() {
        assert_eq!(ttl_days(None), 7);
        assert_eq!(ttl_days(Some("3")), 3);
        assert_eq!(ttl_days(Some("junk")), 7);
    }

    #[test]
    fn ttl_expires_only_strictly_older() {
        assert!(ttl_expired(8, 7));
        assert!(
            !ttl_expired(7, 7),
            "a target exactly at its TTL is still kept"
        );
        assert!(!ttl_expired(0, 7));
    }

    #[test]
    fn a_cargo_or_rustc_process_is_a_live_build() {
        assert!(is_build_comm("cargo\n"));
        assert!(is_build_comm("rustc"));
        assert!(!is_build_comm("bash"));
        assert!(!is_build_comm("cargo-nextest-helper"));
    }
}
