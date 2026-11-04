//! Spec 371 R3: `max_in_flight` is enforced. A second invoke beyond the limit is refused with a typed error
//! (and counted); once the in-flight request completes, the slot is free again.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pii_abi::PiiError;
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_worker::{Supervisor, WorkerSpec};

/// The `pii` binary beside the running test executable (as `supervisor.rs` resolves it).
fn pii_bin() -> PathBuf {
    let mut dir = std::env::current_exe().expect("the test executable's path");
    dir.pop();
    if dir.ends_with("deps") {
        dir.pop();
    }
    dir.join("pii")
}

fn full_ctx() -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    caps.proc_argv0 = ArgvScope::new(vec![pii_bin().to_string_lossy().into_owned()]);
    Ctx::root(caps, Budget::unlimited())
}

fn worker_spec(max_in_flight: u32) -> WorkerSpec {
    WorkerSpec {
        name: String::from("w"),
        program: pii_bin(),
        args: vec![
            String::from("worker-exec"),
            String::from("--name"),
            String::from("w"),
            String::from("--tag"),
            String::from("a"),
        ],
        env: Vec::new(),
        max_in_flight,
        ready_timeout_ms: 5_000,
        restart_backoff_ms: 50,
        restart_window_ms: 60_000,
        max_restarts_in_window: 5,
    }
}

#[test]
fn a_second_invoke_beyond_max_in_flight_is_refused() {
    let ctx = full_ctx();
    let sup = Supervisor::new(worker_spec(1));
    sup.start(&ctx).unwrap();

    let holder = sup.clone();
    let holder_ctx = ctx.clone();
    let first = std::thread::spawn(move || {
        holder.invoke(
            &holder_ctx,
            "sleep",
            serde_json::json!({"ms": 1000}),
            10_000,
        )
    });
    let until = Instant::now() + Duration::from_secs(5);
    while sup.info().in_flight == 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(sup.info().in_flight, 1, "the first invoke is in flight");

    let err = sup
        .invoke(&ctx, "echo", serde_json::json!({"v": 1}), 5_000)
        .unwrap_err();
    assert!(
        matches!(&err, PiiError::Unavailable(m) if m.contains("max_in_flight")),
        "{err:?}"
    );
    assert_eq!(sup.refused_count(), 1, "the refusal is counted");

    first.join().unwrap().unwrap();
    sup.invoke(&ctx, "echo", serde_json::json!({"v": 2}), 5_000)
        .unwrap();
}
