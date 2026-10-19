//! Spec 210 R1-R5: `pii skill list` and `pii skill explain` against the real binary, over a store
//! this test writes. Skills are `DocKind::Skill` documents; the pin is `script: <path> sha256=<hex>`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

use std::fmt::Write as _;
use std::path::Path;

use pii_memory::doc::{Doc, DocKind, Layer, encode, layer_byte};
use pii_memory::tokenize;
use pii_store::MemStore;
use pii_store::mem::PutRows;
use sha2::{Digest as _, Sha256};

fn pii_in(cwd: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap()
}

/// Writes one document through the same rows the daemon writes; returns its id.
fn put(store: &MemStore, layer: Layer, kind: DocKind, text: &str) -> String {
    let doc = Doc::new(layer, kind, text, vec![], 1.0, 1_000);
    let mut terms: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    let toks = tokenize(text);
    for t in &toks {
        *terms.entry(t.clone()).or_insert(0) += 1;
    }
    let rows = PutRows {
        id: doc.id.clone(),
        doc: encode(&doc),
        terms: terms.into_iter().collect(),
        vector: None,
        edges: Vec::new(),
        dl: u32::try_from(toks.len()).unwrap_or(u32::MAX),
        layer: layer_byte(layer),
    };
    store.put(&rows).unwrap();
    doc.id
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn skill_body(slug: &str, pin: &str) -> String {
    format!(
        "PII-SKILL\n{slug}\nWHEN TO USE: a container is unhealthy\nCONTRACT: never restart without a probe\nVERBS: probe, inspect\n{pin}\n"
    )
}

/// A repo root holding one pinned script, and the store dir both live under.
fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let root =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cli-skill-list-").unwrap();
    let script = root.path().join("scripts/probe.sh");
    std::fs::create_dir_all(script.parent().unwrap()).unwrap();
    std::fs::write(&script, b"#!/bin/sh\necho probe\n").unwrap();
    let dir = root.path().join("store");
    std::fs::create_dir_all(&dir).unwrap();
    (root, dir)
}

#[test]
fn list_shows_only_skills() {
    let (root, dir) = fixture();
    let store = MemStore::open(&dir.join("memory.redb")).unwrap();
    put(&store, Layer::L1, DocKind::Atom, "the sky is blue");
    put(
        &store,
        Layer::L2,
        DocKind::Scene,
        "a scene of atoms about sky",
    );
    put(&store, Layer::L3, DocKind::Persona, "the persona is calm");
    let skill = put(
        &store,
        Layer::L3,
        DocKind::Skill,
        &skill_body("podman-probe", "x"),
    );
    drop(store);

    let out = pii_in(
        root.path(),
        &["skill", "list", "--dir", dir.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let rows: Vec<&str> = stdout.lines().collect();
    assert_eq!(rows.len(), 1, "only the skill is listed: {stdout}");
    assert!(rows[0].contains(&skill[..12]), "{stdout}");
    assert!(rows[0].contains("podman-probe"), "{stdout}");
    assert!(!stdout.contains("sky"), "{stdout}");
}

#[test]
fn list_reports_pin_status() {
    let (root, dir) = fixture();
    let store = MemStore::open(&dir.join("memory.redb")).unwrap();
    let wrong = "0".repeat(64);
    put(
        &store,
        Layer::L3,
        DocKind::Skill,
        &skill_body(
            "wrong-pin",
            &format!("script: scripts/probe.sh sha256={wrong}"),
        ),
    );
    drop(store);

    let out = pii_in(
        root.path(),
        &["skill", "list", "--dir", dir.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("DRIFT"), "{stdout}");
    assert!(!stdout.contains(" ok"), "{stdout}");
}

#[test]
fn explain_names_the_pin() {
    let (root, dir) = fixture();
    let real = sha256_hex(&std::fs::read(root.path().join("scripts/probe.sh")).unwrap());
    let store = MemStore::open(&dir.join("memory.redb")).unwrap();
    let id = put(
        &store,
        Layer::L3,
        DocKind::Skill,
        &skill_body(
            "podman-probe",
            &format!("script: scripts/probe.sh sha256={real}"),
        ),
    );
    drop(store);

    let out = pii_in(
        root.path(),
        &["skill", "explain", &id, "--dir", dir.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(&format!("PIN ok {real}")), "{stdout}");
    assert!(
        stdout.contains("WHEN TO USE: a container is unhealthy"),
        "{stdout}"
    );
    assert!(
        stdout.contains("CONTRACT: never restart without a probe"),
        "{stdout}"
    );
    assert!(stdout.contains("VERBS: probe, inspect"), "{stdout}");
}

#[test]
fn explain_without_the_script() {
    let (root, dir) = fixture();
    let real = sha256_hex(&std::fs::read(root.path().join("scripts/probe.sh")).unwrap());
    let store = MemStore::open(&dir.join("memory.redb")).unwrap();
    put(
        &store,
        Layer::L3,
        DocKind::Skill,
        &skill_body(
            "gone-script",
            &format!("script: scripts/probe.sh sha256={real}"),
        ),
    );
    drop(store);
    std::fs::remove_file(root.path().join("scripts/probe.sh")).unwrap();

    let out = pii_in(
        root.path(),
        &[
            "skill",
            "explain",
            "gone-script",
            "--dir",
            dir.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("MISSING FILE"), "{stdout}");
    assert!(stdout.contains("WHEN TO USE"), "{stdout}");
}
