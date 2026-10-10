//! Spec 109 R5: `pii tui` confines by default, like `pii -p` (spec 108 R4) and
//! `pii daemon` (spec 107 R1); only an explicit `--sandbox off` opts out.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the tests run the built `pii` binary.
#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::{Command, Output, Stdio};

fn tui(extra: &[&str]) -> Output {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&scratch).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("tsbx-")
        .tempdir_in(scratch)
        .unwrap();
    Command::new(env!("CARGO_BIN_EXE_pii"))
        .arg("tui")
        .args(["--connect", "loopback", "--workspace"])
        .arg(dir.path())
        .args(extra)
        .env("PII_CONFIG", dir.path().join("absent.toml"))
        .env("PII_SANDBOX_FORCE_UNSUPPORTED", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn default_tui_is_refused_when_confinement_cannot_be_enforced() {
    let o = tui(&[]);
    assert_eq!(
        o.status.code(),
        Some(4),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn explicit_off_gets_past_the_sandbox_step() {
    let o = tui(&["--sandbox", "off"]);
    assert_ne!(
        o.status.code(),
        Some(4),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}
