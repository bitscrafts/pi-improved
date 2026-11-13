//! Spec 394: a lock is held by a PROCESS, so it is released when that process is gone, not when a timestamp
//! says it is old. A killed test used to leave the file and every later test waited fifteen minutes.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: a test file.
#![allow(non_snake_case, clippy::disallowed_methods)]
// reason: the CONTROL_ names are the spec contract, and the test spawns a real `sleep` to own a pid.

#[path = "heavy_lock/mod.rs"]
pub mod heavy_lock;

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

fn lock_path(tag: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), tag).unwrap();
    let path = dir.path().join("heavy.lock");
    (dir, path)
}

/// A process that is alive until killed, and its pid.
fn sleeper() -> (Child, u32) {
    let child = Command::new("sleep")
        .arg("600")
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    (child, pid)
}

#[test]
fn a_dead_holder_is_reaped_immediately() {
    let (_dir, path) = lock_path("heavy-dead-");
    let (mut child, pid) = sleeper();
    std::fs::write(&path, format!("{pid}\n")).unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    let started = Instant::now();
    let guard = heavy_lock::take(&path, Duration::from_secs(30));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a lock whose holder is dead must be taken at once, took {:?}",
        started.elapsed()
    );
    assert_eq!(heavy_lock::holder_pid(&path), Some(std::process::id()));
    drop(guard);
}

// CONTROL: a LIVE holder must NOT be reaped, however fresh or old the file looks; the waiter gives up and
// names the pid it was waiting on.
#[test]
fn CONTROL_a_live_holder_is_not_reaped_and_is_named() {
    let (_dir, path) = lock_path("heavy-live-");
    let (mut child, pid) = sleeper();
    std::fs::write(&path, format!("{pid}\n")).unwrap();
    let result = std::panic::catch_unwind(|| {
        let _ = heavy_lock::take(&path, Duration::from_millis(400));
    });
    let _ = child.kill();
    let _ = child.wait();
    let message = *result
        .expect_err("a live holder keeps the lock")
        .downcast::<String>()
        .unwrap();
    assert!(
        message.contains(&format!("holder pid Some({pid})")),
        "{message}"
    );
}

// CONTROL: a lock file with NO readable pid is not a live holder, and is not reaped on sight either: only age
// stales it. A file written one second ago with no pid is still waited for.
#[test]
fn CONTROL_a_fresh_lock_with_no_pid_is_waited_for() {
    let (_dir, path) = lock_path("heavy-nopid-");
    std::fs::write(&path, "not a pid\n").unwrap();
    let result = std::panic::catch_unwind(|| {
        let _ = heavy_lock::take(&path, Duration::from_millis(300));
    });
    assert!(result.is_err(), "a fresh lock with no pid is still held");
}

#[test]
fn a_released_lock_is_taken_by_the_next_caller() {
    let (_dir, path) = lock_path("heavy-free-");
    let first = heavy_lock::take(&path, Duration::from_secs(5));
    drop(first);
    let second = heavy_lock::take(&path, Duration::from_secs(5));
    assert_eq!(heavy_lock::holder_pid(&path), Some(std::process::id()));
    drop(second);
}

fn mtime(path: &std::path::Path) -> SystemTime {
    std::fs::metadata(path).unwrap().modified().unwrap()
}

fn set_mtime(path: &std::path::Path, when: SystemTime) {
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

// A holder that keeps its heartbeat is progressing, so a waiter whose budget is 1 s is waited for past it: the
// holder holds for 3 s, and the waiter acquires only after the holder drops.
#[test]
fn a_holder_that_keeps_its_heartbeat_is_waited_for_past_the_budget() {
    let (_dir, path) = lock_path("heavy-beat-");
    let (held_tx, held_rx) = std::sync::mpsc::channel();
    let holder_path = path.clone();
    let holder = std::thread::spawn(move || {
        let guard = heavy_lock::take(&holder_path, Duration::from_secs(5));
        held_tx.send(()).unwrap();
        std::thread::sleep(Duration::from_secs(3));
        drop(guard);
    });
    held_rx.recv().unwrap();
    let started = Instant::now();
    let second = heavy_lock::take(&path, Duration::from_secs(1));
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "the waiter must have waited for the holder, took {:?}",
        started.elapsed()
    );
    drop(second);
    holder.join().unwrap();
}

// The heartbeat touches the lock only while the file names this process. The first half shows the heartbeat
// runs at all, so this cannot pass on a lock that never refreshes; the second half shows it stops once another
// pid is written, by holding the mtime at a marker across three heartbeats.
#[test]
fn a_heartbeat_stops_when_the_lock_no_longer_names_this_process() {
    let (_dir, path) = lock_path("heavy-stop-");
    let guard = heavy_lock::take(&path, Duration::from_secs(5));
    let (mut other, other_pid) = sleeper();

    let marker = SystemTime::now() - Duration::from_secs(1000);
    set_mtime(&path, marker);
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        mtime(&path) > marker,
        "a held lock that names this process must be refreshed by its heartbeat"
    );

    std::fs::write(&path, format!("{other_pid}\n")).unwrap();
    set_mtime(&path, marker);
    std::thread::sleep(Duration::from_secs(7));
    assert_eq!(
        mtime(&path),
        marker,
        "the heartbeat must stop once the lock names another process"
    );

    drop(guard);
    let _ = other.kill();
    let _ = other.wait();
}
