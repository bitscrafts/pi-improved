//! Container runtimes for `coder` nodes (spec 230).
//!
//! How a container is started, waited on and removed. [`PodmanCli`] execs `podman run` (spec 209); [`PodmanApi`]
//! speaks the podman REST API over the user's runtime socket. The node names one by the graph's `container_runtime`,
//! never by probing the host, and the journal names the one it used. The spawn-and-wait of a child process lives here
//! too ([`wait_child`]), shared by the CLI runtime and the host fallback. The node's journal and refusals stay in
//! [`super::coder`].

use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pii_abi::PiiError;
use pii_core::Ctx;

use super::coder_output_keep::OutputKeep;

pub use super::coder_runtime_api::PodmanApi;
pub use super::coder_runtime_cli::PodmanCli;
/// The elapsed-ms helper the coder node reports durations with (kept at this path for callers).
pub use super::coder_runtime_util::elapsed_ms;

/// How often a running child or container is checked for cancellation and the run's deadline.
pub(super) const POLL: Duration = Duration::from_millis(50);

/// How long the image probe may run before the node refuses (spec 209).
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_mins(1);

/// What a launch came to: the exit code, pid and duration of a run that ended, or the refusal that stopped it
/// before anything ran.
pub enum Launch {
    /// The run ended with `code`; `pid` is the process the runtime reported, `0` when it was unknown.
    Exited {
        /// The exit code (the container's `ExitCode`, or the child's).
        code: i32,
        /// The pid the runtime reported before the wait.
        pid: u32,
        /// Milliseconds from the start of the launch to the exit.
        dur_ms: i64,
        /// The agent's captured stdout (bounded). Empty when the runtime cannot return it (a container).
        output: String,
        /// The agent's captured stderr (bounded): the reason it exited, when it said one. Empty for a container.
        stderr: String,
        /// The container whose logs the runtime could not read (spec 241 R5); `None` when they were read.
        logs_unreadable: Option<String>,
    },
    /// The launch was refused before the run started.
    Refused(Refusal),
}

/// Why a launch was refused before its container or agent ran. Each variant carries the journal event it names.
#[derive(Debug)]
pub enum Refusal {
    /// The image is absent or has no digest (spec 209): journalled as `coder.image.missing`.
    Image {
        /// The image the graph names.
        image: String,
        /// Podman's own words (spec 228 R2), or the node's reason for an undigested image.
        reason: String,
    },
    /// The runtime could not be reached, or answered a call with an error (spec 230 R3): journalled as
    /// `coder.runtime.unavailable`, never as an absent image.
    Runtime {
        /// The socket the node called.
        socket: String,
        /// The HTTP status of the runtime's reply; `None` when no reply came.
        status: Option<u16>,
        /// What failed, in the runtime's own words where it gave some.
        reason: String,
    },
    /// A refusal with no event of its own: a mount, an env file.
    Other(String),
}

impl Refusal {
    /// The reason the `worker.start.refused` event and the `err` port carry.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::Image { reason, .. } | Self::Runtime { reason, .. } | Self::Other(reason) => {
                reason.clone()
            }
        }
    }
}

/// What a container run is given: the image, the mounts, the network, the agent's argv and the env-pass values.
pub struct Request<'a> {
    /// The prebuilt image, by the tag or digest the graph names.
    pub image: &'a str,
    /// The host workspace, mounted at [`CONTAINER_WORKDIR`].
    pub workdir: &'a str,
    /// `none` unless the graph declares a network grant.
    pub network: &'a str,
    /// The run's id, labelled on the container so a killed run's container can be found.
    pub run_id: &'a str,
    /// The run's materialised skills (spec 225 R3), bound read-only over [`SKILL_DIRS`]; `None` when none are pinned.
    pub skills_dir: Option<&'a str>,
    /// The agent's argv, run inside the image after it.
    pub agent_argv: &'a [String],
    /// The `env_pass` values. The CLI carries them in an env file, the API in the create body; never on an argv.
    pub env: &'a [(String, String)],
}

/// A container or child that has a pid, reported before the node waits on it: what `coder.spawned` journals.
pub struct Spawn {
    /// The runtime's name: `cli`, `socket` or `host`.
    pub runtime: &'static str,
    /// The image the run uses; empty for a host run.
    pub image: String,
    /// The image's digest as the runtime probed it; empty when the runtime does not probe.
    pub image_digest: String,
    /// The container's id; empty for a host run.
    pub container: String,
    /// The pid the runtime reported; `0` when it was not known at the spawn.
    pub pid: u32,
    /// The argv the node handed over: the `podman run` argv, the API's agent argv, or the host argv.
    pub argv: Vec<String>,
    /// The names of the environment given to the run; never a value.
    pub env_names: Vec<String>,
}

/// A container the runtime holds, with its labels (spec 242 R1).
#[derive(Clone)]
pub struct Listed {
    /// The container's id.
    pub id: String,
    /// Its labels as `(key, value)` pairs.
    pub labels: Vec<(String, String)>,
}

/// A way to run a `coder` node's agent inside its image: the seam between the node and the container runtime
/// (spec 230 R1).
pub trait ContainerRuntime {
    /// The runtime's name, as the journal carries it in `runtime`.
    fn name(&self) -> &'static str;

    /// Runs the container to its exit. `spawned` is called once the container has a pid and before the wait, so the
    /// node journals `coder.spawned` for a run it may cancel. `timeout` bounds the run: when it passes, the container
    /// is stopped and the launch ends with [`TIMEOUT_EXIT_CODE`] (spec 241 R1). `None` waits without a bound.
    /// The output comes back after the exit and before removal: `GET /containers/{id}/logs?stdout=1&stderr=1` over
    /// the API, the attached `podman run` streams on the CLI (spec 241 R3).
    ///
    /// # Errors
    /// [`PiiError`] when the run is cancelled, or the node lacks the capability to spawn.
    fn run(
        &self,
        ctx: &Ctx,
        req: &Request<'_>,
        timeout: Option<Duration>,
        spawned: &mut dyn FnMut(&Spawn),
    ) -> Result<Launch, PiiError>;

    /// Every container the runtime holds, stopped ones included, with its labels (spec 242 R1). Boot recovery reads
    /// them to find the ones a dead daemon's runs left behind.
    ///
    /// # Errors
    /// [`PiiError`] when the runtime cannot be asked.
    fn list_all(&self, ctx: &Ctx) -> Result<Vec<Listed>, PiiError>;

    /// Force-removes container `id` (spec 242 R2). `Ok(false)` when it was already gone.
    ///
    /// # Errors
    /// [`PiiError`] when the runtime refuses the removal or cannot be asked.
    fn force_remove(&self, ctx: &Ctx, id: &str) -> Result<bool, PiiError>;
}

/// The exit code a launch reports when the node's own timeout killed the agent.
///
/// The `timeout(1)` convention, so the journal carries a real `coder.exited` with a non-zero rc instead of a run that
/// never ends.
pub const TIMEOUT_EXIT_CODE: i32 = 124;

/// # Errors
/// Returns [`PiiError`] when the child cannot be spawned or the wait fails.
/// [`PiiError`] when the node lacks the spawn capability, the spawn fails, or the run's context ends first.
///
/// `timeout` bounds the whole run of the child: when it passes, the child's process group is killed and the launch
/// ends with [`TIMEOUT_EXIT_CODE`]. `None` waits without a bound.
pub fn wait_child(
    ctx: &Ctx,
    argv: &[String],
    workdir: &Path,
    env: &[(String, String)],
    timeout: Option<Duration>,
    spawned: &mut dyn FnMut(u32),
) -> Result<Launch, PiiError> {
    let started = Instant::now();
    let mut child = ctx.proc()?.spawn_child_exact(argv, workdir, env)?;
    let pid = child.pid();
    spawned(pid);
    // Nothing is written to the child's stdin. Both streams are kept, bounded, and captured BEFORE the wait so
    // the pipes never fill, then finished AFTER the child exits. stderr is what says WHY an agent exited, so it
    // is journaled too (a fast failure that wrote only to stderr was invisible before).
    let capture = StdoutCapture::start(child.stdout());
    let err_capture = StdoutCapture::start(child.stderr());
    drop(child.stdin());
    let code = loop {
        if let Some(code) = child.wait_timeout(POLL)? {
            break code;
        }
        if let Err(e) = ctx.check() {
            let _ = child.kill();
            return Err(e);
        }
        // A hung agent cannot be cancelled (a running node refuses `run cancel`) and nothing else bounds it, so the
        // node bounds itself: kill the whole group and report the timeout as an exit, not as a run that never ends.
        if timeout.is_some_and(|limit| started.elapsed() >= limit) {
            let _ = child.kill();
            break TIMEOUT_EXIT_CODE;
        }
    };
    // Anything the agent left running in its process group dies with the node (the stand-in matrix measured a
    // `sleep 100000` surviving as pid 1's child). Killed BEFORE the capture is finished, so a grandchild that holds
    // the pipe open cannot stall the capture for its whole wait bound.
    child.reap_group();
    let output = capture.finish();
    let stderr = err_capture.finish();
    Ok(Launch::Exited {
        code,
        pid,
        dur_ms: elapsed_ms(started),
        output,
        stderr,
        logs_unreadable: None,
    })
}

/// The most agent output one run keeps. Enough for a `--mode json` event stream's routing fields; the rest
/// is drained and discarded so a long run cannot exhaust memory.
pub(super) const OUTPUT_KEEP_BYTES: usize = 256 * 1024;

/// A running stdout reader: started before the child is waited on, so the pipe never fills, and joined after.
pub struct StdoutCapture {
    buf: Arc<parking_lot::Mutex<OutputKeep>>,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl StdoutCapture {
    /// Starts draining `out` in a background thread, keeping at most [`OUTPUT_KEEP_BYTES`].
    pub fn start(out: Option<impl Read + Send + 'static>) -> Self {
        let buf = Arc::new(parking_lot::Mutex::new(OutputKeep::default()));
        let reader = out.map(|mut out| {
            let reader_buf = Arc::clone(&buf);
            std::thread::spawn(move || capture_bounded(&mut out, &reader_buf))
        });
        Self { buf, reader }
    }

    /// Waits for the reader to end (the child has exited, so its pipe closed), bounded to five seconds, then
    /// returns the captured text. A reader still open past the bound yields what was read so far.
    pub fn finish(self) -> String {
        if let Some(reader) = self.reader {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !reader.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        self.buf.lock().finish()
    }
}

/// Reads a child's stdout to its end and returns it: start the capture and finish it in one call.
pub fn drain_stdout(out: impl Read + Send + 'static) -> String {
    StdoutCapture::start(Some(out)).finish()
}

/// Feeds `src` into `buf` until it ends, keeping the head and the tail of what it carried (spec 263).
fn capture_bounded(src: &mut dyn Read, buf: &parking_lot::Mutex<OutputKeep>) {
    let mut chunk = [0u8; 8192];
    loop {
        match src.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if let Some(read) = chunk.get(..n) {
                    buf.lock().push(read);
                }
            }
        }
    }
}
