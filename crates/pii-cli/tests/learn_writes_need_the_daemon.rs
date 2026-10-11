//! Spec 138 R1/R5(b): `pii learn promote|revert|promotions --sweep` write
//! nothing themselves — the daemon is the ledger's only writer — so with no
//! daemon they fail closed, name the daemon, and leave `learn/` untouched.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary and reads its stderr.
#![allow(clippy::disallowed_methods)]

fn pii(args: &[&str], runtime: &std::path::Path) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(args)
        .env("PII_RUNTIME_DIR", runtime)
        .output()
        .unwrap()
}

#[test]
fn the_cli_refuses_to_write_the_ledger_without_the_daemon() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().to_str().unwrap();
    for args in [
        vec![
            "learn",
            "promote",
            "c-0",
            "--metric",
            "graph_failed_runs",
            "--window-secs",
            "10",
            "--dir",
            dir,
        ],
        vec!["learn", "revert", "p-0000000000000000", "--dir", dir],
        vec!["learn", "promotions", "--sweep", "--dir", dir],
    ] {
        let out = pii(&args, d.path());
        assert_ne!(out.status.code(), Some(0), "{args:?} must fail: {out:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("the daemon owns the ledger"),
            "{args:?} must name the daemon as the writer, got: {err}"
        );
    }
    assert!(
        !d.path().join("learn/ledger.jsonl").exists(),
        "no ledger may be written without the daemon"
    );
}
