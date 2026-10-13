//! Spec 116 R3: where the bytes of a run go. Measures the process's own
//! `/proc/self/io` around store operations, so the breakdown is bytes the
//! kernel was handed, not an estimate. Run with `--nocapture` to read it.

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

fn io() -> (u64, u64) {
    let s = std::fs::read_to_string("/proc/self/io").unwrap();
    let get = |k: &str| {
        s.lines()
            .find_map(|l| l.strip_prefix(k))
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap()
    };
    (get("write_bytes:"), get("wchar:"))
}

#[test]
fn per_run_store_bytes_by_writer() {
    let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("write-amp-")
        .tempdir_in(root)
        .unwrap();
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let spec = load_toml(LEAN).unwrap();
    let reg = NodeRegistry::builtin();
    let n: u64 = std::env::var("PII_PROBE_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let (mut id_w, mut id_c) = (0, 0);
    let (mut run_w, mut run_c) = (0, 0);
    let txn0 = store.write_txn_count();
    for _ in 0..n {
        let a = io();
        let run_id = store.next_run_id().unwrap();
        let b = io();
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
        let c = io();
        id_w += b.0 - a.0;
        id_c += b.1 - a.1;
        run_w += c.0 - b.0;
        run_c += c.1 - b.1;
    }
    let txns = store.write_txn_count() - txn0;
    // Hundredths of a commit per run, in integers.
    let per_run = txns * 100 / n;
    println!(
        "runs={n} write_txns/run={}.{:02}",
        per_run / 100,
        per_run % 100
    );
    // Two commits per run: the synchronous acceptance write (header + `Started`,
    // spec 154) and the journal append, plus the amortised id blocks and the
    // first sight of the graph and input bodies. It used to be 4.
    assert!(
        per_run < 220,
        "a run costs {per_run}/100 commits; expected ~2"
    );
    println!(
        "next_run_id: write_bytes/run={} wchar/run={}",
        id_w / n,
        id_c / n
    );
    println!(
        "run (journal+header+status): write_bytes/run={} wchar/run={}",
        run_w / n,
        run_c / n
    );
    let first = store
        .read_journal(&pii_abi::RunId::from("r000000000001"))
        .unwrap();
    println!("journal events/run={}", first.len());
}
