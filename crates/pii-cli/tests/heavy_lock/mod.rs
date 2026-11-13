//! The lock the heavy tests in `control_plane_is_not_behind_runs` take turns with.
//!
//! A FILE, not a Mutex: `clippy.toml` bans `std::sync::Mutex` here, and a file lock survives the test
//! binary's own process boundaries. Spec 394: it is released when its HOLDER is gone, not when a timestamp
//! says it is old. The pid the file records is the check; age is only the fallback for a pid `/proc` cannot
//! see. Measured 2026-11-05: the pid was written and never read, so a killed test left the file and every
//! later test waited fifteen minutes, which the gate reported as a hang on three different names.
//!
//! Spec 416: a LIVE holder proves it is progressing by refreshing the file's modification time. Measured
//! 2026-11-13 in the gate for row 415: one holder took 120 s under the load of every other test binary, and the
//! five tests that were waiting failed at a flat 2 min budget without ever running. A wall clock cannot tell a
//! busy machine from a stuck holder; the file's age can, once the holder keeps it fresh.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: a test-support module; a failure to take the lock IS the test failing.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

/// How often a held lock proves it is progressing.
///
/// It must be a small fraction of the SHORTEST `give_up` any caller passes, because a waiter fails once the
/// file's age exceeds its own budget: an interval close to that budget would make a live holder look silent.
/// Measured 2026-11-13: with a 2 s interval, the waiter in `a_holder_that_keeps_its_heartbeat_is_waited_for_past_the_budget`
/// (budget 1 s) panicked before the second beat.
const HEARTBEAT: Duration = Duration::from_millis(200);

/// Holds the lock file and keeps it fresh until dropped.
pub struct HeavyGuard {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    beat: Option<std::thread::JoinHandle<()>>,
}

impl Drop for HeavyGuard {
    fn drop(&mut self) {
        // Spec 416 R1: stop the beat and wait for it BEFORE the file goes, so a guard never leaves a file its
        // own thread can still touch, and the next taker never sees a beat for a lock nobody holds.
        self.stop.store(true, Ordering::Relaxed);
        if let Some(beat) = self.beat.take() {
            let _ = beat.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The pid the lock file names, if it names one.
#[must_use]
pub fn holder_pid(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Refreshes `path`'s modification time while it still names this process, until `stop` is set.
fn beat(path: &Path, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    let path = path.to_path_buf();
    let me = std::process::id();
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(HEARTBEAT);
            // The file is read immediately before it is touched: the window in which another process could take
            // the lock is the two statements between, not the interval of the beat.
            if stop.load(Ordering::Relaxed) || holder_pid(&path) != Some(me) {
                return;
            }
            let Ok(f) = std::fs::OpenOptions::new().write(true).open(&path) else {
                return;
            };
            if f.set_modified(SystemTime::now()).is_err() {
                return;
            }
        }
    })
}

/// Takes the lock at `path`, waiting while its holder is alive and keeping it fresh.
///
/// # Panics
/// When the holder's last freshness is older than `give_up` — a dead process, a killed one, or one that stopped
/// progressing — the message names the holder's pid.
pub fn take(path: &Path, give_up: Duration) -> HeavyGuard {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    loop {
        match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
        {
            Ok(mut f) => {
                use std::io::Write;
                let _ = writeln!(f, "{}", std::process::id());
                drop(f);
                let stop = Arc::new(AtomicBool::new(false));
                let handle = beat(path, Arc::clone(&stop));
                return HeavyGuard {
                    path: path.to_path_buf(),
                    stop,
                    beat: Some(handle),
                };
            }
            Err(e) => {
                if let Some(holder) = holder_pid(path)
                    && !Path::new(&format!("/proc/{holder}")).exists()
                {
                    // The holder is gone: take the lock it left behind. A reaped lock is a normal event.
                    let _ = std::fs::remove_file(path);
                    continue;
                }
                let age = std::fs::metadata(path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok());
                let stale = age.is_some_and(|d| d.as_secs() > 900);
                if stale && holder_pid(path).is_none() {
                    let _ = std::fs::remove_file(path);
                    continue;
                }
                // Spec 416 R2: `give_up` is the budget for SILENCE, not the total wait. A holder that keeps its
                // heartbeat is waited for however long it needs, because it is demonstrably making progress.
                assert!(
                    age.is_none_or(|d| d < give_up),
                    "the holder of {} has not refreshed it for {:?} (budget {give_up:?}, holder pid {:?}, {e})",
                    path.display(),
                    age.unwrap_or_default(),
                    holder_pid(path)
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}
