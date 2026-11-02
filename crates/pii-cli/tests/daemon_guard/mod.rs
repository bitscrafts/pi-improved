//! Spec 331 R1: the one owner of a `pii daemon` child a test spawns. `Child` has no `Drop` that kills, so a panic,
//! an `unwrap()` or an early `return` between the spawn and the explicit kill leaves the daemon running. The guard
//! kills and reaps its child on every exit path, and it is the only killer a test file may use.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods // reason: the guard kills and waits on the child it spawned, and nothing else
)]
#![allow(dead_code, clippy::redundant_pub_crate)]

use std::process::Child;

pub(crate) struct DaemonGuard(Option<Child>);

impl DaemonGuard {
    pub(crate) fn new(child: Child) -> Self {
        Self(Some(child))
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

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        self.kill_now();
    }
}
