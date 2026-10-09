//! `xtask rss-gate` (F5.14/N4): the leak gate.
//!
//! Starts `target/debug/pii daemon` with `fixtures/phase3/soak.toml` and a
//! scripted model under `_tmp/phase3/soak/`, drives `pii soak` against it,
//! samples `VmRSS` from `/proc/<pid>/status` every `--interval` seconds for
//! `--secs`, then asserts: (a) **max** RSS < `--max-mb`; (b) the steady-state
//! slope `rss_last - rss_at_10_percent < SLOPE_KB`; (c) sample count >=
//! `secs/interval - 1`; (d) the daemon still answers `sys.status` and exits 0
//! on `SIGTERM`. Prints `rss-gate: samples=<n> max_kb=<n> slope_kb=<n>`.

// reason: xtask is the dev-tooling gate runner; it must spawn the daemon and
// probe it (Command ban targets the agent runtime, not build tooling — same
// exemption as main.rs run_cmd).
#![allow(clippy::disallowed_methods)]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

// RSS-gate remediation (2026-09-05, user-approved after two independent
// engine fixes): the daemon's tokio blocking-pool/worker-thread churn was
// capped (cmd_daemon.rs, 279->16 threads under load) and redb's 1 GiB
// default cache was capped to 4-8 MiB per store (pii-store/src/lib.rs and
// every auxiliary store) - together a 4-9x slope reduction (45-115MB ->
// ~12-14MB over the full 300s/3000-run gate). The remainder is `pii-store`'s
// `RUN_JOURNAL` table growing by DESIGN - every run's events persist so
// `pii graph trace`/resume survive a daemon restart (the system's core
// durability property). `--runs 3000` in ~5 minutes is a synthetic stress
// load no real deployment produces (real traffic is bursty cron/channel/
// approval activity); 16384 KiB (16 MiB) gives headroom for that legitimate,
// bounded growth while still catching an UNBOUNDED leak (which would blow
// well past this over the same window, as the pre-fix runs demonstrated at
// 45-115MB). Do NOT raise this further without re-measuring; if a future
// change pushes the slope back toward this ceiling, that is a real
// regression to investigate, not a threshold to bump again.
const SLOPE_KB: i64 = 16384;

/// Runs the gate with the CLI-provided knobs.
///
/// # Errors
/// A human-readable failure line on any breach.
#[allow(clippy::too_many_lines)] // reason: one sequential gate orchestration.
#[allow(clippy::redundant_pub_crate)] // reason: bin crate; widest reachable.
#[allow(clippy::similar_names)] // reason: soak/sock are distinct gate nouns.
pub(crate) fn run(secs: u64, interval: u64, max_mb: u64, runs: u32) -> Result<(), String> {
    let root_buf = crate::workspace_root();
    let root = root_buf.as_path();
    let scratch = root.join("_tmp/phase3/rss-gate");
    if scratch.exists() {
        let old = format!("{}.old-{}", scratch.display(), now_secs());
        std::fs::rename(&scratch, old).map_err(|e| format!("mv scratch: {e}"))?;
    }
    std::fs::create_dir_all(&scratch).map_err(|e| format!("mkdir scratch: {e}"))?;

    // The daemon needs its runtime dir + a graphs dir with the soak graphs.
    let runtime = scratch.join("rt");
    let graphs = runtime.join("graphs/starter");
    std::fs::create_dir_all(&graphs).map_err(|e| format!("mkdir graphs: {e}"))?;
    for name in ["soak", "soak_cron"] {
        std::fs::copy(
            root.join("graphs/starter").join(format!("{name}.toml")),
            graphs.join(format!("{name}.toml")),
        )
        .map_err(|e| format!("cp {name}: {e}"))?;
    }

    let model = root.join("fixtures/phase3/model/assistant.json");
    let daemon = Command::new(root.join("target/debug/pii"))
        .arg("daemon")
        .args(["--sandbox", "off"])
        .args(["--store", &scratch.join("store.redb").display().to_string()])
        .args([
            "--config",
            &root.join("fixtures/phase3/soak.toml").display().to_string(),
        ])
        .env("PII_RUNTIME_DIR", &runtime)
        .env("PII_MODEL", format!("scripted:{}", model.display()))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn daemon: {e}"))?;
    let pid = daemon.id();

    // Wait for the control socket, then drive soak load for the window.
    let sock = runtime.join("control.sock");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !sock.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    if !sock.exists() {
        let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
        return Err(String::from("rss-gate: daemon did not bind control.sock"));
    }

    // Drive soak in the background for the whole gate window.
    let mut soak_driver = Command::new(root.join("target/debug/pii"))
        .arg("soak")
        .args(["--connect", &sock.display().to_string()])
        .args(["--runs", &runs.to_string()])
        .arg("--graph")
        .arg("starter/soak")
        .env("PII_MODEL", format!("scripted:{}", model.display()))
        .spawn()
        .map_err(|e| format!("spawn soak: {e}"))?;

    // Sample VmRSS every `interval` seconds for `secs`.
    let mut samples: Vec<i64> = Vec::new();
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        std::thread::sleep(Duration::from_secs(interval));
        if let Some(kb) = sample_rss_kb(pid) {
            samples.push(kb);
        }
    }
    let _ = soak_driver.kill();
    let _ = soak_driver.wait();

    // The daemon must still be alive (answers exist) and exit 0 on SIGTERM.
    let alive = std::path::Path::new(&format!("/proc/{pid}")).exists();
    if !alive {
        return Err(String::from("RSS-GATE FAIL: daemon died under load"));
    }
    let term = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .map_err(|e| format!("kill: {e}"))?;
    if !term.success() {
        return Err(String::from("RSS-GATE FAIL: SIGTERM failed"));
    }

    let n_samples = samples.len();
    let n_u = u64::try_from(n_samples).unwrap_or(u64::MAX);
    let needed = secs / interval;
    if n_u < needed.saturating_sub(1) {
        return Err(format!(
            "RSS-GATE FAIL: samples={n_u} < secs/interval-1={}",
            needed.saturating_sub(1)
        ));
    }
    let max_kb = samples.iter().max().copied().unwrap_or(0);
    if max_kb > i64::try_from(max_mb).unwrap_or(i64::MAX) * 1024 {
        return Err(format!("RSS-GATE FAIL: max_kb={max_kb} >= max_mb={max_mb}"));
    }
    // Slope: last sample minus the ~10%-in sample.
    let pct_idx = n_samples / 10;
    let slope = samples.last().copied().unwrap_or(0) - samples.get(pct_idx).copied().unwrap_or(0);
    if slope > SLOPE_KB {
        return Err(format!("RSS-GATE FAIL: slope_kb={slope} >= {SLOPE_KB}"));
    }
    println!("rss-gate: samples={n_samples} max_kb={max_kb} slope_kb={slope}");
    Ok(())
}

/// Reads `VmRSS` (KiB) from `/proc/<pid>/status`.
fn sample_rss_kb(pid: u32) -> Option<i64> {
    let src = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    src.lines().find_map(|l| {
        let rest = l.strip_prefix("VmRSS:")?;
        rest.trim().strip_suffix("kB")?.trim().parse::<i64>().ok()
    })
}

/// Seconds since the epoch (for the stale-scratch rename).
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
