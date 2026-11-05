//! The lock the heavy tests in `control_plane_is_not_behind_runs` take turns with.
//!
//! A FILE, not a Mutex: `clippy.toml` bans `std::sync::Mutex` here, and a file lock survives the test
//! binary's own process boundaries. Spec 394: it is released when its HOLDER is gone, not when a timestamp
//! says it is old. The pid the file records is the check; age is only the fallback for a pid `/proc` cannot
//! see. Measured 2026-11-05: the pid was written and never read, so a killed test left the file and every
//! later test waited fifteen minutes, which the gate reported as a hang on three different names.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: a test-support module; a failure to take the lock IS the test failing.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Holds the lock file; dropping it releases the lock.
pub struct HeavyGuard(PathBuf);

impl Drop for HeavyGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The pid the lock file names, if it names one.
#[must_use]
pub fn holder_pid(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Takes the lock at `path`, waiting at most `give_up` for a LIVE holder.
///
/// # Panics
/// When a live holder keeps the lock past `give_up`; the message names the holder's pid.
pub fn take(path: &Path, give_up: Duration) -> HeavyGuard {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let deadline = Instant::now() + give_up;
    loop {
        match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
        {
            Ok(mut f) => {
                use std::io::Write;
                let _ = writeln!(f, "{}", std::process::id());
                return HeavyGuard(path.to_path_buf());
            }
            Err(e) => {
                if let Some(holder) = holder_pid(path)
                    && !Path::new(&format!("/proc/{holder}")).exists()
                {
                    // The holder is gone: take the lock it left behind. A reaped lock is a normal event.
                    let _ = std::fs::remove_file(path);
                    continue;
                }
                let stale = std::fs::metadata(path)
                    .and_then(|m| m.modified())
                    .is_ok_and(|t| t.elapsed().is_ok_and(|d| d.as_secs() > 900));
                if stale && holder_pid(path).is_none() {
                    let _ = std::fs::remove_file(path);
                    continue;
                }
                assert!(
                    Instant::now() < deadline,
                    "another heavy test has held {} for {give_up:?} (holder pid {:?}, {e})",
                    path.display(),
                    holder_pid(path)
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}
