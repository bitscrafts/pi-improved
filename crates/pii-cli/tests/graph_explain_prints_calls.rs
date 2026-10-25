//! Spec 269: `pii graph explain <name>` prints one `call` line per call the loaded graph expanded, in the shape the
//! spec fixes; a nested call names its parent; a graph with no call prints no `call` line. The binary runs against an
//! installed directory of fixtures under a scratch root.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::Output;

/// The sub-graph `note`: one transform `fmt` that reads the title the host maps into it.
const NOTE: &str = r#"name = "note"
exit = "fmt"

[[node]]
id = "fmt"
kind = "transform"
expr = "$.input.title"
"#;

/// A host `start -> docs(call note, maps title from $.input.spec) -> done`, pinned to `NOTE`.
fn host(graph: &str, sha: &str) -> String {
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
sha256 = "{sha}"
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

fn digest(src: &str) -> String {
    pii_graph::fragment::sha256_hex(src.as_bytes())
}

/// An installed directory holding `files` (name stem, source), and an empty repo directory, under a scratch root.
fn fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cli-graph-calls-").unwrap();
    let installed = root.path().join("state").join("graphs");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&installed).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    for (name, src) in files {
        std::fs::write(installed.join(format!("{name}.toml")), src).unwrap();
    }
    (root, installed, repo)
}

fn explain(cwd: &Path, name: &str, installed: &Path, repo: &Path) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .current_dir(cwd)
        .env("PII_RUNTIME_DIR", cwd.join("no-daemon-here"))
        .args([
            "graph",
            "explain",
            name,
            "--dir",
            installed.to_str().unwrap(),
            "--repo",
            repo.to_str().unwrap(),
        ])
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn explain_prints_the_call_line() {
    let (root, installed, repo) =
        fixture(&[("note", NOTE), ("host", &host("note", &digest(NOTE)))]);
    let out = explain(root.path(), "host", &installed, &repo);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    let want = format!(
        "call docs -> graph note sha256 {} caps [] needs [none] exit fmt",
        &digest(NOTE)[..12]
    );
    assert!(stdout.lines().any(|l| l == want), "{stdout}");
}

#[test]
fn explain_prints_the_parent_of_a_nested_call() {
    let inner = "name = \"inner\"\nexit = \"a\"\n\n[[node]]\nid = \"a\"\nkind = \"transform\"\nvalue = \"deep\"\n";
    let outer = format!(
        "name = \"outer\"\nexit = \"tail\"\n\n[[node]]\nid = \"n\"\nkind = \"call\"\ngraph = \"inner\"\nsha256 = \"{}\"\ncaps = []\n\n[[node]]\nid = \"tail\"\nkind = \"transform\"\nvalue = \"t\"\n\n[[edge]]\nfrom = \"n\"\nport = \"ok\"\nto = \"tail\"\nkind = \"data\"\n",
        digest(inner)
    );
    let top = host("outer", &digest(&outer));
    let (root, installed, repo) = fixture(&[("inner", inner), ("outer", &outer), ("host", &top)]);
    let out = explain(root.path(), "host", &installed, &repo);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    let want = format!(
        "call n -> graph inner sha256 {} caps [] needs [none] exit a (in docs)",
        &digest(inner)[..12]
    );
    assert!(stdout.lines().any(|l| l == want), "{stdout}");
}

#[test]
fn control_explain_prints_no_call_line_without_a_call() {
    let (root, installed, repo) = fixture(&[("plain", PLAIN)]);
    let out = explain(root.path(), "plain", &installed, &repo);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(!stdout.lines().any(|l| l.starts_with("call ")), "{stdout}");
}
