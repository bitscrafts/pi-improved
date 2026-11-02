//! Spec 233 R2: `pii skill add`, `list` and `show` go through the running daemon while it holds the store, say so on
//! stderr, and fall back to the store only when no daemon answers. The assertions read the slug, the sha256 and the
//! body back, not just the exit code.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout and stderr.
#![allow(clippy::disallowed_methods)]

#[path = "daemon_guard/mod.rs"]
mod daemon_guard;

use daemon_guard::DaemonGuard;
use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use pii_store::testing::tmp_scratch;
use sha2::{Digest as _, Sha256};

const SKILL: &str = "PII-SKILL\nlint-233\nWHEN TO USE: a file needs linting\nCONTRACT: never edit the file\nVERBS: lint\n";

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
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {what}");
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// A daemon on `dir`, killed when dropped.
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
        // The daemon must not read the operator's ~/.pii/pii.toml (see trace_reads_through_the_daemon).
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

fn text_of(out: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn skill_add_and_list_read_through_the_daemon_then_the_store() {
    let tmp = tmp_scratch(env!("CARGO_MANIFEST_DIR"), "skill-cli-daemon-").unwrap();
    let dir = tmp.path();
    let file = dir.join("lint.skill");
    std::fs::write(&file, SKILL).unwrap();
    let file_arg = file.display().to_string();
    let hex = sha256_hex(SKILL.as_bytes());

    let daemon = start_daemon(dir);

    // add: the daemon writes its own store, and the line names the id, the slug and the pin hash.
    let out = pii(&["skill", "add", "--file", &file_arg], dir);
    let (stdout, stderr) = text_of(&out);
    assert!(out.status.success(), "add exits zero: {stderr}");
    assert!(
        stderr.contains("via daemon"),
        "add names its source: {stderr}"
    );
    let added = stdout
        .lines()
        .find(|l| l.starts_with("added "))
        .unwrap_or_default();
    assert!(
        added.contains(" lint-233 ") && added.ends_with(&format!("sha256={hex}")),
        "add prints the slug and the pin hash: {stdout}"
    );

    // add again under the same slug: refused by the daemon, and the CLI does not open the locked store instead.
    let out = pii(&["skill", "add", "--file", &file_arg], dir);
    let (_, stderr) = text_of(&out);
    assert!(!out.status.success(), "a second add of the slug is refused");
    assert!(
        stderr.contains("lint-233"),
        "the refusal names the slug: {stderr}"
    );
    assert!(
        !stderr.contains("via store"),
        "no fallback to the store: {stderr}"
    );

    // list while the daemon holds the store: the slug comes back and the source is the daemon.
    let out = pii(&["skill", "list"], dir);
    let (stdout, stderr) = text_of(&out);
    assert!(out.status.success(), "list exits zero: {stderr}");
    assert!(stderr.contains("via daemon"), "{stderr}");
    assert!(
        stdout.contains("lint-233"),
        "the daemon lists the slug: {stdout}"
    );

    drop(daemon);

    // With the daemon stopped, the same list over the same dir reads the store and says so.
    let dir_arg = dir.display().to_string();
    let out = pii(&["skill", "list", "--dir", &dir_arg], dir);
    let (stdout, stderr) = text_of(&out);
    assert!(out.status.success(), "list exits zero: {stderr}");
    assert!(stderr.contains("via store"), "{stderr}");
    assert!(
        stdout.contains("lint-233"),
        "the store lists the slug: {stdout}"
    );

    // show reads the stored body verbatim, and its bytes hash to the pin the add printed.
    let out = pii(&["skill", "show", "lint-233", "--dir", &dir_arg], dir);
    let (stdout, stderr) = text_of(&out);
    assert!(out.status.success(), "show exits zero: {stderr}");
    let body = stdout
        .split_once("---\n")
        .map(|(_, b)| b)
        .unwrap_or_default();
    assert_eq!(body, SKILL, "the body is the file's text: {stdout}");
    assert_eq!(
        sha256_hex(body.as_bytes()),
        hex,
        "the body hashes to the pin"
    );
}
