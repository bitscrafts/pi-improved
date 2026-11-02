//! Spec 243: `pii run cancel <id> --kill` stops a run whose node is executing, and the run closes as
//! failed, naming the abandoned node. Without `--kill` the refusal stands (spec 189).
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
    let child = DaemonGuard::new(
        Command::new(env!("CARGO_BIN_EXE_pii"))
            .arg("daemon")
            .args(["--sandbox", "off", "--workspace"])
            .arg(dir)
            .arg("--log-file")
            .arg(dir.join("daemon.log"))
            .env("PII_RUNTIME_DIR", dir)
            // The daemon must not read the operator's ~/.pii/pii.toml: its [[cron]] entries arm at
            // boot and park suspended runs in this scratch store (measured 2026-10-20: the failing
            // "has 1 runs" count was 2-3, wall-clock dependent). A missing file is an empty config.
            .env("PII_CONFIG", dir.join("pii.toml"))
            .env("PII_MODEL", format!("scripted:{}", slow.display()))
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
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
fn cancel_kill_stops_an_executing_run_and_the_refusal_stays_without_the_flag() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&root).unwrap();
    let tmp = tempfile::Builder::new()
        .prefix("run-cancel-kill-")
        .tempdir_in(root)
        .unwrap();
    let dir = tmp.path();
    let mut daemon = start_daemon(dir);

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

    let usage = pii(&["run", "cancel", "--kill"], dir);
    assert_eq!(usage.status.code(), Some(2), "--kill needs a run id");

    let killed = pii(
        &["run", "cancel", &busy, "--kill", "--reason", "hung agent"],
        dir,
    );
    assert!(killed.status.success(), "{}", text(&killed));
    let got = text(&pii(&["run", "get", &busy], dir)).to_lowercase();
    assert!(got.contains("failed"), "{got}");

    let log = std::fs::read_to_string(dir.join("daemon.log")).unwrap();
    daemon.kill_now();
    let killing = log
        .lines()
        .find(|l| l.contains("run.cancel_killing") && l.contains(&busy))
        .unwrap_or_else(|| panic!("no run.cancel_killing in log:\n{log}"));
    let node = refusal
        .split("executing node '")
        .nth(1)
        .unwrap()
        .split('\'')
        .next()
        .unwrap();
    assert!(killing.contains(node), "the kill names the node: {killing}");
}
