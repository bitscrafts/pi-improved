//! A coder agent that never exits must be stopped by the node's own timeout, not wait forever. The matrix measured a
//! hung agent: still `running` after 40 s, `run cancel` refused because a node was executing, and no deadline anywhere
//! (`with_deadline` has no caller, `max_wall_ms` is only checked between nodes). The timeout is enforced in
//! `wait_child`: on expiry the child's whole process group is killed and the launch ends with exit code 124, the
//! `timeout(1)` convention, so the journal carries a real `coder.exited` with a non-zero rc.
//! Controls: a child that finishes inside the timeout is untouched, and no timeout means the old behaviour.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::time::{Duration, Instant};

use pii_abi::CapKind;
use pii_core::{ArgvScope, Budget, CapSet, Ctx};
use pii_server::node_kinds::nodes::coder_runtime::{Launch, TIMEOUT_EXIT_CODE, wait_child};

fn ctx() -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    caps.proc_argv0 = ArgvScope::new(vec![String::from("/bin/sh")]);
    Ctx::root(caps, Budget::unlimited())
}

fn run(script: &str, timeout: Option<Duration>) -> (Launch, Duration) {
    let argv = vec![
        String::from("/bin/sh"),
        String::from("-c"),
        script.to_owned(),
    ];
    let started = Instant::now();
    let launch = wait_child(&ctx(), &argv, Path::new("."), &[], timeout, &mut |_| {}).unwrap();
    (launch, started.elapsed())
}

fn code_of(launch: &Launch) -> i32 {
    match launch {
        Launch::Exited { code, .. } => *code,
        Launch::Refused(_) => panic!("refused"),
    }
}

#[test]
fn a_hung_child_is_stopped_at_the_timeout() {
    let (launch, took) = run("sleep 100000", Some(Duration::from_millis(400)));
    assert_eq!(code_of(&launch), TIMEOUT_EXIT_CODE, "a timeout exits 124");
    assert!(
        took < Duration::from_secs(5),
        "the timeout must end the wait promptly, took {took:?}"
    );
}

#[test]
fn a_timeout_also_stops_what_the_child_started() {
    // The hung agent in the matrix had a `sleep` child; killing only the leader would leave it running.
    let (launch, _) = run(
        "sleep 100000 & sleep 100000",
        Some(Duration::from_millis(400)),
    );
    assert_eq!(code_of(&launch), TIMEOUT_EXIT_CODE);
}

#[test]
fn a_child_that_finishes_in_time_is_untouched() {
    let (launch, took) = run("exit 0", Some(Duration::from_secs(30)));
    assert_eq!(
        code_of(&launch),
        0,
        "near-miss: inside the timeout nothing changes"
    );
    assert!(took < Duration::from_secs(5));
}

#[test]
fn a_failing_child_keeps_its_own_exit_code() {
    let (launch, _) = run("exit 3", Some(Duration::from_secs(30)));
    assert_eq!(
        code_of(&launch),
        3,
        "a real exit code is not rewritten to 124"
    );
}

#[test]
fn no_timeout_waits_for_the_child_as_before() {
    let (launch, _) = run("sleep 1; exit 0", None);
    assert_eq!(code_of(&launch), 0);
}
