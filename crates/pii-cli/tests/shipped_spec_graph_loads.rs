//! Spec 272: the shipped `graphs/spec.toml` loads through the BUILT `pii` binary: it checks clean, its explain prints the
//! call of `docs`, and a copy whose pin is wrong is refused by name. The binary reads the repo's own graphs directory.

#![allow(
    non_snake_case,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::Output;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn graphs_dir() -> PathBuf {
    repo_root().join("graphs")
}

/// Runs the built `pii` with `args` from a scratch directory of its own. The lint stores open in the working directory
/// and under `PII_RUNTIME_DIR`: a missing runtime directory leaves the `judge` kind unregistered, and two runs in one
/// directory contend for the same store lock. So each run gets a fresh directory for both.
fn pii(args: &[&str]) -> Output {
    let scratch =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cli-docs-run-").unwrap();
    std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .current_dir(scratch.path())
        .env("PII_RUNTIME_DIR", scratch.path())
        .args(args)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn check_loads_the_shipped_graph() {
    let file = graphs_dir().join("spec.toml");
    let out = pii(&["graph", "check", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
}

/// `pii graph explain` takes an installed graph's name, not a path (spec 269), so the tracked graph is named through
/// `--dir`, with no daemon.
#[test]
fn explain_prints_the_call_line() {
    let root = repo_root();
    let out = pii(&[
        "graph",
        "explain",
        "spec",
        "--dir",
        graphs_dir().to_str().unwrap(),
        "--repo",
        root.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    let sha =
        pii_graph::fragment::sha256_hex(&std::fs::read(graphs_dir().join("docs.toml")).unwrap());
    let want = format!(
        "call docs -> graph docs sha256 {} caps [proc,spawn] needs [agent,model,provider,spec,workdir,brief] exit docs",
        &sha[..12]
    );
    assert!(stdout.lines().any(|l| l == want), "{stdout}");
}

#[test]
fn CONTROL_a_wrong_pin_fails_check() {
    let copy =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cli-docs-pin-").unwrap();
    let spec = std::fs::read_to_string(graphs_dir().join("spec.toml")).unwrap();
    let sha =
        pii_graph::fragment::sha256_hex(&std::fs::read(graphs_dir().join("docs.toml")).unwrap());
    std::fs::write(
        copy.path().join("spec.toml"),
        spec.replace(&sha, &"0".repeat(64)),
    )
    .unwrap();
    // `spec` is a router (spec 385): it calls the methodology graphs too, and a call to a graph that is not in
    // the dir is refused as `not_found` before the docs pin is ever read. Copy what it routes to, so the pin
    // is the only thing wrong.
    for g in ["docs", "sdd", "method-judge"] {
        let f = format!("{g}.toml");
        std::fs::copy(graphs_dir().join(&f), copy.path().join(&f)).unwrap();
    }
    let out = pii(&[
        "graph",
        "check",
        copy.path().join("spec.toml").to_str().unwrap(),
    ]);
    assert_ne!(out.status.code(), Some(0), "a wrong pin is refused");
    assert!(
        text(&out.stderr).contains("fragment.pin.mismatch"),
        "{}",
        text(&out.stderr)
    );
}
