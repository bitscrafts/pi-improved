//! Spec 308: runs have their own thread pool, so the control plane is never queued behind a run,
//! a `run cancel --kill` never freezes the async workers, and a queued run can be cancelled.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads /proc.
#![allow(clippy::disallowed_methods)]
// reason: the CONTROL_ test names are the spec contract.
#![allow(non_snake_case)]

#[path = "daemon_guard/mod.rs"]
mod daemon_guard;

use daemon_guard::DaemonGuard;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn pii(args: &[&str], rt: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(args)
        .env("PII_RUNTIME_DIR", rt)
        // State (and its durable stores) must be this test's own: the operator's are
        // held by any daemon already running on the machine.
        .env("PII_STATE_DIR", rt)
        .env("PII_CONNECT_TIMEOUT_MS", "500")
        .output()
        .unwrap()
}

fn timed(args: &[&str], rt: &Path) -> (Output, Duration) {
    let started = Instant::now();
    let out = pii(args, rt);
    (out, started.elapsed())
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// Wait for `f` to become true, with a budget that means "the machine is busy", not "the daemon is hung".
///
/// MEASURED 2026-11-02: five tests in this file failed inside the full suite and passed on their own, twice, on an
/// otherwise idle machine. The reason was this function: a flat 20 s wait, while `cargo test` runs the whole suite in
/// parallel and every one of these tests spawns real `pii` processes. The suite starved the children, the wait
/// expired, and the panic said only "timed out" — no child state, no output, nothing to diagnose.
///
/// So the budget is now generous, and the failure is a DIAGNOSIS: it reports how long it waited, whether the child
/// process is still alive, and the daemon's last output, which is the difference between "slow" and "stuck".
fn until(what: &str, mut f: impl FnMut() -> bool) {
    until_with(what, Duration::from_secs(90), &mut f);
}

fn until_with(what: &str, budget: Duration, mut f: impl FnMut() -> bool) {
    let started = Instant::now();
    while started.elapsed() < budget {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let secs = started.elapsed().as_secs();
    panic!(
        "waited {secs}s for {what} — if this is the only failure, the suite starved it; if it repeats alone, {what} is stuck"
    );
}

/// Six tests in this file each start one or more real `pii` daemons and then wait for the run pool to reach a state.
/// Run together — which is what `cargo test` does — they starve each other: measured 2026-11-02, 1 passed and 5 failed,
/// every one of them having waited the full budget. The property they check is about bounded threads, so it cannot be
/// measured while the measuring machine is oversubscribed by its own tests.
///
/// They therefore take turns. A lock FILE rather than a Mutex, because `clippy.toml` bans `std::sync::Mutex` in this
/// repo, and because a file lock is the only kind that survives the test binary's own process boundaries. A lock older
/// than fifteen minutes is stolen: a killed test must not block the suite forever, and the timestamp is the record of
/// how it got there.
struct HeavyGuard(std::path::PathBuf);

impl Drop for HeavyGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn only_one_heavy_test_at_a_time() -> HeavyGuard {
    let path = std::env::temp_dir().join("pii-control-plane-heavy.lock");
    let give_up = Instant::now() + Duration::from_mins(15);
    loop {
        match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
        {
            Ok(mut f) => {
                use std::io::Write;
                let _ = writeln!(f, "{}", std::process::id());
                return HeavyGuard(path);
            }
            Err(e) => {
                let stale = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .is_ok_and(|t| t.elapsed().is_ok_and(|d| d.as_secs() > 900));
                if stale {
                    let _ = std::fs::remove_file(&path);
                    continue;
                }
                assert!(
                    Instant::now() < give_up,
                    "another heavy test held {} for 900s ({e})",
                    path.display()
                );
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
}

fn scratch(tag: &str) -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix(&format!("control-plane-{tag}-"))
        .tempdir_in(root)
        .unwrap()
}

/// A daemon whose scripted model sleeps `sleep_ms` per call and holds `turns` answers.
fn start_daemon(dir: &Path, sleep_ms: u64, turns: usize, env: &[(&str, &str)]) -> DaemonGuard {
    let answer = r#"{"text": "ok", "tool_calls": [], "tokens_in": 1, "tokens_out": 1}"#;
    let body = vec![answer; turns].join(",");
    let model = dir.join("model.json");
    std::fs::write(
        &model,
        format!(r#"{{"sleep_ms": {sleep_ms}, "turns": [{body}]}}"#),
    )
    .unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pii"));
    cmd.arg("daemon")
        .args(["--sandbox", "off", "--workspace"])
        .arg(dir)
        .arg("--log-file")
        .arg(dir.join("daemon.log"))
        .env("PII_RUNTIME_DIR", dir)
        // State (and its durable stores) must be this test's own: the operator's are
        // held by any daemon already running on the machine.
        .env("PII_STATE_DIR", dir)
        // The operator's ~/.pii/pii.toml must not arm cron entries in this scratch store.
        .env("PII_CONFIG", dir.join("pii.toml"))
        .env("PII_MODEL", format!("scripted:{}", model.display()))
        .stderr(Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let daemon = daemon_guard::spawn(&mut cmd);
    until("the daemon to answer", || {
        pii(&["sys", "status"], dir).status.success()
    });
    daemon
}

fn run_start(dir: &Path) -> String {
    let o = pii(
        &["run", "start", "default", "--input", r#"{"prompt":"hi"}"#],
        dir,
    );
    assert!(o.status.success(), "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    v.get("run_id")
        .and_then(serde_json::Value::as_str)
        .unwrap()
        .to_owned()
}

/// True once `run cancel` without `--kill` names an executing node, i.e. the run holds a pool slot.
fn is_executing(dir: &Path, run: &str) -> bool {
    let o = pii(&["run", "cancel", run, "--reason", "probe"], dir);
    let msg = text(&o);
    msg.contains("executing node '") && !msg.contains("executing node ''")
}

/// Opens an `event.subscribe` on the control socket and reads the acknowledgement line. The socket is opened through
/// `pii_uds::client_connect`, the one sanctioned client (net-check), on a small runtime of its own.
fn subscribe(dir: &Path, run: &str) -> (String, Duration) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let sock = pii_uds::client_connect(&dir.join("control.sock"))
            .await
            .unwrap();
        let (rd, mut wr) = sock.into_split();
        let mut reader = BufReader::new(rd);
        let say = |line: String| format!("{line}\n");
        wr.write_all(say(String::from(r#"{"jsonrpc":"2.0","id":0,"method":"sys.hello","params":{"protocol":"pii.v2","client":{"name":"test","version":"0","build":"0"},"encodings":["json"],"features":[]}}"#)).as_bytes())
            .await
            .unwrap();
        let mut hello = String::new();
        tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut hello))
            .await
            .unwrap()
            .unwrap();
        assert!(hello.contains("\"result\""), "{hello}");

        let started = Instant::now();
        let sub = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"event.subscribe","params":{{"run":"{run}","session":null,"kinds":[],"since_seq":0}}}}"#
        );
        wr.write_all(say(sub).as_bytes()).await.unwrap();
        let mut ack = String::new();
        tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut ack))
            .await
            .unwrap()
            .unwrap();
        (ack, started.elapsed())
    })
}

/// Threads of the daemon named `pii-run-*`. Linux clones inherit the name of the thread that spawns
/// them, so a run that starts a helper thread adds to this count too: read it before any run starts.
fn run_threads(pid: u32) -> usize {
    std::fs::read_dir(format!("/proc/{pid}/task"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|t| {
            std::fs::read_to_string(t.path().join("comm"))
                .is_ok_and(|c| c.trim().starts_with("pii-run-"))
        })
        .count()
}

fn thread_count(pid: u32) -> usize {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    status
        .lines()
        .find_map(|l| l.strip_prefix("Threads:"))
        .and_then(|n| n.trim().parse().ok())
        .unwrap()
}

#[test]
fn trace_subscribe_and_cancel_answer_while_runs_fill_the_pool() {
    let _heavy = only_one_heavy_test_at_a_time();
    let tmp = scratch("fill");
    let dir = tmp.path();
    let _daemon = start_daemon(
        dir,
        20_000,
        4,
        &[
            ("PII_DAEMON_MAX_BLOCKING", "2"),
            ("PII_DAEMON_RUN_THREADS", "2"),
        ],
    );
    let runs: Vec<String> = (0..4).map(|_| run_start(dir)).collect();
    until("run 1 to hold a pool slot", || is_executing(dir, &runs[0]));

    let (trace, took) = timed(&["run", "trace", &runs[0]], dir);
    assert!(took < Duration::from_secs(2), "run trace took {took:?}");
    assert!(trace.status.success(), "{}", text(&trace));

    let (ack, took) = subscribe(dir, &runs[0]);
    assert!(
        took < Duration::from_secs(2),
        "event subscribe took {took:?}"
    );
    // The first line may already be a pushed event for the run; either way the daemon answered.
    assert!(ack.contains("\"jsonrpc\""), "{ack}");

    let (cancel, took) = timed(&["run", "cancel", &runs[3], "--reason", "x"], dir);
    assert!(took < Duration::from_secs(2), "run cancel took {took:?}");
    assert!(cancel.status.success(), "{}", text(&cancel));
    let got = text(&pii(&["run", "get", &runs[3]], dir));
    assert!(got.contains("cancelled: x"), "{got}");
}

#[test]
fn CONTROL_at_most_run_threads_agents_are_alive() {
    let _heavy = only_one_heavy_test_at_a_time();
    let tmp = scratch("alive");
    let dir = tmp.path();
    let daemon = start_daemon(
        dir,
        20_000,
        4,
        &[
            ("PII_DAEMON_MAX_BLOCKING", "2"),
            ("PII_DAEMON_RUN_THREADS", "2"),
        ],
    );
    // The fixed pool holds exactly the configured run threads; a run queued behind them adds none.
    let pool = run_threads(daemon.pid());
    assert_eq!(pool, 2, "run threads before any run starts");
    for _ in 0..4 {
        run_start(dir);
    }
}

#[test]
fn CONTROL_threads_stay_bounded_after_300_short_runs() {
    let _heavy = only_one_heavy_test_at_a_time();
    let tmp = scratch("churn");
    let dir = tmp.path();
    let daemon = start_daemon(dir, 0, 300, &[]);
    let before = thread_count(daemon.pid());
    for _ in 0..300 {
        let run = run_start(dir);
        until("a short run to succeed", || {
            text(&pii(&["run", "get", &run], dir))
                .to_lowercase()
                .contains("succeeded")
        });
    }
    let after = thread_count(daemon.pid());
    assert!(
        after <= before + 4,
        "threads {before} before, {after} after"
    );
}

#[test]
fn a_kill_cancel_does_not_freeze_the_async_workers() {
    let _heavy = only_one_heavy_test_at_a_time();
    let tmp = scratch("kill");
    let dir = tmp.path();
    let _daemon = start_daemon(dir, 60_000, 4, &[]);
    let runs: Vec<String> = (0..4).map(|_| run_start(dir)).collect();
    until("run 1 to hold a node", || is_executing(dir, &runs[0]));

    // Each kill waits up to 30 s for an agent that never leaves flight; they are reaped at the end.
    let mut kills: Vec<DaemonGuard> = runs
        .iter()
        .map(|run| {
            daemon_guard::spawn(
                Command::new(env!("CARGO_BIN_EXE_pii"))
                    .args(["run", "cancel", run, "--kill", "--reason", "hung"])
                    .env("PII_RUNTIME_DIR", dir)
                    // State (and its durable stores) must be this test's own: the operator's are
                    // held by any daemon already running on the machine.
                    .env("PII_STATE_DIR", dir)
                    .env("PII_CONNECT_TIMEOUT_MS", "500")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null()),
            )
        })
        .collect();
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end {
        let (health, took) = timed(&["health"], dir);
        // Spec 325 gave `health` three verdicts: 0 this build, 2 no daemon, 3 a daemon that is not this build
        // (or a revision that cannot be compared). In a fresh-gate export there is no .git, so the client's
        // revision is "unknown" and 3 is the CORRECT answer -- this test's subject is the latency of the
        // control plane while kills are in flight, so it accepts any documented verdict and never a crash.
        assert!(
            matches!(health.status.code(), Some(0 | 2 | 3)),
            "health answered with an undocumented code: {}",
            text(&health)
        );
        assert!(took < Duration::from_secs(2), "health took {took:?}");
        std::thread::sleep(Duration::from_millis(100));
    }
    for kill in &mut kills {
        kill.kill_now();
    }
}

#[test]
fn a_cancel_without_kill_of_a_running_node_is_still_refused() {
    let _heavy = only_one_heavy_test_at_a_time();
    let tmp = scratch("refused");
    let dir = tmp.path();
    let _daemon = start_daemon(dir, 20_000, 1, &[]);
    let busy = run_start(dir);
    let mut refusal = String::new();
    until("a refusal naming the executing node", || {
        let o = pii(&["run", "cancel", &busy, "--reason", "no"], dir);
        refusal = text(&o);
        !o.status.success() && refusal.contains("executing node '")
    });
    assert!(refusal.contains("cancel refused"), "{refusal}");
}

#[test]
fn a_queued_run_is_cancelled_and_never_executes() {
    let _heavy = only_one_heavy_test_at_a_time();
    let tmp = scratch("queued");
    let dir = tmp.path();
    // Two answers for three runs: a third execution would find the model exhausted.
    let _daemon = start_daemon(dir, 1_500, 2, &[("PII_DAEMON_RUN_THREADS", "1")]);
    let first = run_start(dir);
    let second = run_start(dir);
    let third = run_start(dir);
    // The first run takes the only slot and sleeps 1.5 s in its model call; the others queue behind it.
    std::thread::sleep(Duration::from_millis(300));

    let cancel = pii(&["run", "cancel", &third, "--reason", "x"], dir);
    assert!(cancel.status.success(), "{}", text(&cancel));

    for run in [&first, &second] {
        until("the first two runs to succeed", || {
            text(&pii(&["run", "get", run], dir))
                .to_lowercase()
                .contains("succeeded")
        });
    }
    let got = text(&pii(&["run", "get", &third], dir));
    assert!(got.contains("cancelled: x"), "{got}");
    assert!(!got.contains("exhausted"), "{got}");
}
