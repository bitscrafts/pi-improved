//! Spec 222 R1/R3: a graph that `pii graph list` shows is the graph `pii run start` finds. The daemon
//! runs with its runtime dir empty and its store beside that dir, so the only copy of the graph is
//! the installer's state dir under `$HOME`. Both readers must search that same dir.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

const GRAPH: &str = "name = \"coder-demo\"\nentry = \"step\"\ncaps = []\n\n[[node]]\nid = \"step\"\nkind = \"transform\"\nvalue = \"ok\"\n";

/// A scratch root with `$HOME/.local/state/pii/graphs/coder-demo.toml` and an empty runtime dir.
fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "run-start-resolves-").unwrap();
    let home = root.path().join("home");
    let installed = home.join(".local/state/pii/graphs");
    std::fs::create_dir_all(&installed).unwrap();
    std::fs::write(installed.join("coder-demo.toml"), GRAPH).unwrap();
    let runtime = root.path().join("rt");
    std::fs::create_dir_all(&runtime).unwrap();
    (root, home, runtime)
}

fn pii_as(home: &Path, runtime: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pii"))
        .env("HOME", home)
        .env("PII_RUNTIME_DIR", runtime)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn list_shows_the_graph_and_run_start_finds_it() {
    let (root, home, runtime) = fixture();
    let graphs = pii_as(&home, &runtime, &["graph", "list"]);
    let stdout = String::from_utf8_lossy(&graphs.stdout);
    assert_eq!(graphs.status.code(), Some(0), "{graphs:?}");
    assert!(
        stdout.lines().any(|r| r.starts_with("coder-demo ")),
        "{stdout}"
    );

    let hello = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase2/hello.json");
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_pii"))
        .arg("daemon")
        .args(["--sandbox", "off", "--workspace"])
        .arg(root.path())
        .env("HOME", &home)
        .env("PII_RUNTIME_DIR", &runtime)
        .env("PII_MODEL", format!("scripted:{}", hello.display()))
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();

    // The daemon answers once its socket is up. Until then `run start` cannot connect at all.
    let end = Instant::now() + Duration::from_secs(15);
    let mut last = String::new();
    let mut started = false;
    while Instant::now() < end {
        let out = pii_as(&home, &runtime, &["run", "start", "coder-demo"]);
        last = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if last.contains("not found") {
            break;
        }
        if out.status.success() {
            started = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    daemon.kill().unwrap();
    daemon.wait().unwrap();
    assert!(started, "run start did not return a run id: {last}");
}
