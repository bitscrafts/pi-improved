//! Spec 115 R3b: `pii store compact` is offline, refuses while a daemon holds
//! the store, and reports measured before/after sizes (not claims).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

use pii_store::Store;

fn pii(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(args)
        .output()
        .unwrap()
}

fn fragmented(dir: &std::path::Path) {
    let store = Store::open(&dir.join("store.redb")).unwrap();
    for s in 0..20 {
        let id = format!("s{s}");
        store.put_session(&id, b"x").unwrap();
        for _ in 0..10 {
            store
                .append_session_message(&id, &vec![7u8; 40_000])
                .unwrap();
        }
    }
    for s in (0..20).step_by(2) {
        store.del_session(&format!("s{s}")).unwrap();
    }
}

#[test]
fn compact_reports_the_real_before_and_after_sizes() {
    let d = tempfile::tempdir().unwrap();
    fragmented(d.path());
    let file = d.path().join("store.redb");
    let before = std::fs::metadata(&file).unwrap().len();
    let out = pii(&["store", "compact", "--dir", d.path().to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let after = std::fs::metadata(&file).unwrap().len();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains(&format!("store.redb {before} -> {after} bytes")),
        "the printed sizes must be the measured ones: {text}"
    );
    assert!(after < before, "{before} -> {after}");
    // Shown with `--nocapture`: the measured pair, for the runbook.
    let _ = std::io::Write::write_fmt(
        &mut std::io::stderr(),
        format_args!("MEASURED fragmented store: {before} -> {after} bytes\n"),
    );
}

#[test]
fn compact_refuses_while_the_store_is_held() {
    let d = tempfile::tempdir().unwrap();
    let _held = Store::open(&d.path().join("store.redb")).unwrap();
    let before = std::fs::metadata(d.path().join("store.redb"))
        .unwrap()
        .len();
    let out = pii(&["store", "compact", "--dir", d.path().to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(
        String::from_utf8(out.stderr)
            .unwrap()
            .contains("stop the daemon")
    );
    assert_eq!(
        std::fs::metadata(d.path().join("store.redb"))
            .unwrap()
            .len(),
        before
    );
}
