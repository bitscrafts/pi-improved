//! Spec 212: a fragment whose bytes no longer match its pin is refused, and a
//! fragment that attaches a fragment is refused (depth 1 only).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;

use pii_graph::fragment;
use pii_graph::node::NodeRegistry;

const GUARD: &str = r#"
name = "podman-guard"

[fragment]
name = "podman-guard"
inputs = ["$.input.runtime"]
outputs = ["ok", "refuse"]
may_block = true

[[node]]
id = "check"
kind = "route"
on = "$.input.runtime"
rules = [{ eq = "podman", port = "refuse" }, { port = "ok" }]

[[node]]
id = "refuse"
kind = "refuse"
reason = "podman is refused by the guard"

[[edge]]
from = "check"
port = "refuse"
to = "refuse"
kind = "data"
"#;

const HOST: &str = r#"
name = "host"
entry = "build"

[[attach]]
id = "guard"
fragment = "podman-guard"
at = "run.start"
pin = "PIN"

[[node]]
id = "build"
kind = "transform"
value = "built"
"#;

/// Writes the fragment under a fresh `fragments/` dir and weaves the host.
fn weave_with(host: &str, fragment_src: &str) -> Result<(), String> {
    let root = tempfile::tempdir().expect("tempdir");
    let dir: PathBuf = root.path().join("fragments");
    std::fs::create_dir_all(&dir).expect("mkdir fragments");
    std::fs::write(dir.join("podman-guard.toml"), fragment_src).expect("write fragment");
    fragment::weave(host, &dir, &NodeRegistry::builtin())
        .map(drop)
        .map_err(|e| e.to_string())
}

/// Spec 212 contract: `fragment_pin_checked` — drift is refused, and the
/// refusal names the file and both hashes.
#[test]
fn fragment_pin_checked() {
    let real = fragment::sha256_hex(GUARD.as_bytes());
    let zeros = "0".repeat(64);
    assert_ne!(real, zeros);

    // The pin matches the bytes: the weave goes through.
    let good = HOST.replace("PIN", &real);
    weave_with(&good, GUARD).expect("matching pin weaves");

    // Edit the fragment by one byte: the old pin is now drift.
    let edited = GUARD.replace("podman is refused", "podman is refused!");
    let msg = weave_with(&good, &edited).expect_err("drifted pin must be refused");
    assert!(msg.contains("fragment.pin.drift"), "got: {msg}");
    assert!(msg.contains("podman-guard.toml"), "got: {msg}");
    assert!(msg.contains(&real), "expected hash missing: {msg}");
    assert!(
        msg.contains(&fragment::sha256_hex(edited.as_bytes())),
        "actual hash missing: {msg}"
    );

    // A wrong pin is refused the same way.
    let wrong = HOST.replace("PIN", &zeros);
    let msg = weave_with(&wrong, GUARD).expect_err("wrong pin must be refused");
    assert!(msg.contains("fragment.pin.drift"), "got: {msg}");
}

/// A missing pin is refused: nothing is inlined without a pin check.
#[test]
fn fragment_without_pin_is_refused() {
    let unpinned = HOST.replace("pin = \"PIN\"\n", "");
    let msg = weave_with(&unpinned, GUARD).expect_err("unpinned attach must be refused");
    assert!(msg.contains("fragment.pin.missing"), "got: {msg}");
}

/// Spec 212 contract: `no_nesting` — a fragment that attaches a fragment is
/// refused, whatever the nested attach points at.
#[test]
fn no_nesting() {
    let nested = GUARD.replacen(
        "[[node]]",
        "[[attach]]\nid = \"inner\"\nfragment = \"podman-guard\"\nat = \"run.start\"\npin = \"0\"\n\n[[node]]",
        1,
    );
    let host = HOST.replace("PIN", &fragment::sha256_hex(nested.as_bytes()));
    let msg = weave_with(&host, &nested).expect_err("nested attach must be refused");
    assert!(msg.contains("fragment.nesting.forbidden"), "got: {msg}");
}
