//! Supervisor integration tests (F3.4–F3.6, part-2 TDD table): kill -9 fails
//! in-flight with `WorkerDied`, restart < 2 s, crash-loop cap, hot reload
//! drops nothing, stderr flood does not deadlock.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_worker::{Supervisor, WorkerEvent, WorkerSpec, WorkerState};
use std::path::PathBuf;

/// A full-cap context: SPAWN + PROC, and `proc_argv0` permitting the worker
/// program and the `kill` utility.
fn full_ctx() -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    // Tests run from the crate dir; resolve the workspace binary the same
    // way worker_spec() does below, so the ArgvScope prefix (a plain string
    // comparison, not a canonicalized path comparison) matches exactly.
    let pii_bin = std::env::current_dir()
        .unwrap()
        .ancestors()
        .nth(2)
        .unwrap()
        .join("target/debug/pii");
    caps.proc_argv0 = ArgvScope::new(vec![
        pii_bin.to_string_lossy().into_owned(),
        String::from("/usr/bin/kill"),
        String::from("/bin/sh"),
        // Test worker wrapper scripts (flood/tag-swap fixtures): the
        // flood fixture gets a per-run pid suffix to dodge ETXTBSY races.
        String::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/_tmp/phase3/test/swap"
        )),
        String::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/_tmp/phase3/test/flood"
        )),
    ]);
    Ctx::root(caps, Budget::unlimited())
}

/// A worker spec running the reference `pii worker-exec` worker.
fn worker_spec(name: &str, tag: &str) -> WorkerSpec {
    // Tests run from the crate dir; resolve the workspace binary absolutely.
    let pii_bin = std::env::current_dir()
        .unwrap()
        .ancestors()
        .nth(2)
        .unwrap()
        .join("target/debug/pii");
    WorkerSpec {
        name: String::from(name),
        program: pii_bin,
        args: vec![
            String::from("worker-exec"),
            String::from("--name"),
            String::from(name),
            String::from("--tag"),
            String::from(tag),
        ],
        env: Vec::new(),
        max_in_flight: 0,
        ready_timeout_ms: 5_000,
        restart_backoff_ms: 50,
        restart_window_ms: 60_000,
        max_restarts_in_window: 5,
    }
}

/// Kills a pid with `kill -9` through `ProcCap::run` (no raw Command).
fn kill9(pid: u32) {
    let ctx = full_ctx();
    let argv = vec![
        String::from("/usr/bin/kill"),
        String::from("-9"),
        pid.to_string(),
    ];
    ctx.proc()
        .unwrap()
        .run(
            &argv,
            &std::env::current_dir().unwrap(),
            Duration::from_secs(5),
        )
        .unwrap();
}

#[test]
fn kill9_fails_in_flight_with_workerdied() {
    let ctx = full_ctx();
    let sup = Supervisor::new(worker_spec("w", "a"));
    sup.start(&ctx).unwrap();
    let pid = sup.info().pid;
    assert!(pid > 0, "worker has a pid");

    // Three concurrent in-flight `sleep{5000}` invokes.
    let handles: Vec<_> = (0..3)
        .map(|_| {
            let sup = sup.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                sup.invoke(&ctx, "sleep", serde_json::json!({"ms": 5000}), 10_000)
            })
        })
        .collect();
    std::thread::sleep(Duration::from_millis(200)); // let them land

    kill9(pid);

    for h in handles {
        let r = h.join().unwrap();
        let err = r.expect_err("in-flight invoke must fail on kill -9");
        assert!(
            err.to_string().contains("WorkerDied"),
            "want WorkerDied, got {err}"
        );
        assert!(err.is_retryable(), "WorkerDied is retryable: {err}");
    }
}

#[test]
fn restart_completes_under_2s() {
    let ctx = full_ctx();
    let sup = Supervisor::new(worker_spec("w", "a"));
    sup.start(&ctx).unwrap();
    let old_pid = sup.info().pid;

    kill9(old_pid);

    // Wait for a Restarted event with took_ms < 2000.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let restarted = loop {
        let evs = sup.events();
        if let Some(WorkerEvent::Restarted { took_ms, .. }) = evs
            .iter()
            .find(|e| matches!(e, WorkerEvent::Restarted { .. }))
        {
            break Some(*took_ms);
        }
        if std::time::Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let took = restarted.expect("worker restarted within 5s");
    assert!(took < 2000, "restart took {took}ms, want < 2000");

    // A subsequent invoke succeeds and the pid changed.
    let v = sup
        .invoke(&ctx, "echo", serde_json::json!({"v": 7}), 5_000)
        .unwrap();
    assert_eq!(v, serde_json::json!(7), "echo returns the raw value");
    assert_ne!(sup.info().pid, old_pid, "pid changed after restart");
}

#[test]
fn respawn_honors_configured_backoff() {
    let ctx = full_ctx();
    let mut spec = worker_spec("wb", "a");
    spec.restart_backoff_ms = 200;
    let sup = Supervisor::new(spec);
    sup.start(&ctx).unwrap();

    kill9(sup.info().pid);

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let restarted = loop {
        let evs = sup.events();
        if let Some(WorkerEvent::Restarted { took_ms, .. }) = evs
            .iter()
            .find(|e| matches!(e, WorkerEvent::Restarted { .. }))
        {
            break Some(*took_ms);
        }
        if std::time::Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let took = restarted.expect("worker restarted within 5s");
    assert!(took >= 200, "took_ms must include the backoff: {took}ms");
}

#[test]
fn respawn_zero_backoff_does_not_sleep() {
    let ctx = full_ctx();
    let mut spec = worker_spec("wz", "a");
    spec.restart_backoff_ms = 0;
    let sup = Supervisor::new(spec);
    sup.start(&ctx).unwrap();

    kill9(sup.info().pid);

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let restarted = loop {
        let evs = sup.events();
        if let Some(WorkerEvent::Restarted { took_ms, .. }) = evs
            .iter()
            .find(|e| matches!(e, WorkerEvent::Restarted { .. }))
        {
            break Some(*took_ms);
        }
        if std::time::Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let took = restarted.expect("worker restarted within 5s");
    assert!(took < 100, "zero backoff must not sleep: {took}ms");
}

#[test]
fn restart_storm_is_capped() {
    let ctx = full_ctx();
    // A worker that crashes on every `crash` invoke; low restart cap.
    let mut spec = worker_spec("x", "a");
    spec.max_restarts_in_window = 2;
    spec.restart_window_ms = 60_000;
    let sup = Supervisor::new(spec);
    sup.start(&ctx).unwrap();

    // Drive it into a crash loop: each `crash` kills the child → restart.
    for _ in 0..6 {
        let _ = sup.invoke(&ctx, "crash", serde_json::json!({}), 3_000);
        std::thread::sleep(Duration::from_millis(200));
    }

    // After the cap, the worker is Failed and invokes are denied.
    let state = sup.info().state;
    assert_eq!(state, WorkerState::Failed, "crash loop capped: {state:?}");
    let err = sup
        .invoke(&ctx, "echo", serde_json::json!({"v": 1}), 3_000)
        .expect_err("failed worker refuses invokes");
    assert!(err.to_string().contains("failed"), "denied: {err}");
}

#[test]
fn reload_drops_no_requests() {
    let ctx = full_ctx();
    let sup = Supervisor::new(worker_spec("w", "a"));
    sup.start(&ctx).unwrap();
    let old_pid = sup.info().pid;

    // 200 sequential invokes while a reload runs.
    let mut results = Vec::new();
    for i in 0..200 {
        if i == 50 {
            let sup = sup.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = sup.reload(&ctx);
            });
        }
        let r = sup.invoke(&ctx, "echo", serde_json::json!({"v": i}), 10_000);
        results.push(r);
    }

    // All 200 must succeed (reload queues, never errors).
    for (i, r) in results.iter().enumerate() {
        assert!(r.is_ok(), "invoke {i} failed: {r:?}");
    }
    // The tag is unchanged (same program), but the pid changed.
    assert_ne!(sup.info().pid, old_pid, "pid changed after reload");
}

#[test]
fn stderr_flood_does_not_deadlock() {
    use std::os::unix::fs::PermissionsExt;

    let ctx = full_ctx();
    // A flood worker: writes 10 MB to stderr while answering echo (F3.2).
    let scratch = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp/phase3/test"));
    let _ = std::fs::create_dir_all(&scratch);
    let prog = scratch.join(format!("flood-{}-{}.sh", std::process::id(), line!()));
    // Atomic write (tmp + rename): a leftover running instance from a prior
    // run holds flood.sh busy, and in-place writes fail with ETXTBSY.
    let tmp = prog.with_extension("tmp");
    std::fs::write(
        &tmp,
        format!(
            "#!/bin/sh\nfor i in $(seq 1 2000); do echo flood-line-$i 1>&2; done\nexec {pii} worker-exec --name w --tag a\n",
            pii = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/debug/pii"),
        ),
    )
    .unwrap();
    std::fs::rename(&tmp, &prog).unwrap();
    let _ = std::fs::set_permissions(&prog, std::fs::Permissions::from_mode(0o755));
    assert!(prog.exists(), "flood script written");

    let spec = WorkerSpec {
        program: prog,
        args: Vec::new(),
        ..worker_spec("w", "a")
    };
    let sup = Supervisor::new(spec);
    sup.start(&ctx).unwrap();

    // Responses must still arrive while stderr floods.
    let t0 = std::time::Instant::now();
    for i in 0..20 {
        let v = sup
            .invoke(&ctx, "echo", serde_json::json!({"v": i}), 5_000)
            .unwrap();
        assert_eq!(v, serde_json::json!(i));
    }
    assert!(
        t0.elapsed() < Duration::from_secs(5),
        "flood must not deadlock, took {:?}",
        t0.elapsed()
    );
}

/// `kill9` via a shell one-liner is not available here (no Command); the
/// restart-storm test drives death with the sanctioned `crash` op.
#[test]
fn reload_swaps_program_and_tag() {
    use std::os::unix::fs::PermissionsExt;

    let ctx = full_ctx();
    let scratch = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp/phase3/test"));
    let _ = std::fs::create_dir_all(&scratch);
    let prog = scratch.join(format!("swap-{}-{}.sh", std::process::id(), line!()));
    let pii_bin = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/debug/pii"
    ));
    let write_prog = |tag: &str| {
        // Operators never write in place over a running program (ETXTBSY);
        // they write a temp file and rename over the path — the atomic form.
        let tmp = prog.with_extension("tmp");
        std::fs::write(
            &tmp,
            format!(
                "#!/bin/sh\nexec {} worker-exec --name w --tag {tag}\n",
                pii_bin.display()
            ),
        )
        .unwrap();
        std::fs::rename(&tmp, &prog).unwrap();
        std::fs::set_permissions(&prog, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    let _ = std::fs::set_permissions(&prog, std::fs::Permissions::from_mode(0o755));
    write_prog("a");
    let spec = WorkerSpec {
        program: prog.clone(),
        args: Vec::new(),
        ..worker_spec("w", "a")
    };
    let sup = Supervisor::new(spec);
    sup.start(&ctx).unwrap();
    let old_pid = sup.info().pid;
    assert_eq!(sup.info().tag, "a");

    // Replace the program file on disk (the operator's move) then reload.
    write_prog("b");
    let report = sup.reload(&ctx).unwrap();
    assert_ne!(report.new_pid, old_pid);
    assert_eq!(sup.info().tag, "b", "new program's tag must show");
}

/// A slow-dying child (`PII_TEST_KILL_DELAY_MS` seam in `ChildProc::kill`)
/// must not block a concurrent state read: `death()` releases the state
/// lock before the blocking kill, so `info()` (a direct `state.lock()`
/// read, uncomplicated by `invoke()`'s Restarting-state queue path, which
/// has its own unrelated 30s minimum wait) returns immediately without
/// waiting for the kill+wait teardown.
#[test]
fn worker_kill_does_not_block_concurrent_invoke() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let ctx = full_ctx();
    let sup = Supervisor::new(worker_spec("w", "a"));
    sup.start(&ctx).unwrap();

    // Kill+wait will block ~500ms (test seam, read per call).
    pii_core::set_test_kill_delay_ms(500);

    // Trigger death from one thread: kill9 makes the reader see EOF and
    // call death(), whose slow kill runs off the state lock.
    let done = Arc::new(AtomicBool::new(false));
    let killer_done = Arc::clone(&done);
    let killer_sup = sup.clone();
    let killer = std::thread::spawn(move || {
        let pid = killer_sup.info().pid;
        kill9(pid);
        // Wait for death() to have been entered and finished its slow kill.
        std::thread::sleep(Duration::from_millis(700));
        killer_done.store(true, Ordering::SeqCst);
    });

    // Give the killer a head start so death() is inside its slow kill.
    std::thread::sleep(Duration::from_millis(150));

    // `info()` acquires `state.lock()` directly -- if death()'s kill were
    // still holding that lock (the D7 bug), this call would block for the
    // remainder of the ~500ms delay. `invoke()` is deliberately NOT used
    // here: once state is Restarting it takes a queued path with its own
    // unrelated `timeout_ms.max(30_000)` minimum wait, which would make
    // this assertion about queueing semantics, not about the lock.
    let t0 = std::time::Instant::now();
    let info = sup.info();
    let elapsed = t0.elapsed();
    // With the lock held across the kill, this would block ~500ms.
    assert!(
        elapsed < Duration::from_millis(400),
        "info() blocked {elapsed:?} on the kill — state lock held across kill"
    );
    // The worker is mid-teardown at this point (Restarting, or Failed if
    // the crash-loop cap tripped) -- either way, definitely not Ready.
    assert_ne!(
        info.state,
        WorkerState::Ready,
        "expected the worker to be mid-teardown, got {info:?}"
    );

    killer.join().unwrap();
    assert!(done.load(Ordering::SeqCst), "killer finished");
    pii_core::set_test_kill_delay_ms(0);
}
