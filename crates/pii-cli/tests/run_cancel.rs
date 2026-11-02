//! Spec 189: `pii run cancel <id> --reason <text>` retires a suspended run with the reason on
//! the record, and refuses a run whose node is executing, naming the node.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary.
#![allow(clippy::disallowed_methods)]

#[path = "daemon_guard/mod.rs"]
mod daemon_guard;

use daemon_guard::DaemonGuard;
use std::path::Path;
use std::process::{Command, Output};
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

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {what}");
}

fn start_daemon(dir: &Path) -> DaemonGuard {
    // A turn that outlasts every poll below, so the run stays in flight while it is cancelled.
    let slow = dir.join("slow-model.json");
    std::fs::write(
        &slow,
        r#"{"sleep_ms": 15000, "turns": [{"text": "slow", "tool_calls": [], "tokens_in": 1, "tokens_out": 1}]}"#,
    )
    .unwrap();
    let child = daemon_guard::spawn(
        Command::new(env!("CARGO_BIN_EXE_pii"))
            .arg("daemon")
            .args(["--sandbox", "off", "--workspace"])
            .arg(dir)
            .arg("--log-file")
            .arg(dir.join("daemon.log"))
            .env("PII_RUNTIME_DIR", dir)
            // State (and its durable stores) must be this test's own: the operator's are
            // held by any daemon already running on the machine.
            .env("PII_STATE_DIR", dir)
            // The daemon must not read the operator's ~/.pii/pii.toml: its [[cron]] entries arm at
            // boot and park suspended runs in this scratch store (measured 2026-10-20: the failing
            // "has 1 runs" count was 2-3, wall-clock dependent). A missing file is an empty config.
            .env("PII_CONFIG", dir.join("pii.toml"))
            .env("PII_MODEL", format!("scripted:{}", slow.display()))
            .stderr(std::process::Stdio::null()),
    );
    until("the daemon to answer", || {
        pii(&["sys", "status"], dir).status.success()
    });
    child
}

fn run_id(started: &Output) -> String {
    let v: serde_json::Value = serde_json::from_slice(&started.stdout).unwrap();
    v.get("run_id")
        .and_then(serde_json::Value::as_str)
        .unwrap()
        .to_owned()
}

#[test]
fn cancel_retires_a_suspended_run_and_refuses_an_executing_one() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&root).unwrap();
    let tmp = tempfile::Builder::new()
        .prefix("run-cancel-")
        .tempdir_in(root)
        .unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    std::fs::write(
        dir.join("graphs/park.toml"),
        "name = \"park\"\nentry = \"wait\"\ncaps = []\n\n[[node]]\nid = \"wait\"\nkind = \"timer\"\nin_ms = 3600000\n",
    )
    .unwrap();
    let mut daemon = start_daemon(dir);

    // A suspended run is cancelled, with the reason on the record.
    let parked = run_id(&pii(&["run", "start", "park"], dir));
    until("the run to suspend", || {
        text(&pii(&["run", "get", &parked], dir)).contains("Suspended")
            || text(&pii(&["run", "get", &parked], dir)).contains("suspended")
    });
    let out = pii(
        &["run", "cancel", &parked, "--reason", "duplicate digest"],
        dir,
    );
    assert!(out.status.success(), "{}", text(&out));
    let got = text(&pii(&["run", "get", &parked], dir)).to_lowercase();
    assert!(got.contains("failed"), "{got}");
    let again = pii(&["run", "cancel", &parked, "--reason", "twice"], dir);
    assert!(
        !again.status.success(),
        "a finished run is not cancelled twice"
    );
    let missing = pii(&["run", "cancel", &parked], dir);
    assert_eq!(missing.status.code(), Some(2), "--reason is required");

    // A run whose node is executing is refused, naming the node.
    let busy = run_id(&pii(
        &["run", "start", "default", "--input", r#"{"prompt":"hi"}"#],
        dir,
    ));
    let mut refusal = String::new();
    until("a refusal naming the executing node", || {
        let o = pii(&["run", "cancel", &busy, "--reason", "no"], dir);
        assert!(!o.status.success(), "an executing run was cancelled");
        refusal = text(&o);
        refusal.contains("executing node '") && !refusal.contains("executing node ''")
    });
    assert!(refusal.contains("cancel refused"), "{refusal}");
    let log = std::fs::read_to_string(dir.join("daemon.log")).unwrap();
    daemon.kill_now();

    let line = |name: &str| {
        log.lines()
            .find(|l| l.contains(name))
            .unwrap_or_else(|| panic!("no {name} in log:\n{log}"))
            .to_owned()
    };
    let cancelled = line("run.cancelled");
    assert!(cancelled.contains(&parked) && cancelled.contains("duplicate digest"));
    let refused = line("run.cancel_refused");
    assert!(refused.contains(&busy), "{refused}");
    assert!(
        !log.lines()
            .any(|l| l.contains("run.cancelled") && l.contains(&busy)),
        "the executing run must not be reported cancelled"
    );
}
