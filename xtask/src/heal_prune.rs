//! `xtask heal-prune` (S1.5): moves stale worktrees to trash via `mv`,
//! NEVER `rm -rf`. Removes the git worktree registration first.

const WORKTREES: &str = "/projects/pi-improved/_tmp/heal-worktrees";
const TRASH: &str = "/projects/pi-improved/_tmp/heal-trash";

/// Runs `heal-prune [--all]`.
///
/// # Errors
/// Filesystem or git failures.
#[allow(clippy::too_many_lines)] // reason: one sequential prune loop
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
#[allow(clippy::disallowed_methods)] // reason: heal-prune is dev tooling that
// shells git worktree (same as main::run_cmd / rss_gate).
pub(crate) fn run(all: bool) -> Result<(), String> {
    let dir = std::path::Path::new(WORKTREES);
    let trash = std::path::Path::new(TRASH);
    std::fs::create_dir_all(trash).map_err(|e| e.to_string())?;
    // Dev tooling: SystemTime is the honest way to measure 7 real days; the
    // Clock trait is for the agent runtime, not the operator toolbelt.
    let now = system_secs();
    let mut moved = 0usize;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let path = e.path();
            if !path.is_dir() {
                continue;
            }
            let mtime = file_mtime_secs(&e.path());
            let age_secs = now.saturating_sub(mtime);
            let older_than_7d = age_secs > 7 * 86400;
            if !all && !older_than_7d {
                continue;
            }
            // mv to trash FIRST (never rm -rf); then drop the git worktree
            // registration. If the tree is dirty, `git worktree remove`
            // errors — that is fine (S1.5): the dir is already in trash and
            // `git worktree prune` reclaims the registration later.
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let dest = trash.join(name);
            if dest.exists() {
                let _ = std::fs::rename(&dest, trash.join(format!("{name}.old-{now}")));
            }
            std::fs::rename(&path, &dest).map_err(|e| format!("mv {name}: {e}"))?;
            let _ = std::process::Command::new("git")
                .args([
                    "worktree",
                    "remove",
                    "--force",
                    path.to_str().unwrap_or_default(),
                ])
                .status();
            moved += 1;
        }
    }
    println!("heal-prune: moved={moved} worktrees to {TRASH}");
    Ok(())
}

/// Seconds since the Unix epoch.
#[allow(clippy::disallowed_methods)] // reason: heal-prune is dev tooling;
// SystemTime is the honest 7-day measure (main::run_cmd precedent).
fn system_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// File mtime in epoch seconds (0 on any error — treated as "now").
fn file_mtime_secs(p: &std::path::Path) -> u64 {
    std::fs::metadata(p)
        .ok()
        .and_then(|m| {
            m.modified().ok().and_then(|t| {
                t.duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .map(|d| d.as_secs())
            })
        })
        .unwrap_or(0)
}
