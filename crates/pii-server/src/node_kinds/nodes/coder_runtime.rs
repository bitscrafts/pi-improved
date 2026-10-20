//! Container runtimes for `coder` nodes (spec 230).
//!
//! How a container is started, waited on and removed. [`PodmanCli`] execs `podman run` (spec 209); [`PodmanApi`]
//! speaks the podman REST API over the user's runtime socket. The node names one by the graph's `container_runtime`,
//! never by probing the host, and the journal names the one it used. The spawn-and-wait of a child process lives here
//! too ([`wait_child`]), shared by the CLI runtime and the host fallback. The node's journal and refusals stay in
//! [`super::coder`].

use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use pii_abi::PiiError;
use pii_core::Ctx;
use serde_json::{Value, json};

use super::coder_container::{self, CONTAINER_WORKDIR, ContainerRun, SKILL_DIRS};

/// How often a running child or container is checked for cancellation and the run's deadline.
const POLL: Duration = Duration::from_millis(50);

/// How long the image probe may run before the node refuses (spec 209).
const PROBE_TIMEOUT: Duration = Duration::from_mins(1);

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

/// A way to run a `coder` node's agent inside its image: the seam between the node and the container runtime
/// (spec 230 R1).
pub trait ContainerRuntime {
    /// The runtime's name, as the journal carries it in `runtime`.
    fn name(&self) -> &'static str;

    /// Runs the container to its exit. `spawned` is called once the container has a pid and before the wait, so the
    /// node journals `coder.spawned` for a run it may cancel.
    ///
    /// # Errors
    /// [`PiiError`] when the run is cancelled, its deadline passes, or the node lacks the capability to spawn.
    fn run(
        &self,
        ctx: &Ctx,
        req: &Request<'_>,
        spawned: &mut dyn FnMut(&Spawn),
    ) -> Result<Launch, PiiError>;
}

/// The spec 209 runtime: the image is probed, then `podman run` is spawned as a child process.
pub struct PodmanCli {
    /// The podman binary the run execs.
    pub podman: PathBuf,
    /// The daemon's PATH and HOME: the only environment podman itself is given.
    pub env: Vec<(String, String)>,
    /// The runs directory the env file is written under; `None` is a refusal once the image has been probed.
    pub runs_dir: Option<PathBuf>,
}

impl ContainerRuntime for PodmanCli {
    fn name(&self) -> &'static str {
        "cli"
    }

    fn run(
        &self,
        ctx: &Ctx,
        req: &Request<'_>,
        spawned: &mut dyn FnMut(&Spawn),
    ) -> Result<Launch, PiiError> {
        let podman = self.podman.display().to_string();
        let workdir = PathBuf::from(req.workdir);
        let probe = [
            podman.clone(),
            "image".to_owned(),
            "inspect".to_owned(),
            "--format".to_owned(),
            "{{.Digest}}".to_owned(),
            req.image.to_owned(),
        ];
        let digest = match ctx.proc()?.run(&probe, &workdir, PROBE_TIMEOUT) {
            Ok(out) if out.exit_code == 0 => String::from_utf8_lossy(&out.stdout).trim().to_owned(),
            // Spec 231 R2: podman said the image is absent, or said nothing at all (spec 228). Any other words are the
            // runtime failing to answer, not an absent image.
            Ok(out) if probe_says_absent(&out.stderr) => {
                return Ok(Launch::Refused(Refusal::Image {
                    image: req.image.to_owned(),
                    reason: probe_reason(&out.stderr),
                }));
            }
            Ok(out) => {
                return Ok(Launch::Refused(Refusal::Runtime {
                    socket: podman,
                    status: None,
                    reason: probe_reason(&out.stderr),
                }));
            }
            Err(e) => return Ok(Launch::Refused(Refusal::Other(format!("{e}")))),
        };
        if digest.is_empty() {
            return Ok(Launch::Refused(Refusal::Image {
                image: req.image.to_owned(),
                reason: "image has no digest; pin it by digest".to_owned(),
            }));
        }
        let env_file = match env_file_path(self.runs_dir.as_deref()) {
            Ok(path) => path,
            Err(reason) => return Ok(Launch::Refused(Refusal::Other(reason))),
        };
        if let Err(e) = coder_container::write_env_file(&env_file, req.env) {
            return Ok(Launch::Refused(Refusal::Other(format!("env file: {e}"))));
        }
        let env_file_str = env_file.display().to_string();
        let run = ContainerRun {
            podman: &podman,
            image: req.image,
            workdir: req.workdir,
            env_file: &env_file_str,
            network: req.network,
            run_id: req.run_id,
            skills_dir: req.skills_dir,
        };
        let run_argv = coder_container::run_argv(&run, req.agent_argv);
        let result = wait_child(ctx, &run_argv, &workdir, &self.env, &mut |pid| {
            spawned(&Spawn {
                runtime: "cli",
                image: req.image.to_owned(),
                image_digest: digest.clone(),
                container: String::new(),
                pid,
                argv: run_argv.clone(),
                env_names: env_names(&self.env),
            });
        });
        // The env file holds the secrets: it goes on every path, once podman has had its read.
        let _ = std::fs::remove_file(&env_file);
        result
    }
}

/// The podman REST API runtime (spec 230).
///
/// The container is created, started, waited on, inspected for its exit code and removed. No `podman` process runs,
/// and no image is probed or pulled.
pub struct PodmanApi {
    /// The socket the API listens on: `XDG_RUNTIME_DIR/podman/podman.sock` in production.
    pub socket: PathBuf,
}

impl ContainerRuntime for PodmanApi {
    fn name(&self) -> &'static str {
        "socket"
    }

    fn run(
        &self,
        ctx: &Ctx,
        req: &Request<'_>,
        spawned: &mut dyn FnMut(&Spawn),
    ) -> Result<Launch, PiiError> {
        // Spec 232 R4: the digest the runtime will actually run, read before anything starts. An
        // inspect failure refuses exactly like the CLI path's probe (spec 228 R2): podman's words,
        // never a silent empty digest.
        let digest = match self.json_call(
            &mut || ctx.check(),
            "GET",
            &format!("/images/{}/json", req.image),
            None,
            "image inspect",
            is_2xx,
        )? {
            Ok(value) => {
                String::from("sha256:")
                    + value
                        .pointer("/Digest")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim_start_matches("sha256:")
            }
            Err(failure) => return Ok(self.refused(failure)),
        };
        let created = self.json_call(
            &mut || ctx.check(),
            "POST",
            "/containers/create",
            Some(&create_body(req)),
            "create",
            is_2xx,
        )?;
        let value = match created {
            Ok(value) => value,
            Err(failure) => return Ok(self.refused(failure)),
        };
        let Some(id) = value.get("Id").and_then(Value::as_str).map(str::to_owned) else {
            let failure = Failure {
                status: None,
                reason: String::from("create: the reply names no container id"),
            };
            return Ok(self.refused(failure));
        };
        let outcome = self.start_and_wait(ctx, req, &id, &digest, spawned);
        // Removed on every path, forcibly: a cancelled run's container is still running and must not be left.
        let _ = self.call(
            &mut || Ok(()),
            "DELETE",
            &format!("/containers/{id}?force=true"),
            None,
        );
        outcome
    }
}

impl PodmanApi {
    /// Starts `id`, reports its pid, waits for it and reads its exit code (spec 230 R2).
    fn start_and_wait(
        &self,
        ctx: &Ctx,
        req: &Request<'_>,
        id: &str,
        digest: &str,
        spawned: &mut dyn FnMut(&Spawn),
    ) -> Result<Launch, PiiError> {
        let started = Instant::now();
        let start = format!("/containers/{id}/start");
        if let Err(failure) = self.json_call(
            &mut || ctx.check(),
            "POST",
            &start,
            None,
            "start",
            started_ok,
        )? {
            return Ok(self.refused(failure));
        }
        let inspect = format!("/containers/{id}/json");
        let state = match self.json_call(
            &mut || ctx.check(),
            "GET",
            &inspect,
            None,
            "inspect",
            is_2xx,
        )? {
            Ok(value) => value,
            Err(failure) => return Ok(self.refused(failure)),
        };
        let pid = state
            .pointer("/State/Pid")
            .and_then(Value::as_i64)
            .and_then(|p| u32::try_from(p).ok())
            .unwrap_or(0);
        spawned(&Spawn {
            runtime: "socket",
            image: req.image.to_owned(),
            image_digest: digest.to_owned(),
            container: id.to_owned(),
            pid,
            argv: req.agent_argv.to_vec(),
            env_names: req.env.iter().map(|(name, _)| name.clone()).collect(),
        });
        let wait = format!("/containers/{id}/wait");
        if let Err(failure) =
            self.json_call(&mut || ctx.check(), "POST", &wait, None, "wait", is_2xx)?
        {
            // The container had started, so this is a fault of the run, not a refusal to spawn.
            return Err(PiiError::Internal(format!(
                "coder runtime: {}",
                failure.reason
            )));
        }
        let exited = match self.json_call(
            &mut || ctx.check(),
            "GET",
            &inspect,
            None,
            "inspect",
            is_2xx,
        )? {
            Ok(value) => value,
            Err(failure) => {
                return Err(PiiError::Internal(format!(
                    "coder runtime: {}",
                    failure.reason
                )));
            }
        };
        let code = exited
            .pointer("/State/ExitCode")
            .and_then(Value::as_i64)
            .and_then(|c| i32::try_from(c).ok())
            .unwrap_or(-1);
        Ok(Launch::Exited {
            code,
            pid,
            dur_ms: elapsed_ms(started),
        })
    }

    /// A refusal naming this runtime's socket.
    fn refused(&self, failure: Failure) -> Launch {
        Launch::Refused(Refusal::Runtime {
            socket: self.socket.display().to_string(),
            status: failure.status,
            reason: failure.reason,
        })
    }

    /// One API call whose reply must be one of `ok`'s statuses; its JSON body on success, or the failure it became.
    /// The outer error is cancellation, which `tick` raises while the reply is awaited.
    fn json_call(
        &self,
        tick: &mut dyn FnMut() -> Result<(), PiiError>,
        method: &str,
        path: &str,
        body: Option<&Value>,
        what: &str,
        ok: fn(u16) -> bool,
    ) -> Result<Result<Value, Failure>, PiiError> {
        Ok(match self.call(tick, method, path, body)? {
            Reply::Failed(e) => Err(Failure {
                status: None,
                reason: format!("{what}: {e}"),
            }),
            Reply::Answered { status, body } if ok(status) => {
                Ok(serde_json::from_str(&body).unwrap_or(Value::Null))
            }
            Reply::Answered { status, body } => Err(Failure {
                status: Some(status),
                reason: format!("{what}: {}", message(&body)),
            }),
        })
    }

    /// One HTTP/1.1 exchange over the socket, one connection per call. `tick` runs whenever the reply is still
    /// pending, so a cancelled run stops waiting on a container.
    fn call(
        &self,
        tick: &mut dyn FnMut() -> Result<(), PiiError>,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Reply, PiiError> {
        let socket = self.socket.display().to_string();
        let mut conn = match UnixStream::connect(&self.socket) {
            Ok(conn) => conn,
            Err(e) => return Ok(Reply::Failed(format!("connect {socket}: {e}"))),
        };
        if let Err(e) = conn.set_read_timeout(Some(POLL)) {
            return Ok(Reply::Failed(format!("socket {socket}: {e}")));
        }
        let payload = body.map(Value::to_string).unwrap_or_default();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: podman\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
            payload.len()
        );
        if let Err(e) = conn.write_all(request.as_bytes()) {
            return Ok(Reply::Failed(format!("write {socket}: {e}")));
        }
        let mut raw = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            match conn.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => raw.extend_from_slice(buf.get(..n).unwrap_or_default()),
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                    ) =>
                {
                    tick()?;
                }
                Err(e) => return Ok(Reply::Failed(format!("read {socket}: {e}"))),
            }
        }
        Ok(match parse_reply(&raw) {
            Ok((status, body)) => Reply::Answered { status, body },
            Err(e) => Reply::Failed(e),
        })
    }
}

/// One reply from the runtime: its status and body, or the failure to obtain one.
enum Reply {
    Answered { status: u16, body: String },
    Failed(String),
}

/// A call that did not succeed: the HTTP status when the runtime answered, and the reason in its own words.
struct Failure {
    status: Option<u16>,
    reason: String,
}

/// The body of `POST /containers/create`: the image, the agent's argv, the workspace and skill mounts, the network,
/// the labels, and the env-pass values.
fn create_body(req: &Request<'_>) -> Value {
    let mut binds = vec![format!("{}:{CONTAINER_WORKDIR}", req.workdir)];
    if let Some(skills) = req.skills_dir {
        binds.extend(
            SKILL_DIRS
                .iter()
                .map(|dir| format!("{skills}:{CONTAINER_WORKDIR}/{dir}:ro")),
        );
    }
    json!({
        "Image": req.image,
        "Cmd": req.agent_argv,
        "WorkingDir": CONTAINER_WORKDIR,
        "Env": req.env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>(),
        "Labels": { "io.pii": "1", "pii.run": req.run_id },
        "HostConfig": { "Binds": binds, "NetworkMode": req.network },
    })
}

fn is_2xx(status: u16) -> bool {
    (200..300).contains(&status)
}

/// `start` answers 204 on success and 304 when the container was already running.
fn started_ok(status: u16) -> bool {
    is_2xx(status) || status == 304
}

/// Splits an HTTP/1.1 reply into its status and body, decoding a chunked body.
fn parse_reply(raw: &[u8]) -> Result<(u16, String), String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("the reply has no header end")?;
    let head = String::from_utf8_lossy(raw.get(..split).unwrap_or_default()).to_ascii_lowercase();
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| format!("the reply has no status line: {head:?}"))?;
    let body = raw.get(split + 4..).unwrap_or_default();
    let body = if head.contains("transfer-encoding: chunked") {
        dechunk(body)?
    } else {
        body.to_vec()
    };
    Ok((status, String::from_utf8_lossy(&body).into_owned()))
}

/// The payload of a chunked body: each chunk's size line, its bytes, and a CRLF, up to the zero-size chunk.
fn dechunk(mut rest: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let eol = rest
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("a chunk size line is unterminated")?;
        let size_line = String::from_utf8_lossy(rest.get(..eol).unwrap_or_default()).into_owned();
        let size = size_line
            .split(';')
            .next()
            .and_then(|s| usize::from_str_radix(s.trim(), 16).ok())
            .ok_or_else(|| format!("a chunk size is not hex: {size_line:?}"))?;
        rest = rest.get(eol + 2..).unwrap_or_default();
        if size == 0 {
            return Ok(out);
        }
        let chunk = rest.get(..size).ok_or("a chunk is truncated")?;
        out.extend_from_slice(chunk);
        rest = rest.get(size + 2..).unwrap_or_default();
    }
}

/// The runtime's own message from an error body, or the body itself, collapsed to one line and cut to 240 chars.
fn message(body: &str) -> String {
    let text = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| body.split_whitespace().collect::<Vec<_>>().join(" "));
    text.chars().take(240).collect()
}

/// Whether a failed probe's stderr says the image is absent, or says nothing (spec 228 R2, spec 231 R2). Podman's
/// absence words are `no such image` and `image not known`.
fn probe_says_absent(stderr: &[u8]) -> bool {
    let said = String::from_utf8_lossy(stderr).to_lowercase();
    said.trim().is_empty() || said.contains("no such image") || said.contains("image not known")
}

/// Podman's own stderr from a failed probe, collapsed to one line and cut to its last 240 characters (spec 228 R2).
/// The absence clause stands alone only when podman said nothing at all.
fn probe_reason(stderr: &[u8]) -> String {
    let collapsed = String::from_utf8_lossy(stderr)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let tail: String = collapsed.chars().rev().take(240).collect::<String>();
    let tail = tail.chars().rev().collect::<String>();
    if tail.is_empty() {
        "image inspect failed with no stderr".to_owned()
    } else {
        tail
    }
}

/// A fresh env-file path under the runs dir, unique within this process (spec 227 R6).
///
/// The file is created later with `create_new`, so an existing path is an error, never overwritten. No runs dir is a
/// refusal: the secrets have nowhere private to go.
///
/// # Errors
/// The reason string when no runs dir is configured or it cannot be created.
pub fn env_file_path(runs_dir: Option<&Path>) -> Result<PathBuf, String> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let Some(runs) = runs_dir else {
        return Err(String::from("env file: no runs directory is configured"));
    };
    std::fs::create_dir_all(runs).map_err(|e| format!("env file: {e}"))?;
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    Ok(runs.join(format!("pii-coder-{}-{seq}.env", std::process::id())))
}

/// The names of an environment, never its values.
fn env_names(env: &[(String, String)]) -> Vec<String> {
    env.iter().map(|(name, _)| name.clone()).collect()
}

/// Milliseconds since `started`, saturating: a duration never wraps negative.
#[must_use]
pub fn elapsed_ms(started: Instant) -> i64 {
    i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX)
}

/// Spawns `argv` with exactly `env` in `workdir`, and waits for it.
///
/// `spawned` is called with the pid before the wait. The output is drained so a chatty child cannot block. A cancelled
/// or timed-out run kills the child and returns the error.
///
/// # Errors
/// [`PiiError`] when the node lacks the spawn capability, the spawn fails, or the run's context ends first.
pub fn wait_child(
    ctx: &Ctx,
    argv: &[String],
    workdir: &Path,
    env: &[(String, String)],
    spawned: &mut dyn FnMut(u32),
) -> Result<Launch, PiiError> {
    let started = Instant::now();
    let mut child = ctx.proc()?.spawn_child_exact(argv, workdir, env)?;
    let pid = child.pid();
    spawned(pid);
    // Nothing is written to the child's stdin; its output is drained so a chatty child cannot block.
    drop(child.stdin());
    if let Some(mut out) = child.stdout() {
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut out, &mut std::io::sink());
        });
    }
    if let Some(mut err) = child.stderr() {
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut err, &mut std::io::sink());
        });
    }
    let code = loop {
        if let Some(code) = child.wait_timeout(POLL)? {
            break code;
        }
        if let Err(e) = ctx.check() {
            let _ = child.kill();
            return Err(e);
        }
    };
    Ok(Launch::Exited {
        code,
        pid,
        dur_ms: elapsed_ms(started),
    })
}
