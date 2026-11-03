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

use std::cell::Cell;
use std::io::Write;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicUsize, Ordering};

/// How many slots THIS process holds, across every thread. The file is removed when this reaches zero, whichever
/// holder took it first: the first acquirer is not the last to drop, so an owner that drops early must not leave
/// the file behind for the next binary to wait out.
static HELD: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// How many slots THIS thread holds. A test that starts two daemons in sequence takes the slot twice, and the
    /// second take must not wait on the file its own thread holds. Another thread waits on the file, as it must.
    static MINE: Cell<usize> = const { Cell::new(0) };
}

/// Holds one count of [`HELD`]. The file itself is owned by the process, not by any one `Slot`.
pub(crate) struct Slot;

/// A private slot file, set by a test that asserts on the file itself. The default file is shared with every other
/// test run on this machine, so such an assertion would race them.
static PATH_OVERRIDE: parking_lot::Mutex<Option<std::path::PathBuf>> =
    parking_lot::Mutex::new(None);

/// Points this process's slot at `path` for the rest of the test binary.
pub(crate) fn use_private_slot(path: std::path::PathBuf) {
    *PATH_OVERRIDE.lock() = Some(path);
}

/// The one file every daemon test in this package contends for, unless a test made its own.
pub(crate) fn slot_path() -> std::path::PathBuf {
    PATH_OVERRIDE.lock().clone().unwrap_or_else(|| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp/daemon-slot.lock")
    })
}

/// True when the file's owner cannot be alive. The owner writes its pid into the file; a pid that has no `/proc`
/// entry is gone, so the file is a leak and is stolen at once. An empty file has no pid yet, or was left by a run
/// that died before writing one, so it gets a short grace period. 180 s is the backstop for a pid that was reused.
fn is_stale(path: &std::path::Path) -> bool {
    let Some(age) = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|m| std::time::SystemTime::now().duration_since(m).ok())
    else {
        return false;
    };
    let owner = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());
    let owner_gone = owner.map_or_else(
        || age > std::time::Duration::from_secs(5),
        |pid| !std::path::Path::new("/proc").join(pid.to_string()).exists(),
    );
    owner_gone || age > std::time::Duration::from_secs(180)
}

impl Slot {
    pub(crate) fn acquire() -> Self {
        if MINE.with(Cell::get) > 0 {
            MINE.with(|m| m.set(m.get() + 1));
            HELD.fetch_add(1, Ordering::SeqCst);
            return Self;
        }
        let path = slot_path();
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
                Ok(mut f) => {
                    let _ = write!(f, "{}", std::process::id());
                    MINE.with(|m| m.set(1));
                    HELD.fetch_add(1, Ordering::SeqCst);
                    return Self;
                }
                Err(_) if std::time::Instant::now() < give_up => {
                    // A killed test (or a killed agent run) leaves the file behind. Steal it once its owner is
                    // gone, or one interrupted run wedges the suite for good.
                    if is_stale(&path) {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(e) => {
                    panic!("could not take the daemon slot {}: {e}", path.display());
                }
            }
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        MINE.with(|m| m.set(m.get().saturating_sub(1)));
        if HELD.fetch_sub(1, Ordering::SeqCst) == 1 {
            let _ = std::fs::remove_file(slot_path());
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
