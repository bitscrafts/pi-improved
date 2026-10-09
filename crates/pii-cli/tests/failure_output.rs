//! Spec 101: operator-facing failures now flow through `cli.failure`, but the
//! text an operator sees on stderr must not change.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary to check its stderr text.
#![allow(clippy::disallowed_methods)]

#[test]
fn graph_lint_of_a_missing_file_still_prints_the_error_line() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(["graph", "lint", "/nonexistent/spec101-absent.toml"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(
        err.starts_with("error: read /nonexistent/spec101-absent.toml: "),
        "stderr was {err:?}"
    );
}
