//! A child that starts a background process and exits must not leave that process running: the coder spawns an
//! agent, the agent may start a server or a watcher, and when the node ends nothing it started should outlive it.
//! The stand-in matrix measured this: `sleep 100000` started by the agent was still running, parent pid 1, after the
//! node reported `succeeded`. Controls: a child that starts nothing is unaffected, and a grandchild in ANOTHER session
//! is left alone only if it escaped the group on purpose (setsid), which is the documented limit.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::time::{Duration, Instant};

use pii_abi::CapKind;
use pii_core::{ArgvScope, Budget, CapSet, ChildProc, Ctx};

fn ctx() -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    caps.proc_argv0 = ArgvScope::new(vec![String::from("/bin/sh")]);
    Ctx::root(caps, Budget::unlimited())
}

fn alive(pid: i32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
        && std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| !s.contains(") Z "))
}

/// Starts `script` under /bin/sh, waits for it to exit, and returns the child plus the pid it wrote to
/// `pidfile`. The caller decides when the group is reaped, so a test that must let a grandchild leave the
/// group first can do so before the kill.
fn spawn_and_read_pid(script: &str, pidfile: &Path) -> (ChildProc, i32) {
    let ctx = ctx();
    let argv = vec![
        String::from("/bin/sh"),
        String::from("-c"),
        script.to_owned(),
    ];
    let mut child = ctx
        .proc()
        .unwrap()
        .spawn_child_exact(&argv, Path::new("."), &[])
        .unwrap();
    let code = child.wait_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(code, Some(0), "the parent exits cleanly");
    let pid = std::fs::read_to_string(pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    (child, pid)
}

/// Starts `script`, waits for it to exit, reaps its process group and returns the pid it wrote to `pidfile`.
fn run_and_read_pid(script: &str, pidfile: &Path) -> i32 {
    let (child, pid) = spawn_and_read_pid(script, pidfile);
    child.reap_group();
    std::thread::sleep(Duration::from_millis(300));
    pid
}

/// Waits until `pid` has its own session (`setsid` has completed), so a kill of the child's process group can
/// no longer reach it. Returns false when the deadline passes first; the caller decides whether that is a
/// failure. The condition is read from `/proc`, never guessed with a fixed sleep.
fn wait_for_own_session(pid: i32, d: Duration) -> bool {
    let deadline = Instant::now() + d;
    loop {
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            // After the `(comm)` field the columns are state, ppid, pgrp, session; `comm` may contain spaces
            // and parentheses, so the split starts after the last ')'.
            let session = stat
                .rsplit_once(')')
                .and_then(|(_, rest)| rest.split_whitespace().nth(3).map(str::to_owned))
                .and_then(|s| s.parse::<i32>().ok());
            if session == Some(pid) {
                return true;
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_tmp/childproc-group");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{tag}-{}", std::process::id()))
}

#[test]
fn a_background_process_the_child_started_does_not_outlive_it() {
    let pidfile = scratch("bg");
    let script = format!("sleep 100000 & echo $! > {}", pidfile.display());
    let pid = run_and_read_pid(&script, &pidfile);
    assert!(
        !alive(pid),
        "the background process {pid} must be gone once the child's group is reaped"
    );
}

#[test]
fn a_child_that_started_nothing_is_unaffected() {
    let pidfile = scratch("none");
    let script = format!("echo $$ > {}", pidfile.display());
    let pid = run_and_read_pid(&script, &pidfile);
    assert!(!alive(pid), "the child itself has exited");
}

#[test]
fn killing_a_child_also_kills_what_it_started() {
    // The cancel path calls `kill`, not `reap_group`: a cancelled run must not leave the agent's grandchildren either.
    let pidfile = scratch("kill");
    let script = format!(
        "sleep 100000 & echo $! > {}; sleep 100000",
        pidfile.display()
    );
    let ctx = ctx();
    let argv = vec![String::from("/bin/sh"), String::from("-c"), script];
    let mut child = ctx
        .proc()
        .unwrap()
        .spawn_child_exact(&argv, Path::new("."), &[])
        .unwrap();
    std::thread::sleep(Duration::from_millis(400));
    child.kill().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let pid: i32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        !alive(pid),
        "the grandchild {pid} must die with a killed child"
    );
}

#[test]
fn a_process_that_left_the_group_on_purpose_is_the_documented_limit() {
    // setsid puts the grandchild in its own session and group: killpg on the child's group cannot reach it. This
    // control pins the limit instead of hiding it, so a stronger guarantee (a cgroup) is a conscious later change.
    let pidfile = scratch("setsid");
    let script = format!("setsid sleep 100000 & echo $! > {}", pidfile.display());
    let (child, pid) = spawn_and_read_pid(&script, &pidfile);
    // The parent exits as soon as the background job is started, so the group kill below can win the race
    // against the grandchild's own setsid() and kill the process this test is about. Wait for the session
    // change that IS the limit, then reap: measuring the limit, not the scheduler.
    assert!(
        wait_for_own_session(pid, Duration::from_secs(5)),
        "the grandchild {pid} must reach its own session before the group is killed"
    );
    child.reap_group();
    std::thread::sleep(Duration::from_millis(300));
    let escaped = alive(pid);
    // Clean up the test's own process through the gated spawn, so it does not linger.
    let ctx = ctx();
    let argv = vec![
        String::from("/bin/sh"),
        String::from("-c"),
        format!("kill {pid}"),
    ];
    let mut killer = ctx
        .proc()
        .unwrap()
        .spawn_child_exact(&argv, Path::new("."), &[])
        .unwrap();
    let _ = killer.wait_timeout(Duration::from_secs(5));
    assert!(
        escaped,
        "documenting the limit: a setsid'd grandchild survives a group kill ({pid})"
    );
}
