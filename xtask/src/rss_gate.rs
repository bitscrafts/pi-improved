//! `xtask rss-gate` (F5.14/N4): start a real daemon under load and assert the
//! RSS MAX stays under budget and the steady-state slope stays flat.

use std::process::Stdio;
use std::time::Duration;

/// Reads `VmRSS` (kB) from `/proc/<pid>/status` (Linux; 0 elsewhere).
fn rss_kb(pid: u32) -> u64 {
    let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
        return 0;
    };
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
        }
    }
    0
}

/// Runs the gate (F5.14): daemon up with `fixtures/phase3/soak.toml`, runs
/// of `starter/soak` via `pii soak`, RSS sampled every `interval` seconds for
/// `secs`; asserts `max < max_mb`, `slope < 2048 KiB`, sample count, exit 0.
///
/// # Errors
/// A human-readable failure string naming the breached assertion.
#[allow(clippy::redundant_pub_crate)] // bin crate: widest reachable form
#[allow(clippy::too_many_lines)]
// reason: one sequential gate orchestration
// (spawn, drive, sample, assert) is clearer than a state-machine split.
#[allow(clippy::disallowed_methods)] // reason: rss-gate is dev-tooling that
// must spawn the daemon and soak subprocesses (same as main::run_cmd).
pub(crate) fn run(secs: u64, interval: u64, max_mb: u64, runs: u32) -> Result<(), String> {
    let root = "/projects/pi-improved";
    let dir = std::path::Path::new(root).join("_tmp/phase3/rss-gate");
    if dir.exists() {
        // No SystemTime (disallowed): pid+monotonic tick is a unique-enough
        // suffix for the stale-dir move-aside.
        let old = dir.with_extension(format!("old-{}", std::process::id()));
        let _ = std::fs::rename(&dir, old);
    }
    std::fs::create_dir_all(dir.clone()).map_err(|e| format!("mkdir: {e}"))?;
    let rt = dir.join("rt");
    std::fs::create_dir_all(rt.join("graphs/starter")).map_err(|e| format!("mkdir: {e}"))?;
    // The daemon loads graphs from <runtime_dir>/graphs.
    std::fs::copy(
        format!("{root}/graphs/starter/soak.toml"),
        rt.join("graphs/starter/soak.toml"),
    )
    .map_err(|e| format!("copy soak graph: {e}"))?;
    std::fs::copy(
        format!("{root}/graphs/starter/soak_cron.toml"),
        rt.join("graphs/starter/soak_cron.toml"),
    )
    .map_err(|e| format!("copy soak_cron graph: {e}"))?;

    let logf = std::fs::File::create(dir.join("daemon.log")).map_err(|e| e.to_string())?;
    let mut daemon = std::process::Command::new(format!("{root}/target/debug/pii"))
        .args(["daemon", "--sandbox", "off", "--store"])
        .arg(dir.join("store.redb"))
        .env("PII_RUNTIME_DIR", &rt)
        .env(
            "PII_MODEL",
            format!("scripted:{root}/fixtures/phase3/model/assistant.json"),
        )
        .stdout(Stdio::from(logf.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::from(logf))
        .spawn()
        .map_err(|e| format!("spawn daemon: {e}"))?;
    let pid = daemon.id();
    let sock = rt.join("control.sock");

    // Wait for the control socket.
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !sock.exists() {
        if std::time::Instant::now() >= deadline {
            let _ = daemon.kill();
            let log = std::fs::read_to_string(dir.join("daemon.log")).unwrap_or_default();
            return Err(format!("daemon never bound control.sock:\n{log}"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    // Drive load first, then sample the idle daemon (F5.14: slope is the
    // post-load steady state). Sequential `--runs 3000` takes ~5 min, which
    // on top of the 300 s sampling window exceeded the harness shell cap
    // ("timed out waiting for shell command"); `--concurrency 16` keeps the
    // same 3000-run total but completes the load in well under a minute.
    let soak_status = std::process::Command::new(format!("{root}/target/debug/pii"))
        .args(["soak", "--connect"])
        .arg(&sock)
        .args([
            "--runs",
            &runs.to_string(),
            "--graph",
            "starter/soak",
            "--concurrency",
            "16",
        ])
        .env(
            "PII_MODEL",
            format!("scripted:{root}/fixtures/phase3/model/assistant.json"),
        )
        .status()
        .map_err(|e| format!("soak: {e}"))?;
    if !soak_status.success() {
        let _ = daemon.kill();
        return Err(String::from("pii soak failed"));
    }

    // Sample RSS every `interval` seconds for `secs` seconds.
    let mut samples: Vec<u64> = Vec::new();
    let total = std::time::Instant::now();
    while total.elapsed() < Duration::from_secs(secs) {
        samples.push(rss_kb(pid));
        std::thread::sleep(Duration::from_secs(interval));
    }
    samples.push(rss_kb(pid));
    let rss_max = samples.iter().max().copied().unwrap_or(0);
    let n10 = (samples.len() / 10).max(1);
    let rss_at_10 = samples.get(n10).copied().unwrap_or(0);
    let rss_slope = samples
        .last()
        .copied()
        .unwrap_or(0)
        .saturating_sub(rss_at_10);
    let still_alive = rss_kb(pid) > 0;

    // SIGTERM → the daemon must exit 0 (F5.3).
    let _ = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status();
    let exit0 = daemon.wait().is_ok_and(|st| st.success());

    println!(
        "rss-gate: samples={} max_kb={} slope_kb={}",
        samples.len(),
        rss_max,
        rss_slope
    );
    if rss_max >= max_mb * 1024 {
        return Err(format!("RSS-GATE FAIL: max {rss_max}kB >= {max_mb}MB"));
    }
    if rss_slope >= 2048 {
        return Err(format!("RSS-GATE FAIL: slope {rss_slope}kB >= 2048"));
    }
    let want = usize::try_from(secs / interval.max(1))
        .unwrap_or(0)
        .saturating_sub(1);
    if samples.len() < want {
        return Err(format!("RSS-GATE FAIL: samples {} < {want}", samples.len()));
    }
    if !still_alive || !exit0 {
        return Err(String::from("RSS-GATE FAIL: daemon not healthy at end"));
    }
    Ok(())
}
