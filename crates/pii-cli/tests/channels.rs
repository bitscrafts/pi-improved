//! Spec 155: the default channel set names only the kinds that are used, the dormant kinds stay
//! selectable, and the gate summary does not let `hal-gpiod` read as coverage.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs `quality-gate.sh --summary` and reads its output.
#![allow(clippy::disallowed_methods)]

use std::path::PathBuf;
use std::process::Command;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The kinds the default config declares: the value of its `# default channel kinds:` line.
fn default_kinds() -> Vec<String> {
    let raw = std::fs::read_to_string(repo().join("examples/pii.production.toml")).unwrap();
    let line = raw
        .lines()
        .find_map(|l| l.strip_prefix("# default channel kinds:"))
        .expect("the default config states its default channel kinds");
    let list = line.split('(').next().unwrap_or("");
    list.split(',').map(|k| k.trim().to_string()).collect()
}

#[test]
fn the_default_channel_set_names_only_used_kinds() {
    assert_eq!(default_kinds(), ["cli", "discord"]);
}

#[test]
fn a_dormant_kind_still_resolves_when_requested() {
    for kind in ["telegram", "matrix", "webhook"] {
        let raw = format!("[[channel]]\nname = \"c\"\nkind = \"{kind}\"\ngraph = \"g\"\n");
        let cfg = pii_server::daemon::config::Config::parse(&raw).unwrap();
        assert_eq!(cfg.channel.first().unwrap().kind, kind);
    }
}

#[test]
fn the_gate_summary_marks_gpiod_unexercised() {
    let out = Command::new("bash")
        .arg(repo().join("quality-gate.sh"))
        .arg("--summary")
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.lines()
            .any(|l| l.contains("hal-gpiod") && l.contains("UNEXERCISED")),
        "summary was: {text}"
    );
}
