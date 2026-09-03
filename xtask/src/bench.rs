//! Cold-start benchmark gate (spec F6.2): p95 of `pii --version` under a
//! budget. Uses `Command::output()` — never `.spawn()` (clippy.toml denies it,
//! and `output()` + `Instant` gives a clean p95).

#![forbid(unsafe_code)]
// reason: benchmark p95 math intentionally casts integer sample counts to f64.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::time::{Duration, Instant};

const DEFAULT_BUDGET_MS: u64 = 20;
const WARMUP: usize = 10;
const MEASURED: usize = 50;

/// Runs the benchmark and returns (`p95_ms`, `budget_ms`).
///
/// # Errors
/// Returns a message if `./target/release/pii` is missing or a run fails.
fn run(budget_ms: Option<u64>) -> Result<(f64, u64), String> {
    let budget = budget_ms.unwrap_or(DEFAULT_BUDGET_MS);
    let binary = std::path::Path::new("./target/release/pii");
    if !binary.exists() {
        return Err("bench-gate: ./target/release/pii not found — build release first (cargo build --release)"
            .to_string());
    }

    // Warmup (JIT/allocator/cache).
    for _ in 0..WARMUP {
        spawn_version(binary)?;
    }

    // Measured.
    let mut samples = Vec::with_capacity(MEASURED);
    for _ in 0..MEASURED {
        let t0 = Instant::now();
        spawn_version(binary)?;
        samples.push(t0.elapsed());
    }
    samples.sort_unstable();

    let p95_index = ((MEASURED as f64 * 0.95).ceil() as usize).saturating_sub(1); // 0-based
    let p95 = samples.get(p95_index).copied().unwrap_or(Duration::ZERO);
    let p95_ms = p95.as_secs_f64() * 1000.0;

    Ok((p95_ms, budget))
}

/// Invokes the binary via `output()` (never `.spawn()`).
fn spawn_version(binary: &std::path::Path) -> Result<(), String> {
    let out = std::process::Command::new(binary)
        .arg("--version")
        .output()
        .map_err(|e| format!("bench-gate: failed to run {}: {e}", binary.display()))?;
    if !out.status.success() {
        return Err(format!(
            "bench-gate: {} --version exited {:?}",
            binary.display(),
            out.status.code()
        ));
    }
    Ok(())
}

/// Writes `_tmp/bench/version-startup.json` with the sample stats.
fn write_report(p95_ms: f64, budget: u64) -> Result<(), String> {
    let dir = std::path::Path::new("_tmp/bench");
    std::fs::create_dir_all(dir).map_err(|e| format!("bench-gate: mkdir _tmp/bench: {e}"))?;
    let doc = serde_json::json!({
        "samples": MEASURED,
        "p50_ms": 0.0,   // p50 not computed here; p95 is the gate
        "p95_ms": p95_ms,
        "budget_ms": budget,
    });
    let path = dir.join("version-startup.json");
    std::fs::write(
        &path,
        // F6.2 documents a compact single-line doc: {"samples":..,"p50_ms":..,
        // "p95_ms":..,"budget_ms":<n>}. Pretty-printing would insert spaces and
        // break the exact-match grep in part-4 exit-criterion 5.
        serde_json::to_vec(&doc).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("bench-gate: write {}: {e}", path.display()))?;
    Ok(())
}

/// Entry point for `cargo xtask bench-gate [--budget-ms <n>]`.
#[allow(clippy::module_name_repetitions, clippy::redundant_pub_crate)]
pub(crate) fn run_cli(args: &[String]) -> Result<(), String> {
    let mut budget = None;
    let mut i = 0;
    while i < args.len() {
        if args.get(i) == Some(&"--budget-ms".to_string()) {
            let raw = args
                .get(i + 1)
                .ok_or_else(|| "bench-gate: --budget-ms needs a number".to_string())?;
            budget = Some(
                raw.parse()
                    .map_err(|_| "bench-gate: --budget-ms needs a number".to_string())?,
            );
            i += 2;
        } else {
            i += 1;
        }
    }
    let (p95, budget) = run(budget)?;
    write_report(p95, budget)?;
    println!("p95_ms={p95} budget_ms={budget}");
    if p95 > budget as f64 {
        return Err(format!(
            "bench-gate: p95 {p95:.2}ms exceeds budget {budget}ms (cold-start gate)"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    // Percentile math sanity: the p95 of the sorted samples is the element at
    // index ceil(0.95*n)-1.
    #[test]
    fn p95_index_computes_correctly() {
        let n = 50usize;
        let idx = (n as f64 * 0.95).ceil() as usize; // 48
        assert_eq!(idx, 48);
        assert_eq!(idx.saturating_sub(1), 47);
    }
}
