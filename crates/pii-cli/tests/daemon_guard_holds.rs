//! Spec 355: the daemon slot's file is removed when its last holder drops, whichever test took it first, and a
//! second taker waits while another thread holds it. The spec 331 guard tests moved to `daemon_guard_reaps.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "daemon_guard/mod.rs"]
mod daemon_guard;

use parking_lot::Mutex;
use std::time::Duration;

/// The slot file is process-wide, so these tests run one at a time, each on a private slot file (see `use_private_slot`):
/// a parallel test would remove or take the file under the other's assertions.
static SERIAL: Mutex<()> = Mutex::new(());

#[test]
fn the_slot_file_is_removed_when_the_last_holder_drops() {
    let _serial = SERIAL.lock();
    let dir = tempfile::tempdir().unwrap();
    daemon_guard::use_private_slot(dir.path().join("daemon-slot.lock"));
    let a = daemon_guard::slot();
    let b = daemon_guard::slot();
    drop(a);
    drop(b);
    assert!(
        !daemon_guard::slot_path().exists(),
        "the slot file {} leaked after both holders dropped",
        daemon_guard::slot_path().display()
    );
}

#[test]
#[allow(non_snake_case)] // reason: the spec names this test with the CONTROL_ prefix
fn CONTROL_a_second_acquire_while_held_waits() {
    let _serial = SERIAL.lock();
    let dir = tempfile::tempdir().unwrap();
    daemon_guard::use_private_slot(dir.path().join("daemon-slot.lock"));
    let a = daemon_guard::slot();
    let (took, taken) = std::sync::mpsc::channel();
    let second = std::thread::spawn(move || {
        let b = daemon_guard::slot();
        took.send(()).unwrap();
        b
    });
    assert!(
        taken.recv_timeout(Duration::from_millis(500)).is_err(),
        "a second acquire from another thread must wait while the first holds the slot"
    );
    drop(a);
    taken
        .recv_timeout(Duration::from_secs(10))
        .expect("the second acquire takes the slot once the first drops");
    drop(second.join().unwrap());
}
