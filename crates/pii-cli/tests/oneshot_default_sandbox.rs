//! Spec 108 R4: `pii -p` is confined unless the operator opts out, like the
//! daemon (spec 107 R1). Where confinement cannot be enforced the default
//! run FAILS (exit 4); only an explicit `--sandbox off` gets past.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the tests run the built `pii` binary.
#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::{Command, Output};

fn oneshot(extra: &[&str], force_unsupported: bool, traj_outside: bool) -> (Output, bool) {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&scratch).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("osbx-")
        .tempdir_in(&scratch)
        .unwrap();
    let outside = tempfile::Builder::new()
        .prefix("osbx-out-")
        .tempdir_in(&scratch)
        .unwrap();
    let model = dir.path().join("model.json");
    std::fs::write(&model, r#"{"turns": [{"text": "ok"}]}"#).unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_pii"));
    c.current_dir(dir.path())
        .args(["-p", "hi", "--json", "--connect", "loopback"])
        .args(extra)
        .arg("--model")
        .arg(format!("scripted:{}", model.display()))
        .env("PII_CONFIG", dir.path().join("absent.toml"));
    if traj_outside {
        c.env("PII_TRAJECTORY_DIR", outside.path().join("traj"));
    }
    if force_unsupported {
        c.env("PII_SANDBOX_FORCE_UNSUPPORTED", "1");
    }
    let out = c.output().unwrap();
    let wrote = outside.path().join("traj/trajectories").exists();
    (out, wrote)
}

#[test]
fn default_run_is_refused_when_confinement_cannot_be_enforced() {
    let (o, _) = oneshot(&[], true, false);
    assert_eq!(
        o.status.code(),
        Some(4),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn explicit_off_runs_even_where_confinement_is_unavailable() {
    let (o, _) = oneshot(&["--sandbox", "off"], true, false);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn default_run_is_confined_yet_completes_and_writes_its_trajectory_root() {
    // Landlock is available here: the confined run still completes, and a
    // trajectory root OUTSIDE the workspace is granted read-write.
    let (o, wrote) = oneshot(&[], false, true);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        wrote,
        "trajectory root outside the workspace was not written"
    );
}
