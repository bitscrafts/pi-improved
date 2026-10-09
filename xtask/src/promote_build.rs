//! `xtask promote-build [--worktree <path>]` (spec 34 R2): builds the
//! worktree, launches a SECOND daemon against an isolated dev runtime dir —
//! never `/root/.pii` (008 §6: the boss may build and test its own patch,
//! but deploy stays human, permanently) — drives the fixed
//! `graphs/starter/soak.toml` through it via `pii soak`, records pass/fail,
//! and shuts the dev daemon down. Every artifact this step writes (the dev
//! runtime dir, the result ledger) lives under `<worktree>/_tmp/`, mirroring
//! `heal_seed`/`heal_collect`'s own confinement convention.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// One recorded `promote-build` attempt (R2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromoteBuildResult {
    /// The worktree the build ran in.
    pub worktree: String,
    /// The isolated dev runtime dir the soak daemon used.
    pub runtime_dir: String,
    /// Whether `cargo build --workspace` succeeded.
    pub build_ok: bool,
    /// Whether the soak run against the dev daemon passed (0 failures).
    pub soak_ok: bool,
    /// Epoch millis this attempt started.
    pub ts_ms: u64,
}

/// The dev runtime dir a `promote-build` attempt started at `ts_ms` uses,
/// confined under `<worktree>/_tmp/promote-builds/<ts_ms>/runtime` — never
/// `/root/.pii` (R1/R2/G4: isolated from, and never touching, prod).
#[must_use]
pub fn runtime_dir_for(worktree: &Path, ts_ms: u64) -> PathBuf {
    worktree
        .join("_tmp/promote-builds")
        .join(ts_ms.to_string())
        .join("runtime")
}

/// Runs `promote-build` against `worktree` (defaults to this repo's own
/// working tree — building writes only to the gitignored `target/`, so
/// running it there is safe; a disposable `heal/<sig>` worktree from
/// `heal-seed` is just as valid a `worktree` argument).
///
/// # Errors
/// The worktree has no `Cargo.toml`, the build fails, the dev daemon never
/// binds its control socket, or the soak run reports a failure.
#[allow(clippy::disallowed_methods)] // reason: xtask dev-tooling spawns the
// build and the dev daemon, same exemption as rss_gate.rs.
pub fn run(worktree: Option<PathBuf>) -> Result<(), String> {
    let worktree = worktree.unwrap_or_else(crate::workspace_root);
    let manifest = worktree.join("Cargo.toml");
    if !manifest.exists() {
        return Err(format!(
            "promote-build: no Cargo.toml under {}",
            worktree.display()
        ));
    }

    let build_ok = Command::new("cargo")
        .args(["build", "--workspace", "--manifest-path"])
        .arg(&manifest)
        .status()
        .map_err(|e| format!("promote-build: cargo build: {e}"))?
        .success();

    let ts_ms = now_ms();
    let runtime = runtime_dir_for(&worktree, ts_ms);
    std::fs::create_dir_all(&runtime).map_err(|e| format!("mkdir runtime: {e}"))?;

    let soak_ok = if build_ok {
        run_dev_soak(&worktree, &runtime)?
    } else {
        false
    };

    let result = PromoteBuildResult {
        worktree: worktree.display().to_string(),
        runtime_dir: runtime.display().to_string(),
        build_ok,
        soak_ok,
        ts_ms,
    };
    write_result(&worktree, &result)?;

    println!(
        "promote-build: worktree={} runtime={} build_ok={build_ok} soak_ok={soak_ok}",
        worktree.display(),
        runtime.display(),
    );
    if !build_ok {
        return Err(String::from("promote-build: build failed"));
    }
    if !soak_ok {
        return Err(String::from("promote-build: soak failed"));
    }
    Ok(())
}

/// Starts the built daemon against `runtime` (a dev runtime dir under
/// `worktree`'s own `_tmp/`), drives one run of `graphs/starter/soak.toml`
/// through `pii soak`, then tears the daemon down. Never touches
/// `/root/.pii` — `PII_RUNTIME_DIR` is the only runtime-dir source the
/// spawned daemon sees (R1/R2).
#[allow(clippy::disallowed_methods)] // reason: xtask dev-tooling spawns the
// dev daemon, same exemption as rss_gate.rs.
fn run_dev_soak(worktree: &Path, runtime: &Path) -> Result<bool, String> {
    let pii_bin = worktree.join("target/debug/pii");
    if !pii_bin.exists() {
        return Err(format!(
            "promote-build: {} missing after a successful build",
            pii_bin.display()
        ));
    }
    let model_fixture = worktree.join("fixtures/phase3/model/assistant.json");

    let graphs_dir = runtime.join("graphs/starter");
    std::fs::create_dir_all(&graphs_dir).map_err(|e| format!("mkdir graphs: {e}"))?;
    std::fs::copy(
        worktree.join("graphs/starter/soak.toml"),
        graphs_dir.join("soak.toml"),
    )
    .map_err(|e| format!("cp soak.toml: {e}"))?;

    let mut daemon = Command::new(&pii_bin)
        .arg("daemon")
        .args(["--sandbox", "off"])
        .args(["--store", &runtime.join("store.redb").display().to_string()])
        .env("PII_RUNTIME_DIR", runtime)
        .env("PII_MODEL", format!("scripted:{}", model_fixture.display()))
        .env_remove("PII_CONFIG")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn dev daemon: {e}"))?;
    let pid = daemon.id();

    let sock = runtime.join("control.sock");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !sock.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let soak_ok = if sock.exists() {
        Command::new(&pii_bin)
            .arg("soak")
            .args(["--connect", &sock.display().to_string()])
            .args(["--runs", "1"])
            .args(["--graph", "starter/soak"])
            .status()
            .is_ok_and(|s| s.success())
    } else {
        false
    };

    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status();
    let _ = daemon.wait();
    Ok(soak_ok)
}

/// Writes `<worktree>/_tmp/promote-builds/<ts_ms>/result.json`.
fn write_result(worktree: &Path, result: &PromoteBuildResult) -> Result<(), String> {
    let dir = worktree
        .join("_tmp/promote-builds")
        .join(result.ts_ms.to_string());
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir result dir: {e}"))?;
    let bytes = serde_json::to_vec_pretty(result).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("result.json"), bytes).map_err(|e| format!("write result: {e}"))
}

/// Epoch millis (best-effort — dev tooling, no `Ctx`/`Clock` exists here).
#[allow(clippy::disallowed_methods)] // reason: same SystemTime exemption as
// heal_seed.rs/heal_collect.rs.
fn now_ms() -> u64 {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::runtime_dir_for;

    /// R1/R2/G4: the dev runtime dir a `promote-build` attempt uses is
    /// always confined under `<worktree>/_tmp/` — never `/root/.pii` (prod's
    /// default runtime dir) regardless of which worktree is given.
    #[test]
    fn runtime_dir_is_confined_under_tmp_never_prod() {
        let worktree_buf = crate::workspace_root();
        let worktree = worktree_buf.as_path();
        let rt = runtime_dir_for(worktree, 12345);
        assert!(
            rt.starts_with(worktree.join("_tmp")),
            "runtime dir must live under <worktree>/_tmp: {}",
            rt.display()
        );
        assert!(
            !rt.starts_with("/root/.pii"),
            "runtime dir must never resolve under prod's /root/.pii: {}",
            rt.display()
        );
    }

    /// Two attempts (distinct `ts_ms`) never collide on the same runtime dir
    /// — each promote-build run is isolated from the last.
    #[test]
    fn runtime_dir_is_unique_per_attempt() {
        let worktree = std::path::Path::new("/tmp/some-worktree");
        assert_ne!(
            runtime_dir_for(worktree, 1),
            runtime_dir_for(worktree, 2),
            "distinct attempts must not share a runtime dir"
        );
    }
}
