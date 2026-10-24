//! Spec 260: `pii graph check|show|run <file>` expand a `call` node the way the daemon does. A host that calls a
//! graph in its own directory is checked, drawn and run with the sub-graph inlined; a wrong pin, a missing graph
//! and a self-call are refused by the binary with the daemon's rule names, and a graph with no call is unchanged.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout and stderr.
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The sub-graph `note`: one transform node `fmt` that reads the title the host maps into it.
const NOTE: &str = r#"name = "note"
exit = "fmt"

[[node]]
id = "fmt"
kind = "transform"
expr = "$.input.title"
"#;

/// A host `start -> docs(call note, maps title from $.input.spec) -> done`, with `PIN` the pin of `note`.
fn host(pin: &str, graph: &str) -> String {
    format!(
        r#"name = "host"
entry = "start"

[[node]]
id = "start"
kind = "transform"
value = "go"

[[node]]
id = "docs"
kind = "call"
graph = "{graph}"
sha256 = "{pin}"
caps = []

[node.in]
title = "$.input.spec"

[[node]]
id = "done"
kind = "transform"
expr = "$.payload"

[[edge]]
from = "start"
port = "ok"
to = "docs"
kind = "data"

[[edge]]
from = "docs"
port = "ok"
to = "done"
kind = "data"
"#
    )
}

/// A host with no call: `start -> done`.
const PLAIN: &str = r#"name = "plain"
entry = "start"

[[node]]
id = "start"
kind = "transform"
value = "go"

[[node]]
id = "done"
kind = "transform"
expr = "$.payload"

[[edge]]
from = "start"
port = "ok"
to = "done"
kind = "data"
"#;

fn scratch() -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "graph-calls-").unwrap()
}

fn write(path: &Path, src: &str) -> PathBuf {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, src).unwrap();
    path.to_path_buf()
}

fn pii(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(args)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn digest(src: &str) -> String {
    pii_graph::fragment::sha256_hex(src.as_bytes())
}

#[test]
fn check_passes_a_host_that_calls_a_graph() {
    let dir = scratch();
    write(&dir.path().join("note.toml"), NOTE);
    let file = write(&dir.path().join("host.toml"), &host(&digest(NOTE), "note"));
    let out = pii(&["graph", "check", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
}

#[test]
fn check_refuses_a_wrong_pin_naming_both_digests() {
    let dir = scratch();
    write(&dir.path().join("note.toml"), NOTE);
    let file = write(
        &dir.path().join("host.toml"),
        &host(&"0".repeat(64), "note"),
    );
    let out = pii(&["graph", "check", file.to_str().unwrap()]);
    assert_ne!(out.status.code(), Some(0));
    let err = text(&out.stderr);
    assert!(err.contains("pin"), "{err}");
    assert!(err.contains(&digest(NOTE)[..12]), "{err}");
}

#[test]
fn check_refuses_a_missing_graph() {
    let dir = scratch();
    let file = write(&dir.path().join("host.toml"), &host(&digest(NOTE), "nope"));
    let out = pii(&["graph", "check", file.to_str().unwrap()]);
    assert_ne!(out.status.code(), Some(0));
    assert!(
        text(&out.stderr).contains("not_found"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn check_refuses_a_graph_that_calls_itself() {
    let dir = scratch();
    // The host is named `a` and calls `a`: the cycle is refused before the pin is read.
    let file = write(
        &dir.path().join("a.toml"),
        &host(&digest(NOTE), "a").replace("name = \"host\"", "name = \"a\""),
    );
    let out = pii(&["graph", "check", file.to_str().unwrap()]);
    assert_ne!(out.status.code(), Some(0));
    assert!(
        text(&out.stderr).contains("a -> a"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn show_draws_the_expanded_ids() {
    let dir = scratch();
    write(&dir.path().join("note.toml"), NOTE);
    let file = write(&dir.path().join("host.toml"), &host(&digest(NOTE), "note"));
    let out = pii(&["graph", "show", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let shown = text(&out.stdout);
    assert!(shown.contains("docs__in"), "{shown}");
    assert!(shown.contains("docs__fmt"), "{shown}");
}

#[test]
fn run_executes_the_expanded_graph() {
    let dir = scratch();
    write(&dir.path().join("note.toml"), NOTE);
    let file = write(&dir.path().join("host.toml"), &host(&digest(NOTE), "note"));
    let db = dir.path().join("store.redb");
    let out = pii(&[
        "graph",
        "run",
        file.to_str().unwrap(),
        "--db",
        db.to_str().unwrap(),
        "--input",
        r#"{"spec":"hello"}"#,
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("hello"), "{}", text(&out.stdout));
}

#[test]
fn run_with_a_wrong_pin_runs_nothing() {
    let dir = scratch();
    write(&dir.path().join("note.toml"), NOTE);
    let file = write(
        &dir.path().join("host.toml"),
        &host(&"0".repeat(64), "note"),
    );
    let db = dir.path().join("store.redb");
    let out = pii(&[
        "graph",
        "run",
        file.to_str().unwrap(),
        "--db",
        db.to_str().unwrap(),
        "--input",
        r#"{"spec":"hello"}"#,
    ]);
    assert_ne!(out.status.code(), Some(0));
    assert!(!db.exists(), "a refused run must not create the store");
}

#[test]
fn a_graph_without_a_call_is_unchanged() {
    let dir = scratch();
    let file = write(&dir.path().join("plain.toml"), PLAIN);
    let path = file.to_str().unwrap();
    let db = dir.path().join("store.redb");

    let check = pii(&["graph", "check", path]);
    assert_eq!(check.status.code(), Some(0), "{}", text(&check.stderr));

    let show = pii(&["graph", "show", path]);
    assert_eq!(show.status.code(), Some(0), "{}", text(&show.stderr));
    assert!(
        text(&show.stdout).contains("start"),
        "{}",
        text(&show.stdout)
    );

    let run = pii(&["graph", "run", path, "--db", db.to_str().unwrap()]);
    assert_eq!(run.status.code(), Some(0), "{}", text(&run.stderr));
}

#[test]
fn a_call_to_a_graph_in_another_directory_is_not_found() {
    let dir = scratch();
    // `note` lives in a sibling directory; the lookup is the host's own directory, not a global search.
    write(&dir.path().join("other/note.toml"), NOTE);
    let file = write(
        &dir.path().join("host/host.toml"),
        &host(&digest(NOTE), "note"),
    );
    let out = pii(&["graph", "check", file.to_str().unwrap()]);
    assert_ne!(out.status.code(), Some(0));
}
