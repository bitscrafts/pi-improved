//! `xtask heal-seed` (S1.1): a deterministic broken test in a disposable
//! git worktree, recorded in a JSON ledger. The seed refuses to exit 0 if
//! the break did not actually break (reproduce before recording).

use serde::{Deserialize, Serialize};

/// One seed record (S1.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::redundant_pub_crate)] // bin crate: reachable form
pub(crate) struct SeedRecord {
    /// `blake3(ts, git HEAD)` first 12 hex.
    pub sig: String,
    /// Absolute worktree path.
    pub worktree: String,
    /// Branch (`heal/<sig>`).
    pub branch: String,
    /// Epoch millis.
    pub ts_ms: u64,
    /// Worktree-relative broken file.
    pub broken_file: String,
    /// 1-based broken line.
    pub broken_line: u64,
    /// The ORIGINAL line content (for heal-collect's human verification).
    pub original_line: String,
}

const LEDGER: &str = "/projects/pi-improved/_tmp/heal-seeds.json";
const WORKTREES: &str = "/projects/pi-improved/_tmp/heal-worktrees";
const MAX_WORKTREES: usize = 50;
const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB

/// Runs `heal-seed` (S1.1).
///
/// # Errors
/// A refusal message (rate limit / disk ceiling / break not reproduced).
#[allow(clippy::too_many_lines)] // reason: one sequential seed orchestration
#[allow(clippy::redundant_pub_crate)] // bin crate: reachable form
#[allow(clippy::disallowed_methods)] // reason: heal-seed is dev tooling that
// drives git worktree + cargo (same as main::run_cmd and rss_gate).
pub(crate) fn run() -> Result<(), String> {
    // 1. Signature: blake3(git HEAD, broken target) — DETERMINISTIC per
    //    (HEAD, target), so re-seeding the same break at the same HEAD
    //    hits the 24 h rate-limit refusal (a pid/nanos-based sig could
    //    never repeat, making the ceiling dead code). A new commit to the
    //    repo legitimately re-allows the seed (the code changed).
    let head = git_head()?;
    let broken_file = "crates/pii-core/src/clock.rs";
    let sig = blake3_short(&format!("{head}:{broken_file}"));
    let dir = std::path::Path::new(WORKTREES);
    let _ = std::fs::create_dir_all(dir);

    // Rate limit + disk ceiling (S1.1 step 5 refusals). Same signature
    // within the last 24 h -> refuse (2 attempts per signature per 24 h
    // ceiling, then escalate-by-refusal).
    let existing = read_ledger();
    let now_ms = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis()),
    )
    .unwrap_or(0);
    if existing
        .iter()
        .any(|e| e.sig == sig && now_ms.saturating_sub(e.ts_ms) < 24 * 3600 * 1000)
    {
        return Err(format!(
            "heal-seed: signature {sig} seeded within the last 24 h (2/24 h ceiling); wait or pick a new target"
        ));
    }
    if dir.read_dir().is_ok_and(|d| d.count() >= MAX_WORKTREES) {
        return Err(String::from(
            "heal-seed: >= 50 worktrees; run heal-prune first",
        ));
    }
    if dir_size(dir)? > MAX_BYTES {
        return Err(String::from(
            "heal-seed: worktrees > 2 GiB; run heal-prune first",
        ));
    }

    // 2. git worktree add -b heal/<sig> <path> HEAD.
    let wt = dir.join(&sig);
    git(&[
        "worktree",
        "add",
        "-b",
        &format!("heal/{sig}"),
        wt.to_str().unwrap_or_default(),
        "HEAD",
    ])?;

    // 3. Apply the deterministic break: flip advance(1500)->advance(1501) in
    //    clock.rs, which makes exactly `fixed_clock_advances` fail (line 67).
    let full = wt.join(broken_file);
    let src =
        std::fs::read_to_string(&full).map_err(|e| format!("read {}: {e}", full.display()))?;
    let original_line = String::from("        c.advance(1500);");
    let broken_line = 66u64;
    if !src.contains(&original_line) {
        let _ = git(&[
            "worktree",
            "remove",
            "--force",
            wt.to_str().unwrap_or_default(),
        ]);
        return Err(String::from(
            "heal-seed: clock.rs no longer matches the expected seed point; pick a new target",
        ));
    }
    let flipped = src.replace(
        "        c.advance(1500);",
        "        c.advance(1501); // SEEDED BREAK (heal-seed)",
    );
    std::fs::write(&full, flipped).map_err(|e| format!("write: {e}"))?;

    // 4. Record in the ledger.
    let mut ledger = existing;
    ledger.push(SeedRecord {
        sig: sig.clone(),
        worktree: wt.to_str().unwrap_or_default().to_owned(),
        branch: format!("heal/{sig}"),
        ts_ms: now_ms,
        broken_file: String::from(broken_file),
        broken_line,
        original_line,
    });
    write_ledger(&ledger)?;

    // 5. Reproduction check: the break MUST fail one test, else refuse 0.
    let test = "fixed_clock_advances";
    let status = std::process::Command::new("cargo")
        .args(["test", "-p", "pii-core", test, "--manifest-path"])
        .arg(wt.join("Cargo.toml"))
        .status()
        .map_err(|e| format!("cargo test: {e}"))?;
    if status.success() {
        // The break did not break — remove the worktree and refuse.
        let _ = git(&[
            "worktree",
            "remove",
            "--force",
            wt.to_str().unwrap_or_default(),
        ]);
        write_ledger(&ledger)?;
        return Err(String::from(
            "heal-seed: seeded break did not fail the test; refusing to record",
        ));
    }

    println!(
        "heal-seed: worktree={} sig={} test={test} break={}:{}",
        wt.display(),
        sig,
        broken_file,
        broken_line
    );
    Ok(())
}

#[allow(clippy::disallowed_methods)] // reason: heal-seed is dev tooling;
// git shell-outs follow the main::run_cmd / rss_gate precedent.
fn git(args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .args(args)
        .output()
        .map_err(|e| format!("git {args:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn git_head() -> Result<String, String> {
    git(&["rev-parse", "HEAD"])
}

fn blake3_short(s: &str) -> String {
    pii_store::hash(s.as_bytes()).chars().take(12).collect()
}

fn read_ledger() -> Vec<SeedRecord> {
    std::fs::read_to_string(LEDGER)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_ledger(ledger: &[SeedRecord]) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(ledger).map_err(|e| e.to_string())?;
    std::fs::write(LEDGER, bytes).map_err(|e| e.to_string())
}

fn dir_size(dir: &std::path::Path) -> Result<u64, String> {
    let mut total = 0u64;
    walk_size(dir, &mut total)?;
    Ok(total)
}

fn walk_size(d: &std::path::Path, total: &mut u64) -> Result<(), String> {
    for e in std::fs::read_dir(d).map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        let p = e.path();
        if p.is_dir() {
            walk_size(&p, total)?;
        } else if let Ok(m) = e.metadata() {
            *total = total.saturating_add(m.len());
        }
    }
    Ok(())
}
