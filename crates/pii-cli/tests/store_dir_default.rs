//! Spec 351: the store defaults read the state dir, the socket stays in the runtime dir.
//! A default `pii backup` must copy the live store (`PII_STATE_DIR`), not the stale runtime-dir copy.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the tests run the built `pii` binary.
#![allow(clippy::disallowed_methods)]
// reason: the spec names its CONTROL tests in upper case, as the contract table does.
#![allow(non_snake_case)]

use std::path::Path;
use std::process::{Command, Output};

fn scratch(name: &str) -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix(name)
        .tempdir_in(root)
        .unwrap()
}

/// A real store at `dir/store.redb`: the backup opens it, so it must be a database. Opened through
/// `pii-store` (deps-check invariant 5: `pii-cli` may not name `redb` itself).
fn make_store(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    drop(pii_store::Store::open(&dir.join("store.redb")).unwrap());
}

fn blake3_of(path: &Path) -> String {
    blake3::hash(&std::fs::read(path).unwrap())
        .to_hex()
        .to_string()
}

fn pii(args: &[&str], env: &[(&str, &Path)], home: &Path) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_pii"));
    c.args(args)
        .env("HOME", home)
        .env_remove("PII_MODEL")
        .env_remove("PII_CONFIG")
        .env_remove("PII_STATE_DIR")
        .env_remove("PII_RUNTIME_DIR");
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().unwrap()
}

#[test]
fn a_backup_default_reads_the_state_dir_not_the_runtime_dir() {
    let a = scratch("rt-a-");
    let b = scratch("st-b-");
    let home = scratch("home-");
    let out = scratch("out-");
    make_store(b.path());
    let dest = out.path().join("backup");

    let o = pii(
        &["backup", dest.to_str().unwrap()],
        &[("PII_RUNTIME_DIR", a.path()), ("PII_STATE_DIR", b.path())],
        home.path(),
    );
    let want = blake3_of(&b.path().join("store.redb"));
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert_eq!(
        o.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        stdout.contains("store.redb (") && stdout.contains(&want),
        "manifest must hash the state-dir store {want}; stdout={stdout}"
    );
}

#[test]
fn CONTROL_an_empty_state_dir_env_falls_back_to_home() {
    let a = scratch("rt-a-");
    let home = scratch("home-");
    let out = scratch("out-");
    let state = home.path().join(".local/state/pii");
    make_store(&state);
    let dest = out.path().join("backup");

    let o = pii(
        &["backup", dest.to_str().unwrap()],
        &[
            ("PII_RUNTIME_DIR", a.path()),
            ("PII_STATE_DIR", Path::new("")),
        ],
        home.path(),
    );
    let want = blake3_of(&state.join("store.redb"));
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(
        stdout.contains(&want),
        "an empty PII_STATE_DIR must fall back to $HOME/.local/state/pii; stdout={stdout} stderr={}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn CONTROL_the_socket_still_comes_from_the_runtime_dir() {
    let a = scratch("rt-a-");
    let b = scratch("st-b-");
    let home = scratch("home-");
    let o = pii(
        &["health"],
        &[("PII_RUNTIME_DIR", a.path()), ("PII_STATE_DIR", b.path())],
        home.path(),
    );
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    let a_str = a.path().display().to_string();
    let b_str = b.path().display().to_string();
    assert!(
        text.contains(&a_str),
        "socket must be under the runtime dir {a_str}; got {text}"
    );
    assert!(
        !text.contains(&b_str),
        "socket must not be under the state dir {b_str}; got {text}"
    );
}
