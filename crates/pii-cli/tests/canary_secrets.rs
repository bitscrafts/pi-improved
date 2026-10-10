//! Spec 107 R4: a secret that reaches a run's output (here an env var the
//! graph's bash node echoes, standing in for a model reply that repeats it)
//! must not appear in the ndjson stream or the trajectory file. The canary is
//! generated per run — it is never written into a tracked file.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary and reads its artefacts.
#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;

fn canary() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("cnry{}x{nanos}", std::process::id())
}

/// Runs `pii -p` against a scripted model whose reply repeats `value`, with
/// `var=value` in the environment; returns (ndjson stdout, trajectory text).
fn run_reply(var: &str, value: &str, redact: Option<&str>) -> (String, String) {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&scratch).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("canary-")
        .tempdir_in(scratch)
        .unwrap();
    let model = dir.path().join("model.json");
    let reply = serde_json::json!({"turns": [{"text": format!("the key is {value}")}]});
    std::fs::write(&model, reply.to_string()).unwrap();
    let traj = dir.path().join("traj");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pii"));
    cmd.current_dir(dir.path())
        .args(["-p", "what is the key", "--json", "--connect", "loopback"])
        .arg("--model")
        .arg(format!("scripted:{}", model.display()))
        .env(var, value)
        .env("PII_TRAJECTORY_DIR", &traj);
    if let Some(r) = redact {
        cmd.env("PII_TRAJECTORY_REDACT", r);
    }
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut trajectory = String::new();
    for f in std::fs::read_dir(traj.join("trajectories")).unwrap() {
        trajectory.push_str(&std::fs::read_to_string(f.unwrap().path()).unwrap());
    }
    (stdout, trajectory)
}

#[test]
fn a_secret_named_env_var_never_reaches_ndjson_or_the_trajectory() {
    let c = canary();
    let (ndjson, traj) = run_reply("CANARY_API_TOKEN", &c, None);
    assert_eq!(ndjson.matches(&c).count(), 0, "canary in ndjson:\n{ndjson}");
    assert_eq!(traj.matches(&c).count(), 0, "canary in trajectory:\n{traj}");
    // The echo really ran and was recorded (the value is replaced, not lost).
    assert!(ndjson.contains("[REDACTED]"), "{ndjson}");
    assert!(traj.contains("[REDACTED]"), "{traj}");
}

#[test]
fn positive_control_a_non_secret_value_is_recorded_so_the_test_can_fail() {
    let c = canary();
    let (ndjson, traj) = run_reply("CANARY_NOTES", &c, None);
    assert!(
        ndjson.matches(&c).count() >= 1,
        "control missing from ndjson"
    );
    assert!(
        traj.matches(&c).count() >= 1,
        "control missing from trajectory"
    );
}

#[test]
fn configured_redaction_drops_the_field_from_the_trajectory() {
    let c = canary();
    let (ndjson, traj) = run_reply("CANARY_NOTES", &c, Some("text"));
    assert_eq!(traj.matches(&c).count(), 0, "redaction missed:\n{traj}");
    assert!(
        ndjson.matches(&c).count() >= 1,
        "control: ndjson is not redacted"
    );
}
