//! Spec 107 R1/R2: the `workspace` profile's grant, pinned by behaviour.
//!
//! Writes outside the granted roots are denied; reads outside them are
//! ALLOWED (the profile grants read-only `/`). A change to either is a
//! failing test, not a silent policy drift. The `off` profile is the control
//! that proves the denial comes from the sandbox and not from the test setup.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the tests run the built `pii` binary.
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Rig {
    _dir: tempfile::TempDir,
    ws: PathBuf,
    outside: PathBuf,
}

fn rig() -> Rig {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&scratch).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("sbx-")
        .tempdir_in(scratch)
        .unwrap();
    let ws = dir.path().join("ws");
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    Rig {
        _dir: dir,
        ws,
        outside,
    }
}

fn run(rig: &Rig, profile: &str, script: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(["sandbox-exec", "--profile", profile, "--workspace"])
        .arg(&rig.ws)
        .args(["--", "/bin/sh", "-c", script])
        // $TMPDIR is a granted root; keep it inside the workspace so the
        // scratch dir above is not accidentally writable.
        .env("TMPDIR", &rig.ws)
        .output()
        .unwrap()
}

fn rc(o: &Output) -> i32 {
    o.status.code().unwrap_or(-1)
}

#[test]
fn workspace_profile_allows_a_write_inside_the_workspace() {
    let r = rig();
    let o = run(&r, "workspace", "echo hi > inside.txt");
    assert_eq!(rc(&o), 0, "{}", String::from_utf8_lossy(&o.stderr));
    assert!(r.ws.join("inside.txt").exists());
}

#[test]
fn workspace_profile_denies_a_write_outside_and_leaves_an_existing_file_intact() {
    let r = rig();
    let target = r.outside.join("new.txt");
    // The in-workspace write in the same invocation proves the sandboxed
    // child really ran: a launch failure cannot pass as "denied".
    let o = run(
        &r,
        "workspace",
        &format!("echo x > ran.txt; echo x > {}", target.display()),
    );
    assert!(r.ws.join("ran.txt").exists(), "child did not run: {o:?}");
    assert_ne!(rc(&o), 0, "write outside must be denied");
    assert!(!target.exists(), "the file must not have been created");

    let existing = r.outside.join("keep.txt");
    std::fs::write(&existing, "original").unwrap();
    let o = run(
        &r,
        "workspace",
        &format!("echo x > ran2.txt; echo x > {}", existing.display()),
    );
    assert!(r.ws.join("ran2.txt").exists(), "child did not run: {o:?}");
    assert_ne!(rc(&o), 0, "clobbering outside must be denied");
    assert_eq!(std::fs::read_to_string(&existing).unwrap(), "original");
}

#[test]
fn workspace_profile_still_allows_reads_outside_the_workspace() {
    let r = rig();
    let secret = r.outside.join("readable.txt");
    std::fs::write(&secret, "visible").unwrap();
    let o = run(&r, "workspace", &format!("cat {}", secret.display()));
    assert_eq!(rc(&o), 0, "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "visible");
    // /etc is readable too (DNS, CA bundles).
    assert_eq!(rc(&run(&r, "workspace", "cat /etc/passwd > /dev/null")), 0);
}

/// Spec 229 R1: the workspace grant includes `/dev/shm` (podman's rootless lock dir), and the
/// grant is the shm directory only: a write beside it in the same child still fails.
#[test]
fn workspace_profile_allows_a_write_inside_dev_shm_and_denies_dev_outside_it() {
    let r = rig();
    let shm = PathBuf::from("/dev/shm").join(format!("pii-229-probe-{}", std::process::id()));
    let target = r.outside.join("shm-denied.txt");
    let o = run(
        &r,
        "workspace",
        &format!(
            "echo x > {} && echo x > ran.txt; echo x > {}",
            shm.display(),
            target.display()
        ),
    );
    let shm_written = shm.exists();
    let _ = std::fs::remove_file(&shm);
    assert!(
        shm_written,
        "a write inside /dev/shm must succeed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(r.ws.join("ran.txt").exists(), "child did not run: {o:?}");
    assert_ne!(rc(&o), 0, "a write outside the grant must still be denied");
    assert!(!target.exists(), "the file must not have been created");
}

#[test]
fn off_profile_control_permits_the_write_the_workspace_profile_denies() {
    let r = rig();
    let target = r.outside.join("off.txt");
    let o = run(&r, "off", &format!("echo x > {}", target.display()));
    assert_eq!(rc(&o), 0, "{}", String::from_utf8_lossy(&o.stderr));
    assert!(target.exists());
}

/// Spec 228 R1: the pinned grant text names the container runtime's state dirs. The
/// assertion reads pii-tools' SOURCE via `include_str` so the test can never drift from a
/// stale copy of the text: the string it checks is the string that ships.
#[test]
fn the_pinned_grant_names_the_podman_state_dirs() {
    let src = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pii-tools/src/sandbox.rs"
    ));
    assert!(
        src.contains("HOME/.local/share/containers")
            && src.contains("XDG_RUNTIME_DIR/containers")
            && src.contains("XDG_RUNTIME_DIR/libpod"),
        "the workspace grant must name podman's state dirs (spec 228)"
    );
    assert!(
        src.contains("/dev/shm"),
        "the workspace grant must name podman's shm lock dir (spec 229)"
    );
    // The clause is in the Workspace arm's grant text, not merely in a comment: the helper
    // that produces the dirs is referenced by the same module.
    assert!(
        src.contains("pub fn podman_state_dirs"),
        "the dirs come from the grant helper, not a comment"
    );
}
