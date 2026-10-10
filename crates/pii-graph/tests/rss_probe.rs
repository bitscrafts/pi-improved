//! Spec 116 R1: the store-only leg of the F2 bisect. Drives `PII_PROBE_RUNS`
//! runs (default 20 000) straight through the store and the graph runner — no
//! engine, no bus, no observer, no connection — and prints `VmRSS` every
//! `PII_PROBE_EVERY` runs. If the daemon's residual per-run growth is here, it
//! lives in the store/runner; if not, it is in the server.
//!
//! `cargo test -p pii-graph --test rss_probe -- --ignored --nocapture`

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

use pii_core::{Budget, CapSet, Ctx, NullObserver, Observer, TraceCtx};
use pii_graph::node::NodeRegistry;
use pii_graph::run;
use pii_graph::toml_load::load_toml;
use pii_store::Store;

const LEAN: &str = "name = \"lean\"\nentry = \"step\"\ncaps = []\n[limits]\nmax_steps = 100\nmax_wall_ms = 30000\n[[node]]\nid = \"step\"\nkind = \"transform\"\nvalue = \"ok\"\n";

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
#[ignore = "a measurement, minutes long: run it explicitly"]
fn store_only_rss_series() {
    let (n, every) = (
        env_num("PII_PROBE_RUNS", 20_000),
        env_num("PII_PROBE_EVERY", 2_000),
    );
    let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("rss-probe-")
        .tempdir_in(root)
        .unwrap();
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let spec = load_toml(LEAN).unwrap();
    let reg = NodeRegistry::builtin();
    println!("runs rss_kb");
    println!("0 {}", rss_kb());
    for i in 1..=n {
        let run_id = store.next_run_id().unwrap();
        let ctx = Ctx::with_observer(
            CapSet::empty(),
            Budget::unlimited(),
            Arc::new(NullObserver) as Arc<dyn Observer>,
            TraceCtx {
                run: run_id.clone(),
                span: 0,
            },
        );
        run::run(&store, &spec, &reg, &ctx, &run_id, serde_json::json!({})).unwrap();
        if i % every == 0 {
            println!("{i} {}", rss_kb());
        }
    }
}
