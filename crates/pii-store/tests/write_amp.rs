//! Spec 116 R3: what one run's journal commit writes, table by table.
//!
//! Each variant is its own commit against a store pre-filled to the depth a
//! busy store has, measured with the process's own `/proc/self/io`
//! (`write_bytes` = bytes the kernel was handed for the block layer). The
//! marginal cost of a table is the difference between cumulative variants.
//! Run it: `cargo test -p pii-store --test write_amp -- --ignored --nocapture`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_macros,
    clippy::print_stdout
)]

use pii_abi::{RunEvent, RunHeader, RunId, RunStatus};
use redb::WriteTransaction;

use pii_store::Store;
use redb::TableDefinition;

// The same definitions as `pii-store`'s private `tables` (redb matches by name and types).
const META: TableDefinition<'_, &str, u64> = TableDefinition::new("meta");
const RUN: TableDefinition<'_, &str, &[u8]> = TableDefinition::new("run");
const RUN_JOURNAL: TableDefinition<'_, (&str, u64), &[u8]> = TableDefinition::new("run_journal");
const RUN_BY_STATUS: TableDefinition<'_, (&str, &str), ()> = TableDefinition::new("run_by_status");

fn write_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/io")
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix("write_bytes:"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap()
}

fn header(id: &str) -> RunHeader {
    RunHeader {
        run: RunId::from(id),
        graph: "g".into(),
        status: RunStatus::Succeeded,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 0,
        created_ms: 0,
    }
}

fn events() -> Vec<RunEvent> {
    (0..5)
        .map(|i| RunEvent::NodeDead {
            node: format!("n{i}").into(),
        })
        .collect()
}

/// Mean `write_bytes` per commit of `f` over `n` commits, each on a fresh run id.
fn per_commit(db: &redb::Database, start: u64, f: impl Fn(&WriteTransaction, &str)) -> u64 {
    let n = 150;
    let mut total = 0;
    for i in 0..n {
        let id = format!("r{:012}", start + i);
        let before = write_bytes();
        let w = db.begin_write().unwrap();
        f(&w, &id);
        w.commit().unwrap();
        total += write_bytes() - before;
    }
    total / n
}

fn journal(w: &WriteTransaction, id: &str) {
    let ev = serde_json::to_vec(&events()[0]).unwrap();
    let mut t = w.open_table(RUN_JOURNAL).unwrap();
    for s in 0..5u64 {
        t.insert((id, s), ev.as_slice()).unwrap();
    }
}

fn run_header(w: &WriteTransaction, id: &str) {
    let bytes = serde_json::to_vec(&header(id)).unwrap();
    w.open_table(RUN)
        .unwrap()
        .insert(id, bytes.as_slice())
        .unwrap();
}

fn status(w: &WriteTransaction, id: &str) {
    let mut t = w.open_table(RUN_BY_STATUS).unwrap();
    for st in ["running", "suspended", "succeeded", "failed"] {
        t.remove((st, id)).unwrap();
    }
    t.insert(("succeeded", id), ()).unwrap();
}

/// The real `Store::append_events`, per run, for comparison with the parts.
fn real_append(store: &Store, start: u64) -> u64 {
    let n = 150;
    let before = write_bytes();
    for i in 0..n {
        let mut h = header(&format!("r{:012}", start + i));
        store
            .append_events(&mut h, &events().iter().collect::<Vec<_>>())
            .unwrap();
    }
    (write_bytes() - before) / n
}

#[test]
#[ignore = "a measurement: run it explicitly with --nocapture"]
fn write_amp_by_table() {
    let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp/pi-store-test"))
        .join(format!("write-amp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.redb");
    let store = Store::open(&path).unwrap();
    // Pre-fill: 1 000 finished runs of 5 events, so the trees have real depth.
    for _ in 0..1_000 {
        let id = store.next_run_id().unwrap();
        let mut h = header(id.as_str());
        store
            .append_events(&mut h, &events().iter().collect::<Vec<_>>())
            .unwrap();
    }
    let real = real_append(&store, 3_000_000);
    drop(store);
    let db = redb::Database::open(&path).unwrap();
    let base = 2_000_000;
    let fixed = per_commit(&db, base, |w, _| {
        w.open_table(META).unwrap().insert("probe", 1u64).unwrap();
    });
    let j = per_commit(&db, base + 1_000, journal);
    let jr = per_commit(&db, base + 2_000, |w, id| {
        journal(w, id);
        run_header(w, id);
    });
    let all = per_commit(&db, base + 3_000, |w, id| {
        journal(w, id);
        run_header(w, id);
        status(w, id);
    });
    println!("one-row META commit (the fixed cost of any commit): {fixed} B");
    println!(
        "journal x5 events   (cumulative): {j} B  marginal: {}",
        j.saturating_sub(fixed)
    );
    println!(
        "+ run header        (cumulative): {jr} B  marginal: {}",
        jr.saturating_sub(j)
    );
    println!(
        "+ run_by_status     (cumulative): {all} B  marginal: {}",
        all.saturating_sub(jr)
    );
    println!("Store::append_events (the real commit): {real} B");
}
