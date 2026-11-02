//! Spec 331 R1: the one owner of a `pii daemon` child a test spawns. `Child` has no `Drop` that kills, so a panic,
//! an `unwrap()` or an early `return` between the spawn and the explicit kill leaves the daemon running. The guard
//! kills and reaps its child on every exit path, and it is the only killer a test file may use.
//!
//! It is also the one owner of the *daemon slot*. MEASURED 2026-11-02: row 337 gave the daemon durable stores, and
//! those stores are exclusive — a second daemon started with its own `--workspace`, `PII_RUNTIME_DIR` and
//! `PII_STATE_DIR` does not refuse, it HANGS before binding `control.sock` (log: `daemon.starting`, then nothing).
//! Tests that spawn daemons in parallel therefore failed with `connect …/control.sock: No such file or directory`,
//! which reads like a broken client and is really a daemon still waiting. The slot is taken BEFORE the spawn, so the
//! second test waits on the file instead of racing into a daemon that can never start.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods // reason: the guard kills and waits on the child it spawned, and nothing else
)]
#![allow(dead_code, clippy::redundant_pub_crate)]

use std::process::{Child, Command};
use std::sync::atomic::{AtomicUsize, Ordering};

/// How many slots THIS process holds. A test that starts two daemons in sequence takes the slot twice: only the
/// first acquisition waits on the file, and only the last release removes it. Without the counter, such a test
/// waits on itself for the whole budget and then panics.
static HELD: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct Slot {
    path: Option<std::path::PathBuf>,
    _file: Option<std::fs::File>,
}

impl Slot {
    pub(crate) fn acquire() -> Self {
        if HELD.fetch_add(1, Ordering::SeqCst) > 0 {
            return Self {
                path: None,
                _file: None,
            };
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp/daemon-slot.lock");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let give_up = std::time::Instant::now() + std::time::Duration::from_secs(600);
        loop {
            match std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
            {
                Ok(f) => {
                    return Self {
                        path: Some(path),
                        _file: Some(f),
                    };
                }
                Err(_) if std::time::Instant::now() < give_up => {
                    // A killed test (or a killed agent run) leaves the file behind. Steal it once it is old
                    // enough that no living test can own it, or one interrupted run wedges the suite for good.
                    let stale = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|m| std::time::SystemTime::now().duration_since(m).ok())
                        .is_some_and(|age| age > std::time::Duration::from_secs(180));
                    if stale {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(e) => {
                    HELD.fetch_sub(1, Ordering::SeqCst);
                    panic!("could not take the daemon slot {}: {e}", path.display());
                }
            }
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let left = HELD.fetch_sub(1, Ordering::SeqCst);
        if left == 1
            && let Some(p) = self.path.take()
        {
            let _ = std::fs::remove_file(p);
        }
    }
}

pub(crate) struct DaemonGuard(Option<Child>, Slot);

/// The only way a test starts a daemon: take the slot, then spawn, then guard. Taking the slot first is what makes
/// a second test wait instead of racing into a daemon that will hang on the exclusive store.
pub(crate) fn spawn(cmd: &mut Command) -> DaemonGuard {
    let slot = Slot::acquire();
    DaemonGuard(Some(cmd.spawn().expect("spawn pii daemon")), slot)
}

impl DaemonGuard {
    /// For a test that must hold the slot before it builds its command: `let slot = daemon_guard::slot();`.
    pub(crate) fn new(child: Child, slot: Slot) -> Self {
        Self(Some(child), slot)
    }

    pub(crate) fn pid(&self) -> u32 {
        self.0.as_ref().expect("the daemon was already killed").id()
    }

    /// The child, for a polling loop that calls `try_wait`.
    pub(crate) fn child_mut(&mut self) -> &mut Child {
        self.0.as_mut().expect("the daemon was already killed")
    }

    /// Hands over the output of a child that has already exited (its `try_wait` returned `Some`). Nothing can be left
    /// running: the process is gone, and `wait_with_output` reaps it.
    pub(crate) fn wait_with_output(mut self) -> std::io::Result<std::process::Output> {
        self.0
            .take()
            .expect("the daemon was already killed")
            .wait_with_output()
    }

    /// Kills and reaps the child now. Idempotent: a second call finds the slot empty and does nothing.
    pub(crate) fn kill_now(&mut self) {
        if let Some(mut child) = self.0.take() {
            // An already-exited child makes `kill` fail; that is the state we want, so the error is ignored.
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Take the slot without spawning, for a test whose command is built across several statements.
pub(crate) fn slot() -> Slot {
    Slot::acquire()
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        self.kill_now();
    }
}
