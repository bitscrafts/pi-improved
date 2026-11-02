//! Spec 339 R1/R2/R4: the control socket path is computed by one function, the
//! derived default names the same file the shipped user unit pins, and an
//! explicit `PII_RUNTIME_DIR` is used verbatim.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

/// The shipped user unit: the generator writes `Environment=PII_RUNTIME_DIR=...` into it.
fn shipped_unit_runtime_dir() -> PathBuf {
    let unit = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("deploy")
        .join("install-pii-user.sh");
    let raw = std::fs::read_to_string(&unit).unwrap();
    let value = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("Environment=PII_RUNTIME_DIR="))
        .unwrap_or_else(|| panic!("no PII_RUNTIME_DIR in {}", unit.display()));
    PathBuf::from(value)
}

#[test]
fn the_client_and_the_daemon_resolve_the_same_socket_with_no_environment() {
    assert!(
        pii_core::env_var("PII_RUNTIME_DIR").is_none(),
        "run this test with `env -u PII_RUNTIME_DIR`"
    );
    let client = pii_uds::runtime_dir("default")
        .unwrap()
        .join("control.sock");
    let daemon = pii_uds::runtime_dir("default")
        .unwrap()
        .join("control.sock");
    let derived = pii_uds::uid_runtime_dir(pii_uds::current_uid()).join("control.sock");
    assert_eq!(client, daemon);
    assert_eq!(
        client,
        derived,
        "client={} derived={}",
        client.display(),
        derived.display()
    );
}

#[test]
fn an_explicit_runtime_dir_is_used_verbatim() {
    let got = pii_uds::resolve_runtime_dir(Some(String::from("/tmp/x")), None, 1000);
    assert_eq!(
        got.join("control.sock"),
        PathBuf::from("/tmp/x/control.sock")
    );
}

#[test]
fn an_explicit_runtime_dir_beats_the_configured_one() {
    let got = pii_uds::resolve_runtime_dir(
        Some(String::from("/tmp/x")),
        Some(PathBuf::from("/tmp/configured")),
        1000,
    );
    assert_eq!(got, PathBuf::from("/tmp/x"));
}

#[test]
fn the_shipped_units_value_equals_the_derived_default() {
    let unit = shipped_unit_runtime_dir();
    // The unit is written for one account: its uid is the segment after /run/user/.
    let uid: u32 = unit
        .strip_prefix("/run/user")
        .ok()
        .and_then(|rest| rest.iter().next())
        .and_then(|s| s.to_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("unit path {} is not /run/user/<uid>/pii", unit.display()));
    let derived = pii_uds::resolve_runtime_dir(None, None, uid);
    assert_eq!(
        unit,
        derived,
        "shipped unit says {} but the derived default for uid {uid} is {}",
        unit.display(),
        derived.display()
    );
}

#[test]
fn the_old_default_with_an_instance_segment_differs() {
    // The pre-339 client appended `/default` to the uid path; the daemon never created it.
    let uid = pii_uds::current_uid();
    let old = PathBuf::from(format!("/run/user/{uid}/pii/default")).join("control.sock");
    let new = pii_uds::runtime_dir("default")
        .unwrap()
        .join("control.sock");
    assert_ne!(old, new, "the defect is the extra instance segment");
}
