//! Spec 210 R4: `pii skill edit` never silently de-pins a skill. A body that drops the pin line
//! is refused (non-zero, body unchanged); a body that keeps the pin is written, with the id kept.
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

use pii_memory::doc::{Doc, DocKind, Layer, decode_text, encode, layer_byte};
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

fn put(store: &MemStore, text: &str) -> String {
    let doc = Doc::new(Layer::L3, DocKind::Skill, text, vec![], 1.0, 1_000);
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
        layer: layer_byte(Layer::L3),
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

#[test]
fn edit_keeps_the_pin() {
    let root =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cli-skill-edit-").unwrap();
    let script = root.path().join("scripts/probe.sh");
    std::fs::create_dir_all(script.parent().unwrap()).unwrap();
    std::fs::write(&script, b"#!/bin/sh\necho probe\n").unwrap();
    let real = sha256_hex(b"#!/bin/sh\necho probe\n");
    let dir = root.path().join("store");
    std::fs::create_dir_all(&dir).unwrap();
    let dir_arg = dir.to_str().unwrap();

    let original = format!(
        "PII-SKILL\npodman-probe\nWHEN TO USE: a container is unhealthy\nscript: scripts/probe.sh sha256={real}\n"
    );
    let store = MemStore::open(&dir.join("memory.redb")).unwrap();
    let id = put(&store, &original);
    drop(store);

    // A body that drops the pin line is refused and leaves the stored body untouched.
    let dropped = root.path().join("dropped.md");
    std::fs::write(
        &dropped,
        "PII-SKILL\npodman-probe\nWHEN TO USE: a container is unhealthy, always\n",
    )
    .unwrap();
    let refused = pii_in(
        root.path(),
        &[
            "skill",
            "edit",
            &id,
            "--file",
            dropped.to_str().unwrap(),
            "--dir",
            dir_arg,
        ],
    );
    assert_ne!(refused.status.code(), Some(0), "{refused:?}");
    let store = MemStore::open(&dir.join("memory.redb")).unwrap();
    assert_eq!(decode_text(store.doc(&id).unwrap()), original);
    drop(store);

    // The same edit with the pin kept is written, and the id does not change.
    let kept = root.path().join("kept.md");
    let edited = format!(
        "PII-SKILL\npodman-probe\nWHEN TO USE: a container is unhealthy, always\nscript: scripts/probe.sh sha256={real}\n"
    );
    std::fs::write(&kept, &edited).unwrap();
    let ok = pii_in(
        root.path(),
        &[
            "skill",
            "edit",
            &id,
            "--file",
            kept.to_str().unwrap(),
            "--dir",
            dir_arg,
        ],
    );
    assert_eq!(ok.status.code(), Some(0), "{ok:?}");
    let store = MemStore::open(&dir.join("memory.redb")).unwrap();
    assert_eq!(decode_text(store.doc(&id).unwrap()), edited);
}
