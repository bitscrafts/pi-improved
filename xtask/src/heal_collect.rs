//! `xtask heal-collect <sig>` (S1.4): the ONLY way a fix leaves the sandbox.
//! Writes the worktree diff to `_tmp/heal-reviews/<sig>.patch` and prints
//! the seed's original line for human verification.

use crate::heal_seed::SeedRecord;

const SEEDS: &str = "/projects/pi-improved/_tmp/heal-seeds.json";
const REVIEWS: &str = "/projects/pi-improved/_tmp/heal-reviews";

/// Runs `heal-collect <sig>`.
///
/// # Errors
/// Unknown signature, empty patch, or write failure.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
#[allow(clippy::disallowed_methods)] // reason: heal-collect is dev tooling
// that shells git diff (same as main::run_cmd / rss_gate).
pub(crate) fn run(sig: &str) -> Result<(), String> {
    let ledger: Vec<SeedRecord> =
        serde_json::from_str(&std::fs::read_to_string(SEEDS).map_err(|e| format!("ledger: {e}"))?)
            .map_err(|e| format!("ledger parse: {e}"))?;
    let seed = ledger
        .iter()
        .find(|s| s.sig == sig)
        .ok_or_else(|| format!("heal-collect: no seed with sig '{sig}'"))?;
    let wt = &seed.worktree;
    let dir = std::path::Path::new(REVIEWS);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let patch_path = dir.join(format!("{sig}.patch"));

    // git -C <worktree> diff > _tmp/heal-reviews/<sig>.patch
    let out = std::process::Command::new("git")
        .args(["-C", wt, "diff"])
        .output()
        .map_err(|e| format!("git diff: {e}"))?;
    let patch = String::from_utf8_lossy(&out.stdout).into_owned();
    if patch.trim().is_empty() {
        // Revert-type repair: the seeded break was restored exactly, so the
        // worktree is pristine and there is nothing to collect — that IS the
        // successful outcome (Slice 1's seeded target is a practice defect).
        // Verify the restoration honestly before declaring success: the
        // seeded file must contain the recorded original line again.
        let full = std::path::Path::new(wt).join(&seed.broken_file);
        let restored = std::fs::read_to_string(&full).ok().is_some_and(|src| {
            usize::try_from(seed.broken_line.saturating_sub(1))
                .is_ok_and(|idx| src.lines().nth(idx) == Some(&seed.original_line))
        });
        if restored {
            let clean = std::process::Command::new("git")
                .args(["-C", wt, "status", "--porcelain"])
                .output()
                .is_ok_and(|o| o.stdout.is_empty());
            if clean {
                println!(
                    "heal-collect: repair RESTORED the seeded break exactly — worktree is pristine, no patch artifact (revert-type repair). Verified: {}:{} == recorded original.",
                    seed.broken_file, seed.broken_line
                );
                return Ok(());
            }
        }
        return Err(String::from(
            "heal-collect: worktree diff is empty and the seeded break was NOT cleanly restored (no fix?)",
        ));
    }
    std::fs::write(&patch_path, &patch).map_err(|e| format!("write patch: {e}"))?;

    println!("heal-collect: patch={}", patch_path.display());
    println!("heal-collect: original_line={}", seed.original_line);
    println!(
        "heal-collect: seed sig={} worktree={} break={}:{}",
        seed.sig, seed.worktree, seed.broken_file, seed.broken_line
    );
    Ok(())
}
