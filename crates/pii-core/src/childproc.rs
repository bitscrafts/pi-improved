//! Child-process machinery (spec 13 split): `ChildProc`, `ProcOutput`,
//! the scrubbed-environment builder, and the process-execution internals
//! split out of `ctx.rs` to respect the 450-line file gate.

use std::path::Path;
use std::time::Instant;

use pii_abi::{CapKind, PiiError};

use crate::Ctx;

/// Test seam (supervisor TDD): an artificial post-SIGKILL delay in
/// [`ChildProc::kill`], simulating a slow-dying child. Test-only; defaults
/// to 0 (no delay) in production. Process-global so integration tests in
/// other crates can set it without env-var races.
static TEST_KILL_DELAY_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Test-only: sets the artificial `kill()` delay in milliseconds (0 =
/// off). Never called by production code.
pub fn set_test_kill_delay_ms(ms: u64) {
    TEST_KILL_DELAY_MS.store(ms, std::sync::atomic::Ordering::SeqCst);
}

/// Process gate (F3.8): `run`/`spawn_child` are the sanctioned
/// child-process sites.
#[derive(Debug)]
pub struct ProcCap<'a> {
    pub(crate) ctx: &'a Ctx,
}

/// A live child process with piped stdio (F1.5). Owned by the worker
/// supervisor; `kill` is the sanctioned way to stop it.
#[derive(Debug)]
pub struct ChildProc {
    inner: std::process::Child,
}

impl ChildProc {
    #[allow(clippy::disallowed_methods)] // reason: ProcCap::spawn_child is
    // the second sanctioned Command site (spec F1.5); the supervisor needs
    // piped stdio and the live handle for kill/try_wait.
    fn spawn(argv: &[String], cwd: &Path, env: &[(String, String)]) -> Result<Self, PiiError> {
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};

        let Some(argv0) = argv.first() else {
            return Err(PiiError::Invalid(String::from("spawn_child: empty argv")));
        };
        let mut cmd = Command::new(argv0);
        cmd.args(argv.get(1..).unwrap_or(&[]))
            .current_dir(cwd)
            // F2 (spec 13): scrubbed env, with the supervisor's extra pairs.
            .env_clear()
            .envs(scrubbed_env(&live_env(), env))
            // F3 (spec 13): own process group so a supervisor kill can reap
            // the whole tree, not just the direct child.
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let child = cmd
            .spawn()
            .map_err(|e| PiiError::Internal(format!("spawn {argv0}: {e}")))?;
        Ok(Self { inner: child })
    }

    /// The child's pid.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.inner.id()
    }

    /// Takes the child's stdin handle (the supervisor owns the writer).
    pub fn stdin(&mut self) -> Option<std::process::ChildStdin> {
        self.inner.stdin.take()
    }

    /// Takes the child's stdout handle (the reader thread demuxes frames).
    pub fn stdout(&mut self) -> Option<std::process::ChildStdout> {
        self.inner.stdout.take()
    }

    /// Takes the child's stderr handle (drained so a chatty worker cannot
    /// fill the pipe and deadlock).
    pub fn stderr(&mut self) -> Option<std::process::ChildStderr> {
        self.inner.stderr.take()
    }

    /// Non-blocking exit check.
    ///
    /// # Errors
    /// [`PiiError::Internal`] when the OS reports a wait failure.
    pub fn try_wait(&mut self) -> Result<Option<i32>, PiiError> {
        self.inner
            .try_wait()
            .map(|s| s.and_then(|st| st.code()))
            .map_err(|e| PiiError::Internal(format!("try_wait: {e}")))
    }

    /// Blocks up to `d` for exit; `Ok(None)` when still running.
    ///
    /// # Errors
    /// [`PiiError::Internal`] when the OS reports a wait failure.
    pub fn wait_timeout(&mut self, d: std::time::Duration) -> Result<Option<i32>, PiiError> {
        let deadline = Instant::now() + d;
        loop {
            match self.inner.try_wait() {
                Err(e) => return Err(PiiError::Internal(format!("wait: {e}"))),
                Ok(Some(st)) => return Ok(st.code()),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        return Ok(None);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }
    }

    /// Forcibly kills the child (kill -9 semantics via `kill()`).
    ///
    /// # Errors
    /// [`PiiError::Internal`] when the kill fails.
    pub fn kill(&mut self) -> Result<(), PiiError> {
        let delay = TEST_KILL_DELAY_MS.load(std::sync::atomic::Ordering::SeqCst);
        let _ = self.inner.kill();
        if delay > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay));
        }
        let _ = self.inner.wait();
        Ok(())
    }
}

/// Result of a completed child process.
#[derive(Debug, Clone)]
pub struct ProcOutput {
    /// Exit status code (124 when the timeout fired).
    pub exit_code: i32,
    /// Captured stdout.
    pub stdout: Vec<u8>,
    /// Captured stderr.
    pub stderr: Vec<u8>,
    /// True when the timeout fired and the child was killed.
    pub timed_out: bool,
    /// True when the run was cancelled (via the ctx cancel token) and the
    /// child was killed. Distinguishes a cancel from a timeout (F3, spec 13).
    pub cancelled: bool,
}

impl ProcCap<'_> {
    /// Runs `argv` in `cwd` with a wall-clock `timeout`, enforcing the
    /// `PROC` kind and the `proc_argv0` scope (F3.8).
    ///
    /// # Errors
    /// [`PiiError::Denied`] when `PROC` is missing or `argv[0]` is out of
    /// scope; [`PiiError::Timeout`] when the child exceeds the timeout;
    /// [`PiiError::Internal`] on spawn/wait failure.
    pub fn run(
        &self,
        argv: &[String],
        cwd: &Path,
        timeout: std::time::Duration,
    ) -> Result<ProcOutput, PiiError> {
        self.ctx.check()?;
        if !self.ctx.caps().kinds.contains(CapKind::PROC) {
            return Err(self.ctx.deny("proc"));
        }
        let Some(argv0) = argv.first() else {
            return Err(PiiError::Invalid(String::from("run: empty argv")));
        };
        if !self.ctx.caps().proc_argv0.permits(argv0.as_str()) {
            return Err(self.ctx.deny("proc"));
        }
        run_child(argv, cwd, timeout, self.ctx.cancel_token())
    }

    /// Spawns a long-lived child with piped stdio (F1.5): the second and
    /// last `std::process::Command` construction site in the workspace.
    /// Enforces `SPAWN` and the `proc_argv0` scope like [`ProcCap::run`].
    ///
    /// # Errors
    /// [`PiiError::Denied`] when `SPAWN` is missing or `argv[0]` is out of
    /// scope; [`PiiError::Internal`] on spawn failure.
    pub fn spawn_child(
        &self,
        argv: &[String],
        cwd: &Path,
        env: &[(String, String)],
    ) -> Result<ChildProc, PiiError> {
        self.ctx.check()?;
        if !self.ctx.caps().kinds.contains(CapKind::SPAWN) {
            return Err(self.ctx.deny("spawn"));
        }
        let Some(argv0) = argv.first() else {
            return Err(PiiError::Invalid(String::from("spawn_child: empty argv")));
        };
        if !self.ctx.caps().proc_argv0.permits(argv0.as_str()) {
            return Err(self.ctx.deny("spawn"));
        }
        ChildProc::spawn(argv, cwd, env)
    }
}

/// The ONLY place in the workspace that constructs a `std::process::Command`
/// (F3.8). `Command::output()` cannot enforce a timeout, so `spawn` is used
/// Environment allowlist for child processes (spec 13 F2): children do NOT
/// inherit the daemon's full environment — only these variables plus any
/// `PII_*` variable survive. Every `*_API_KEY` and other secret is dropped.
const CHILD_ENV_ALLOWLIST: &[&str] = &["PATH", "HOME", "LANG", "LC_ALL", "TERM", "PWD", "TMPDIR"];

/// Builds the scrubbed child environment from a parent env snapshot:
/// allowlisted variables plus every `PII_*` variable, then the caller's
/// extra `env` pairs (last write wins). Pure and table-testable (N12).
fn scrubbed_env(parent: &[(String, String)], extra: &[(String, String)]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut push = |k: String, v: String| {
        if !out.iter().any(|(ek, _)| ek == &k) {
            out.push((k, v));
        }
    };
    for k in CHILD_ENV_ALLOWLIST {
        for (pk, pv) in parent {
            if pk == k {
                push((*k).to_owned(), pv.clone());
            }
        }
    }
    for (pk, pv) in parent {
        if pk.starts_with("PII_") {
            push(pk.clone(), pv.clone());
        }
    }
    for (k, v) in extra {
        push(k.clone(), v.clone());
    }
    out
}

/// The live parent env snapshot (`std::env::vars`), for the two Command sites.
fn live_env() -> Vec<(String, String)> {
    std::env::vars().collect()
}

/// (with the F3.8-sanctioned per-item allow) to keep the child handle for
/// `try_wait`/`kill`. Captured streams are drained on joinable threads.
///
/// F3 (spec 13): the child runs in its own process group (`process_group(0)`),
/// so a timeout or cancel kills the WHOLE group (the direct child AND any
/// grandchildren it spawned) via `killpg`, not just the direct child. The
/// 10ms `try_wait` loop also observes the ctx cancel token, so a `run.cancel`
/// during a long `bash` is honored.
fn run_child(
    argv: &[String],
    cwd: &Path,
    timeout: std::time::Duration,
    cancel: &crate::cancel::CancelToken,
) -> Result<ProcOutput, PiiError> {
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let Some(argv0) = argv.first() else {
        return Err(PiiError::Invalid(String::from("run: empty argv")));
    };
    let mut cmd = Command::new(argv0);
    cmd.args(argv.get(1..).unwrap_or(&[]))
        .current_dir(cwd)
        // F2 (spec 13): children never inherit the daemon's environment.
        .env_clear()
        .envs(scrubbed_env(&live_env(), &[]))
        // F3 (spec 13): own process group so killpg can reap grandchildren.
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[allow(clippy::disallowed_methods)] // reason: ProcCap::run is the single
    // sanctioned Command site (spec F3.8); the timeout needs the child handle.
    let mut child = cmd
        .spawn()
        .map_err(|e| PiiError::Internal(format!("spawn {argv0}: {e}")))?;
    let drain = |pipe: std::process::ChildStdout| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut p = pipe;
            let _ = p.read_to_end(&mut buf);
            buf
        })
    };
    let out_t = child.stdout.take().map(drain);
    let err_t = child.stderr.take().map(|pipe| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut p = pipe;
            let _ = p.read_to_end(&mut buf);
            buf
        })
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Err(e) => return Err(PiiError::Internal(format!("wait {argv0}: {e}"))),
            Ok(Some(status)) => {
                return Ok(ProcOutput {
                    exit_code: status.code().unwrap_or(1),
                    stdout: out_t.map_or_else(Vec::new, |h| h.join().unwrap_or_default()),
                    stderr: err_t.map_or_else(Vec::new, |h| h.join().unwrap_or_default()),
                    timed_out: false,
                    cancelled: false,
                });
            }
            Ok(None) => {
                // F3: honor a run.cancel mid-flight — kill the whole group.
                if cancel.is_cancelled() {
                    let _ = kill_group(child.id());
                    let _ = child.wait();
                    return Err(PiiError::Cancelled);
                }
                if started.elapsed() > timeout {
                    // F3: kill the whole process group, not just the child.
                    let _ = kill_group(child.id());
                    let _ = child.wait();
                    return Err(PiiError::Timeout {
                        after_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                    });
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
}

/// Kills the process group whose leader is `pid` (SIGKILL). Safe wrapper
/// over `nix::sys::signal::killpg` — no `unsafe` in our code (F3, spec 13).
fn kill_group(pid: u32) -> Result<(), PiiError> {
    use nix::sys::signal::{Signal, killpg};
    use nix::unistd::Pid;
    killpg(
        Pid::from_raw(i32::try_from(pid).unwrap_or(i32::MAX)),
        Signal::SIGKILL,
    )
    .map_err(|e| PiiError::Internal(format!("killpg {pid}: {e}")))
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::{live_env, scrubbed_env};

    /// F2 (spec 13): the scrub drops secrets and unknowns, keeps the
    /// allowlist and every `PII_*` variable, and lets the caller's extra
    /// pairs win (last write).
    #[test]
    fn run_child_scrubs_environment() {
        let parent = vec![
            (String::from("PATH"), String::from("/bin")),
            (String::from("HOME"), String::from("/root")),
            (String::from("PII_TEST_MARKER"), String::from("yes")),
            (String::from("TEST_API_KEY"), String::from("supersecret")),
            (String::from("TEST_SCRUB_PROBE"), String::from("x")),
        ];
        let env = scrubbed_env(&parent, &[]);
        let has = |name: &str| env.iter().any(|(k, _)| k == name);
        let val = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
        assert_eq!(val("PATH").as_deref(), Some("/bin"), "allowlisted");
        assert_eq!(val("HOME").as_deref(), Some("/root"), "allowlisted");
        assert_eq!(
            val("PII_TEST_MARKER").as_deref(),
            Some("yes"),
            "PII_* survives"
        );
        assert!(!has("TEST_API_KEY"), "secret dropped");
        assert!(!has("TEST_SCRUB_PROBE"), "unknown dropped");

        // The caller's extra pairs are appended (supervisor's configured env).
        let empty: Vec<(String, String)> = Vec::new();
        let env = scrubbed_env(&empty, &[(String::from("WORKER_TAG"), String::from("a"))]);
        assert_eq!(env.len(), 1);
        assert_eq!(env[0], (String::from("WORKER_TAG"), String::from("a")));
        let _ = live_env();
    }

    /// F3 (spec 13): a timeout kills the whole process group — the direct
    /// child AND a grandchild it spawned — not just the direct child.
    #[test]
    fn run_child_kills_process_group_on_timeout() {
        use crate::cancel::CancelToken;
        use std::time::Duration;
        // `sh -c 'sleep 60 & wait'` spawns a grandchild `sleep 60` in the
        // same process group; a 200ms timeout must reap both.
        let argv = vec![
            String::from("/bin/sh"),
            String::from("-c"),
            String::from("sleep 60 & wait"),
        ];
        let cancel = CancelToken::new();
        let err = super::run_child(
            &argv,
            std::path::Path::new("."),
            Duration::from_millis(200),
            &cancel,
        )
        .expect_err("timeout must fire");
        assert!(
            matches!(err, pii_abi::PiiError::Timeout { .. }),
            "got {err:?}"
        );
    }

    /// F3 (spec 13): a cancelled ctx kills the child and returns Cancelled.
    #[test]
    fn run_child_returns_cancelled_when_ctx_cancelled() {
        use crate::cancel::CancelToken;
        use std::time::Duration;
        let argv = vec![
            String::from("/bin/sh"),
            String::from("-c"),
            String::from("sleep 60"),
        ];
        let cancel = CancelToken::new();
        // Cancel from another thread after a short delay.
        let c2 = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            c2.cancel();
        });
        let err = super::run_child(
            &argv,
            std::path::Path::new("."),
            Duration::from_secs(30),
            &cancel,
        )
        .expect_err("cancel must fire");
        assert!(matches!(err, pii_abi::PiiError::Cancelled), "got {err:?}");
    }
}
