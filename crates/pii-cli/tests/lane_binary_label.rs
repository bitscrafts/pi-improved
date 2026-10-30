//! Spec 320 acceptance: `pii lane rev` and the guard that refuses a binary built at another commit than HEAD.
//! The stale-label cases run the real binary against the lane's recording scaffold (`tests/lane_support`). A matching
//! label needs HEAD to be the binary's own commit, which no scratch repo can be, so the match and unknown-label
//! verdicts are checked on the pure guard, which this file includes from `src/cmd_lane/label.rs`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods, // reason: the tests drive git and the binary directly
    clippy::needless_pass_by_value
)]

#[allow(dead_code, clippy::redundant_pub_crate)]
#[path = "../src/cmd_lane/label.rs"]
mod label;

#[path = "lane_support/mod.rs"]
mod support;

use label::Guard;
use pii_server::daemon::identity::REVISION;
use std::fs;
use std::path::Path;
use std::process::Command;
use support::{BIN, Lane, executable, git, text};

/// A full HEAD sha for the pure cases; the guard compares its first twelve characters.
const SHA: &str = "abc123def456789012345678901234567890abcd";

/// HEAD's first twelve characters in the scratch repo, as `git rev-parse` reads them.
fn head_short(root: &Path) -> String {
    let out = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim()[..12].to_owned()
}

#[test]
fn lane_rev_prints_the_build_label() {
    let out = Command::new(BIN).args(["lane", "rev"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    assert_eq!(text(&out), format!("rev {REVISION}\n"));
}

#[test]
fn lane_rev_prints_a_dirty_label_verbatim() {
    let out = Command::new(BIN).args(["lane", "rev"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let printed = text(&out);
    assert_eq!(printed, format!("rev {REVISION}\n"));
    if let Some(commit) = REVISION.strip_suffix("-dirty") {
        assert!(
            printed.ends_with("-dirty\n"),
            "{printed} lost the suffix of {commit}"
        );
    }
}

#[test]
fn a_matching_label_runs_past_the_guard() {
    assert!(matches!(
        label::guard("abc123def456", Some(SHA), false),
        Guard::Run
    ));
    assert!(matches!(
        label::guard("abc123def456", Some(SHA), true),
        Guard::Run
    ));
}

#[test]
fn a_stale_label_aborts_with_the_rebuild_command() {
    let lane = Lane::new("stale-abort");
    let out = lane.spawn(false, None, &[], &[("PII_STALE_BIN_OK", "")]);
    let head = head_short(&lane.root);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out));
    assert_eq!(
        text(&out),
        format!(
            "ABORT: this binary was built at {REVISION}; HEAD is {head} — rebuild: cargo build -p pii-cli --bin pii\n"
        )
    );
}

#[test]
fn a_stale_label_records_no_transport_call() {
    let lane = Lane::new("stale-no-call");
    let marker = lane.root.join("daemon-ran");
    executable(
        &lane.stub.daemon,
        &format!(
            "#!/usr/bin/env bash\ntouch '{}'\necho up\n",
            marker.display()
        ),
    );
    let out = lane.spawn(false, None, &[], &[("PII_STALE_BIN_OK", "")]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out));
    assert!(
        lane.calls().is_empty(),
        "ssh was called: {:?}",
        lane.calls()
    );
    assert!(!marker.exists(), "the daemon was started");
}

#[test]
fn a_stale_label_writes_no_log_and_no_file() {
    let lane = Lane::new("stale-no-write");
    let out = lane.spawn(false, None, &[], &[("PII_STALE_BIN_OK", "")]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out));
    assert!(!lane.root.join("_tmp").exists(), "the abort created _tmp/");
    let status = Command::new("git")
        .current_dir(&lane.root)
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    assert!(status.stdout.is_empty(), "{}", text(&out));
}

#[test]
fn the_dirty_suffix_is_not_a_mismatch() {
    assert!(matches!(
        label::guard("abc123def456-dirty", Some(SHA), false),
        Guard::Run
    ));
    assert!(matches!(
        label::guard("abc123def456", Some(SHA), false),
        Guard::Run
    ));
}

#[test]
fn an_unknown_label_aborts() {
    assert!(matches!(
        label::guard("unknown", Some(SHA), false),
        Guard::Abort(_)
    ));
    assert!(matches!(
        label::guard("unknown", None, false),
        Guard::Abort(_)
    ));
}

#[test]
fn git_unreadable_aborts_with_unknown_sha() {
    assert!(matches!(
        label::guard(REVISION, None, false),
        Guard::Abort(line) if line.ends_with("HEAD is unknown — rebuild: cargo build -p pii-cli --bin pii")
    ));
    let lane = Lane::new("git-unreadable");
    git(&lane.root, &["update-ref", "-d", "HEAD"]);
    let out = lane.spawn(false, None, &[], &[("PII_STALE_BIN_OK", "")]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out));
    assert!(
        text(&out).contains("HEAD is unknown — rebuild:"),
        "{}",
        text(&out)
    );
}

#[test]
fn the_knob_prints_a_note_and_runs() {
    let lane = Lane::new("knob-note");
    let out = lane.spawn(false, None, &[], &[("PII_STALE_BIN_OK", "1")]);
    let stdout = text(&out);
    let head = head_short(&lane.root);
    assert!(
        stdout.contains(&format!(
            "NOTE: running a binary built at {REVISION} while HEAD is {head} (PII_STALE_BIN_OK=1)\n"
        )),
        "{stdout}"
    );
    assert!(
        stdout.contains("DAEMON   :"),
        "the run did not proceed: {stdout}"
    );
}

#[test]
fn the_knob_needs_exactly_1() {
    for value in ["true", "0", "yes"] {
        let lane = Lane::new("knob-exact");
        let out = lane.spawn(false, None, &[], &[("PII_STALE_BIN_OK", value)]);
        assert_eq!(out.status.code(), Some(2), "{value}: {}", text(&out));
        assert!(
            text(&out).starts_with(&format!("ABORT: this binary was built at {REVISION};")),
            "{value}: {}",
            text(&out)
        );
    }
}

#[test]
fn the_guard_reads_head_not_the_working_tree() {
    let lane = Lane::new("head-not-tree");
    fs::write(lane.root.join("specs/319-x.md"), "# edited by the worker\n").unwrap();
    let out = lane.spawn(false, None, &[], &[("PII_STALE_BIN_OK", "")]);
    let head = head_short(&lane.root);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out));
    assert!(
        text(&out).contains(&format!("HEAD is {head} —")),
        "the abort did not name HEAD: {}",
        text(&out)
    );
}
