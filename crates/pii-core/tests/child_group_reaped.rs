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
use std::time::Duration;

use pii_abi::CapKind;
use pii_core::{ArgvScope, Budget, CapSet, Ctx};

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

/// Starts `script` under /bin/sh, waits for it to exit, and returns the pid it wrote to `pidfile`.
fn run_and_read_pid(script: &str, pidfile: &Path) -> i32 {
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
    child.reap_group();
    std::thread::sleep(Duration::from_millis(300));
    std::fs::read_to_string(pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
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
    let pid = run_and_read_pid(&script, &pidfile);
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
