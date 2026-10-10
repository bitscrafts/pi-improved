//! Spec 116 R1: the trajectory-sink leg of the F2 bisect. Feeds the sink
//! `PII_PROBE_RUNS` (default 150 000) runs of four events each — no daemon, no
//! store — and prints `VmRSS` every `PII_PROBE_EVERY` runs, past the default
//! 50 000-file retention bound so the plateau (or its absence) is visible.
//!
//! `cargo test -p pii-observe --test rss_probe -- --ignored --nocapture`

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_macros,
    clippy::print_stdout,
    clippy::disallowed_methods
)]

use std::sync::Arc;

use pii_observe::{
    Event, Level, NoRedaction, Observer, Outcome, Retention, RunId, TrajectorySink, Verdict,
};

fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap()
}

fn env_num(k: &str, d: u64) -> u64 {
    std::env::var(k)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(d)
}

#[test]
#[ignore = "a measurement: run it explicitly with --nocapture"]
fn trajectory_sink_rss_series() {
    let (n, every) = (
        env_num("PII_PROBE_RUNS", 150_000),
        env_num("PII_PROBE_EVERY", 10_000),
    );
    let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("rss-probe-")
        .tempdir_in(root)
        .unwrap();
    // The default retention (spec 115): 50 000 files, 512 MiB.
    let sink = TrajectorySink::new(dir.path(), Arc::new(NoRedaction)).with_retention(Retention {
        max_bytes: Some(512 << 20),
        max_files: Some(50_000),
    });
    println!("runs rss_kb tracked_runs");
    println!("0 {} {}", rss_kb(), sink.tracked_runs());
    for i in 1..=n {
        let run = RunId(format!("r{i:012}"));
        for _ in 0..3 {
            sink.event(&Event::new("graph", Level::Info, "run.journal").in_run(run.clone()));
        }
        let done = Outcome {
            verdict: Verdict::Success,
            reason: "t".into(),
            detail: serde_json::Value::Null,
        };
        sink.event(&done.into_event("graph").in_run(run));
        if i % every == 0 {
            println!("{i} {} {}", rss_kb(), sink.tracked_runs());
        }
    }
}
