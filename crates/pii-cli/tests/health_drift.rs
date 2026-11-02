//! Spec 325: `pii health` separates "a daemon answers" from "the daemon is this build". Exit 0 means the daemon
//! answers from this client's revision; 2 means no daemon; 3 means a daemon that is not this build (or a
//! comparison that cannot be made). The stub daemon serves the payload of a live daemon with one field changed.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and binds test sockets.
#![allow(clippy::disallowed_methods)]

#[path = "daemon_guard/mod.rs"]
mod daemon_guard;

use daemon_guard::DaemonGuard;
use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use pii_proto::PROTOCOL_VERSION;
use pii_server::daemon::identity::REVISION;

fn scratch(prefix: &str) -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(root)
        .unwrap()
}

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

fn until(what: &str, mut f: impl FnMut() -> bool) {
    // 90 s, not 20: with the daemon slot in force these tests start daemons one at a time, so each start
    // pays for a cold store open instead of sharing a warm machine. Measured 2026-11-02: 20 s ran out with
    // 3 of 5 failing on a start that was progressing, not stuck.
    let end = Instant::now() + Duration::from_secs(90);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {what}");
}

/// A daemon on `dir`, killed when the test ends, panic or not.
fn start_daemon(dir: &Path) -> DaemonGuard {
    let model = dir.join("model.json");
    std::fs::write(
        &model,
        r#"{"turns": [{"text": "ok", "tool_calls": [], "tokens_in": 1, "tokens_out": 1}]}"#,
    )
    .unwrap();
    let slot = daemon_guard::slot();
    let child = Command::new(env!("CARGO_BIN_EXE_pii"))
        .arg("daemon")
        .args(["--sandbox", "off", "--workspace"])
        .arg(dir)
        .arg("--log-file")
        .arg(dir.join("daemon.log"))
        .env("PII_RUNTIME_DIR", dir)
        // State (and its durable stores) must be this test's own: the operator's are
        // held by any daemon already running on the machine.
        .env("PII_STATE_DIR", dir)
        // The daemon must not read the operator's ~/.pii/pii.toml (its [[cron]] entries arm at boot).
        .env("PII_CONFIG", dir.join("pii.toml"))
        .env("PII_MODEL", format!("scripted:{}", model.display()))
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let daemon = DaemonGuard::new(child, slot);
    until("the daemon to answer", || {
        pii(&["sys", "status"], dir).status.success()
    });
    daemon
}

/// Whether this client can prove anything about its own build. A fresh-gate export is `git archive`, so
/// `identity::REVISION` is `unknown` there and every comparison degrades to `UNVERIFIED` by design (spec 325
/// R3/R6). Tests that expect `DRIFT:` or agreement read this instead of assuming a tree with `.git`.
fn client_can_prove_its_build() -> bool {
    REVISION != "unknown"
}

/// The payload a live daemon of this build serves, taken from `pii health --json` against a daemon the test
/// started. The daemon is stopped before this returns.
fn live_payload() -> serde_json::Value {
    let dir = scratch("health-live-");
    let _daemon = start_daemon(dir.path());
    let out = pii(&["health", "--json"], dir.path());
    // Spec 325 R6: in a fresh-gate export there is no .git, so BOTH revisions are "unknown" and `3` is the
    // correct verdict for a daemon of this very build. The fixture's premise is "a daemon answered", not
    // "the answer was 0" -- 2 would mean no daemon, which would make every assertion below meaningless.
    assert!(
        matches!(out.status.code(), Some(0 | 3)),
        "health answered with an undocumented code: {out:?}"
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

/// Serves `payload` to every `sys.*` call on the socket in `dir`, answering `sys.hello` the way a daemon does.
/// The thread ends when the client hangs up; the test does not wait for it.
fn serve_stub(dir: &Path, payload: serde_json::Value) {
    let listener = UnixListener::bind(dir.join("control.sock")).unwrap();
    std::thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let req: serde_json::Value = serde_json::from_str(&line).unwrap();
            let result = if req["method"] == "sys.hello" {
                serde_json::json!({
                    "protocol": PROTOCOL_VERSION,
                    "server": {"name": "pii", "version": "0.1.0", "build": "stub"},
                    "encoding": "json",
                    "capabilities": [],
                    "limits": {"max_frame_bytes": 1_048_576, "max_subscriptions": 8}
                })
            } else {
                payload.clone()
            };
            let resp = serde_json::json!({"jsonrpc": "2.0", "id": req["id"], "result": result});
            writeln!(writer, "{resp}").unwrap();
        }
    });
}

/// `live` with `/build/revision` replaced by `rev`: exactly one field differs from what a daemon serves.
fn with_revision(mut live: serde_json::Value, rev: &str) -> serde_json::Value {
    live["build"]["revision"] = serde_json::Value::from(rev);
    live
}

/// A runtime dir with a stub daemon serving `payload`; the dir lives as long as the returned guard.
fn stub_dir(payload: serde_json::Value) -> (tempfile::TempDir, PathBuf) {
    let dir = scratch("health-stub-");
    serve_stub(dir.path(), payload);
    let path = dir.path().to_path_buf();
    (dir, path)
}

#[test]
fn health_agrees_with_a_daemon_of_this_build() {
    let dir = scratch("health-agree-");
    let _daemon = start_daemon(dir.path());
    let out = pii(&["health"], dir.path());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Spec 325 R6: agreement is only provable when this client's own revision is known. In a fresh-gate export
    // (`git archive`, no .git) it is "unknown", and then `3` is the correct verdict EVEN for a daemon of this
    // very build -- R3's whole point is that unknown is not agreement. Assert the branch, not a constant.
    if client_can_prove_its_build() {
        assert_eq!(out.status.code(), Some(0), "{stderr}");
    } else {
        assert_eq!(out.status.code(), Some(3), "{stderr}");
        assert!(
            stderr.contains("UNVERIFIED:"),
            "an unprovable agreement must say so:\n{stderr}"
        );
    }
    assert!(
        stdout.contains(&format!("client: {REVISION}")),
        "missing client line in:\n{stdout}"
    );
    assert!(!stderr.contains("DRIFT"), "{stderr}");
}

#[test]
fn health_names_a_drift() {
    let (_dir, rt) = stub_dir(with_revision(live_payload(), "0123456789ab"));
    let out = pii(&["health"], &rt);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(3), "{stderr}");
    // The daemon's revision is known here, but in an export this client's is not: R3 then names the CLIENT as
    // the side that cannot be compared, and calling that "drift" would be a claim the client cannot make.
    #[allow(clippy::if_not_else)]
    // the positive branch is a 20-line DRIFT assertion block; the negative one is
    // four lines naming an unprovable client, and putting the long block first reads worse than this
    if !client_can_prove_its_build() {
        assert!(
            stderr.contains("UNVERIFIED: client"),
            "an unprovable client must be the side named: {stderr}"
        );
        assert!(
            !stderr.contains("DRIFT:"),
            "unverified is not drift: {stderr}"
        );
        assert!(stdout.contains("0123456789ab"), "{stdout}");
        return;
    }
    let drift = stderr
        .lines()
        .filter(|l| l.starts_with("DRIFT:"))
        .collect::<Vec<_>>();
    assert_eq!(drift.len(), 1, "{stderr}");
    assert!(drift[0].contains("0123456789ab"), "{stderr}");
    assert!(drift[0].contains(REVISION), "{stderr}");
    // stdout keeps the payload the daemon answered with.
    assert!(stdout.contains("0123456789ab"), "{stdout}");
}

#[test]
fn health_refuses_unknown() {
    let (_dir, rt) = stub_dir(with_revision(live_payload(), "unknown"));
    let out = pii(&["health"], &rt);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(3), "{stderr}");
    assert!(stderr.contains("UNVERIFIED:"), "{stderr}");
    assert!(
        stderr.contains("daemon build revision is unknown"),
        "{stderr}"
    );
}

#[test]
fn json_is_the_daemons_payload() {
    let served = with_revision(live_payload(), "0123456789ab");
    let (_dir, rt) = stub_dir(served.clone());
    let out = pii(&["health", "--json"], &rt);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(parsed, served);
}

#[test]
fn no_daemon_is_still_two() {
    let dir = scratch("health-none-");
    let out = pii(&["health"], dir.path());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("error: no daemon answered"), "{stderr}");
}
