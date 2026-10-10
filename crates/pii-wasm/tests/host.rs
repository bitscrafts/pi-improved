//! Spec 122: the plugin host's limits and refusals, one test each, on
//! hand-written WAT compiled at test time. Offline; nothing here fetches a
//! module. Each limit/refusal test fails if its enforcement is removed (see
//! the mutations pasted in the spec report).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout,
    clippy::disallowed_macros
)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pii_abi::PiiError;
use pii_observe::{Health, Recorder, State, names};
use pii_wasm::{CallOpts, Fuel, WasmConfig, WasmHost, WasmHub};
use serde_json::json;

/// Shared preamble: one page of memory, a bump allocator at 1024.
const PRE: &str = r#"
  (memory (export "memory") 1)
  (global $bump (mut i32) (i32.const 1024))
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $bump))
    (global.set $bump (i32.add (global.get $bump) (local.get $n)))
    (local.get $p))
"#;

fn module(body: &str) -> String {
    format!("(module {PRE} {body})")
}

/// `run` echoes the input bytes back.
fn echo() -> String {
    module(
        r#"(func (export "run") (param $p i32) (param $n i32) (result i64)
             (i64.or (i64.shl (i64.extend_i32_u (local.get $p)) (i64.const 32))
                     (i64.extend_i32_u (local.get $n))))"#,
    )
}

fn spin() -> String {
    module(
        r#"(func (export "run") (param i32 i32) (result i64)
             (loop $l (br $l))
             (i64.const 0))"#,
    )
}

fn grow_forever() -> String {
    module(
        r#"(func (export "run") (param i32 i32) (result i64)
             (loop $l (drop (memory.grow (i32.const 1))) (br $l))
             (i64.const 0))"#,
    )
}

fn trap() -> String {
    module(
        r#"(func $boom (unreachable))
           (func (export "run") (param i32 i32) (result i64)
             (call $boom) (i64.const 0))"#,
    )
}

/// Returns the 8 bytes "not json" stored at offset 2048.
fn garbage() -> String {
    module(
        r#"(data (i32.const 2048) "not json")
           (func (export "run") (param i32 i32) (result i64)
             (i64.or (i64.shl (i64.const 2048) (i64.const 32)) (i64.const 8)))"#,
    )
}

/// Returns a pointer far outside its one page of memory.
fn wild_pointer() -> String {
    module(
        r#"(func (export "run") (param i32 i32) (result i64)
             (i64.or (i64.shl (i64.const 4000000) (i64.const 32)) (i64.const 10)))"#,
    )
}

/// Tries to open a file through WASI: imports `path_open` and calls it.
fn fs_open() -> String {
    format!(
        r#"(module
  (import "wasi_snapshot_preview1" "path_open"
    (func $open (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))
  {PRE}
  (data (i32.const 0) "/etc/passwd")
  (func (export "run") (param i32 i32) (result i64)
    (drop (call $open (i32.const 3) (i32.const 0) (i32.const 0) (i32.const 11)
                      (i32.const 0) (i64.const 2) (i64.const 0) (i32.const 0) (i32.const 64)))
    (i64.const 0)))"#
    )
}

/// Initial memory (100 pages = 6.4 MB) far above a 1 MiB cap.
fn big_initial_memory() -> String {
    r#"(module
  (memory (export "memory") 100)
  (func (export "alloc") (param i32) (result i32) (i32.const 0))
  (func (export "run") (param i32 i32) (result i64) (i64.const 0)))"#
        .to_owned()
}

struct Rig {
    host: WasmHost,
    rec: Arc<Recorder>,
    dir: tempfile::TempDir,
}

fn write(dir: &Path, name: &str, wat: &str) {
    std::fs::write(dir.join(name), wat::parse_str(wat).unwrap()).unwrap();
}

fn rig_with(tune: impl FnOnce(&mut WasmConfig)) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = WasmConfig::new(dir.path());
    cfg.timeout = Duration::from_millis(400);
    cfg.max_memory = 1024 * 1024;
    tune(&mut cfg);
    let rec = Arc::new(Recorder::new());
    let host = WasmHost::new(cfg, Some(rec.clone())).unwrap();
    for (n, w) in [
        ("echo.wasm", echo()),
        ("spin.wasm", spin()),
        ("grow.wasm", grow_forever()),
        ("trap.wasm", trap()),
        ("garbage.wasm", garbage()),
        ("wild.wasm", wild_pointer()),
        ("fsopen.wasm", fs_open()),
        ("bigmem.wasm", big_initial_memory()),
    ] {
        write(dir.path(), n, &w);
    }
    Rig { host, rec, dir }
}

fn rig() -> Rig {
    rig_with(|_| {})
}

fn reason(r: &Recorder, name: &str) -> String {
    let e = r.events_named(name);
    assert_eq!(e.len(), 1, "exactly one {name}: {:?}", r.events());
    assert!(e[0].fields.get("module").is_some(), "module field");
    assert!(e[0].fields.get("entry").is_some(), "entry field");
    e[0].fields
        .get("reason")
        .map(|v| v.as_str().unwrap_or_default().to_owned())
        .unwrap_or_default()
}

#[test]
fn echo_round_trips_json_and_emits_load_instantiate_call() {
    let r = rig();
    let input = json!({"a": [1, 2, {"b": "c"}], "n": null});
    let out = r
        .host
        .call("echo.wasm", &input, &CallOpts::default())
        .unwrap();
    assert_eq!(out, input);
    let kinds: Vec<_> = r.rec.events().into_iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [names::WASM_LOAD, names::WASM_INSTANTIATE, names::WASM_CALL]
    );
    let c = r.host.counts();
    assert_eq!((c.ok, c.refusals), (1, 0));
}

#[test]
fn an_infinite_loop_is_interrupted_by_the_epoch_deadline_with_a_named_timeout() {
    let r = rig();
    let t = Instant::now();
    let err = r
        .host
        .call("spin.wasm", &json!({}), &CallOpts::default())
        .unwrap_err();
    let elapsed = t.elapsed();
    println!("EPOCH: infinite loop interrupted after {elapsed:?} (budget 400ms)");
    assert_eq!(err, PiiError::Timeout { after_ms: 400 }, "{err}");
    assert!(elapsed >= Duration::from_millis(350), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
    let ev = &r.rec.events_named(names::WASM_TIMEOUT)[0];
    assert!(ev.fields.get("elapsed_ms").is_some() && ev.fields.get("module").is_some());
    assert_eq!(r.host.counts().timeout, 1);
}

#[test]
fn a_node_may_lower_the_budget_but_not_raise_it() {
    let r = rig();
    let lower = CallOpts {
        timeout: Some(Duration::from_millis(100)),
        ..CallOpts::default()
    };
    let err = r.host.call("spin.wasm", &json!({}), &lower).unwrap_err();
    assert_eq!(err, PiiError::Timeout { after_ms: 100 });
    let higher = CallOpts {
        timeout: Some(Duration::from_mins(1)),
        ..CallOpts::default()
    };
    let err = r.host.call("spin.wasm", &json!({}), &higher).unwrap_err();
    assert_eq!(
        err,
        PiiError::Timeout { after_ms: 400 },
        "clamped to the host ceiling"
    );
}

#[test]
fn a_module_that_burns_fuel_is_stopped_before_the_clock_runs_out() {
    let r = rig_with(|c| c.timeout = Duration::from_secs(20));
    let opts = CallOpts {
        fuel: Fuel::Limit(100_000),
        ..CallOpts::default()
    };
    let t = Instant::now();
    let err = r.host.call("spin.wasm", &json!({}), &opts).unwrap_err();
    println!("FUEL: exhausted after {:?} (clock budget 20s)", t.elapsed());
    assert!(
        matches!(&err, PiiError::BudgetExhausted(m) if m.contains("fuel")),
        "{err}"
    );
    assert!(t.elapsed() < Duration::from_secs(5));
    assert_eq!(r.host.counts().fuel, 1);
    assert_eq!(r.rec.events_named(names::WASM_FUEL_EXHAUSTED).len(), 1);
}

#[test]
fn fuel_enabled_without_a_number_uses_the_default_ceiling() {
    let r = rig_with(|c| {
        c.timeout = Duration::from_secs(20);
        c.fuel = 200_000;
    });
    let opts = CallOpts {
        fuel: Fuel::Default,
        ..CallOpts::default()
    };
    let err = r.host.call("spin.wasm", &json!({}), &opts).unwrap_err();
    assert!(matches!(err, PiiError::BudgetExhausted(_)), "{err}");
    // A limit above the ceiling is clamped to it, not honoured.
    let opts = CallOpts {
        fuel: Fuel::Limit(u64::MAX),
        ..CallOpts::default()
    };
    let err = r.host.call("spin.wasm", &json!({}), &opts).unwrap_err();
    assert!(matches!(err, PiiError::BudgetExhausted(_)), "{err}");
}

#[test]
fn unbounded_memory_growth_is_refused_not_granted() {
    let r = rig();
    let err = r
        .host
        .call("grow.wasm", &json!({}), &CallOpts::default())
        .unwrap_err();
    println!("MEMORY: {err}");
    assert!(
        matches!(&err, PiiError::BudgetExhausted(m) if m.contains("memory")),
        "{err}"
    );
    assert_eq!(r.host.counts().memory, 1);
    assert_eq!(r.rec.events_named(names::WASM_MEMORY_LIMIT).len(), 1);
}

#[test]
fn a_module_declaring_more_initial_memory_than_the_cap_is_refused() {
    let r = rig();
    let err = r
        .host
        .call("bigmem.wasm", &json!({}), &CallOpts::default())
        .unwrap_err();
    assert!(
        matches!(&err, PiiError::BudgetExhausted(m) if m.contains("memory")),
        "{err}"
    );
}

#[test]
fn a_module_that_imports_wasi_to_open_a_file_is_refused_before_it_runs() {
    let r = rig();
    let err = r
        .host
        .call("fsopen.wasm", &json!({}), &CallOpts::default())
        .unwrap_err();
    println!("NO AMBIENT AUTHORITY: {err}");
    assert!(
        matches!(&err, PiiError::Denied(m) if m.contains("path_open") && m.contains("no imports")),
        "{err}"
    );
    assert_eq!(reason(&r.rec, names::WASM_REFUSAL), "imports");
    assert_eq!(r.host.counts().refusals, 1);
    assert!(
        r.rec.events_named(names::WASM_INSTANTIATE).is_empty(),
        "refused before instantiation"
    );
}

#[test]
fn a_trap_is_reported_with_the_module_and_a_wasm_backtrace() {
    let r = rig();
    let err = r
        .host
        .call("trap.wasm", &json!({}), &CallOpts::default())
        .unwrap_err();
    let PiiError::Internal(m) = &err else {
        panic!("{err:?}")
    };
    println!("TRAP: {m}");
    assert!(m.contains("trap.wasm") && m.contains("unreachable"), "{m}");
    assert!(m.contains("wasm backtrace"), "{m}");
    let ev = &r.rec.events_named(names::WASM_TRAP)[0];
    assert!(
        ev.fields
            .get("backtrace")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("wasm backtrace")
    );
    assert_eq!(r.host.counts().trap, 1);
}

#[test]
fn a_module_outside_the_configured_dir_is_refused() {
    let r = rig();
    let other = tempfile::tempdir().unwrap();
    write(other.path(), "evil.wasm", &echo());
    let abs = other.path().join("evil.wasm");
    let err = r
        .host
        .call(abs.to_str().unwrap(), &json!({}), &CallOpts::default())
        .unwrap_err();
    assert!(
        matches!(&err, PiiError::Denied(m) if m.contains("outside")),
        "{err}"
    );
    assert_eq!(reason(&r.rec, names::WASM_REFUSAL), "out_of_scope");

    // `..` traversal out of the dir is the same refusal.
    let rel = format!(
        "../{}/evil.wasm",
        other.path().file_name().unwrap().to_str().unwrap()
    );
    let up = r.dir.path().parent().unwrap();
    assert!(up.join(&rel[3..]).exists() || other.path().parent() == Some(up));
    let err = r
        .host
        .call(&rel, &json!({}), &CallOpts::default())
        .unwrap_err();
    assert!(matches!(err, PiiError::Denied(_)), "{err}");
    assert_eq!(r.host.counts().refusals, 2);
}

#[test]
fn a_symlink_inside_the_dir_pointing_outside_is_refused() {
    let r = rig();
    let other = tempfile::tempdir().unwrap();
    write(other.path(), "evil.wasm", &echo());
    std::os::unix::fs::symlink(
        other.path().join("evil.wasm"),
        r.dir.path().join("link.wasm"),
    )
    .unwrap();
    let err = r
        .host
        .call("link.wasm", &json!({}), &CallOpts::default())
        .unwrap_err();
    assert!(
        matches!(&err, PiiError::Denied(m) if m.contains("outside")),
        "{err}"
    );
}

#[test]
fn a_module_returning_invalid_json_fails_the_call_with_a_named_error() {
    let r = rig();
    let err = r
        .host
        .call("garbage.wasm", &json!({}), &CallOpts::default())
        .unwrap_err();
    assert!(
        matches!(&err, PiiError::Invalid(m) if m.contains("invalid JSON")),
        "{err}"
    );
    assert_eq!(reason(&r.rec, names::WASM_REFUSAL), "invalid_output");
    assert_eq!(r.host.counts().invalid, 1);
}

#[test]
fn a_wild_output_pointer_is_an_error_not_a_host_panic() {
    let r = rig();
    let err = r
        .host
        .call("wild.wasm", &json!({}), &CallOpts::default())
        .unwrap_err();
    assert!(
        matches!(&err, PiiError::Invalid(m) if m.contains("outside its memory")),
        "{err}"
    );
}

#[test]
fn a_missing_entry_or_module_or_garbage_bytes_is_a_named_refusal() {
    let r = rig();
    let opts = CallOpts {
        entry: Some("nope".into()),
        ..CallOpts::default()
    };
    let err = r.host.call("echo.wasm", &json!({}), &opts).unwrap_err();
    assert!(
        matches!(&err, PiiError::Invalid(m) if m.contains("nope")),
        "{err}"
    );
    let err = r
        .host
        .call("absent.wasm", &json!({}), &CallOpts::default())
        .unwrap_err();
    assert!(
        matches!(err, PiiError::NotFound(_) | PiiError::Denied(_)),
        "{err}"
    );
    std::fs::write(r.dir.path().join("junk.wasm"), b"\0asm garbage").unwrap();
    let err = r
        .host
        .call("junk.wasm", &json!({}), &CallOpts::default())
        .unwrap_err();
    assert!(matches!(err, PiiError::Invalid(_)), "{err}");
}

#[test]
fn the_host_survives_a_hostile_module_and_serves_the_next_call() {
    let r = rig();
    let _ = r.host.call("trap.wasm", &json!({}), &CallOpts::default());
    let _ = r.host.call("spin.wasm", &json!({}), &CallOpts::default());
    let _ = r.host.call("grow.wasm", &json!({}), &CallOpts::default());
    let out = r
        .host
        .call("echo.wasm", &json!(7), &CallOpts::default())
        .unwrap();
    assert_eq!(out, json!(7));
}

#[test]
fn the_hub_is_off_until_configured_and_says_why() {
    let hub = WasmHub::default();
    let err = hub.require().err().unwrap();
    assert!(
        matches!(&err, PiiError::Denied(m) if m.contains("[wasm]")),
        "{err}"
    );
}

#[test]
fn health_reports_the_subsystem_limits_and_runtime_version() {
    let r = rig();
    let _ = r.host.call("echo.wasm", &json!(1), &CallOpts::default());
    let h = r.host.health();
    assert_eq!(h.name, "wasm");
    assert_eq!(h.state, State::Ok);
    assert_eq!(h.identity["runtime"], "wasmtime 48.0.5");
    assert_eq!(h.identity["timeout_ms"], "400");
    assert_eq!(h.identity["max_memory_bytes"], "1048576");
    assert_eq!(h.identity["calls_ok"], "1");
}

#[test]
fn the_pinned_wasmtime_version_matches_the_lockfile() {
    let lock =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock")).unwrap();
    let want = format!(
        "name = \"wasmtime\"\nversion = \"{}\"",
        pii_wasm::WASMTIME_VERSION
    );
    assert!(
        lock.contains(&want),
        "WASMTIME_VERSION drifted from Cargo.lock"
    );
}
