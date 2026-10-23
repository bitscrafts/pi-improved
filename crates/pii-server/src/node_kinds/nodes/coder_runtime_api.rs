//! The podman REST API runtime (spec 230): the container is created, started, waited on, read and removed over the
//! runtime socket. Split out of `coder_runtime.rs` by the 450-line gate; the trait and the child wait stay there.

use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use pii_abi::PiiError;
use pii_core::Ctx;
use serde_json::{Value, json};

use super::coder_container::{CONTAINER_WORKDIR, SKILL_DIRS};
use super::coder_runtime::{
    ContainerRuntime, Launch, OUTPUT_KEEP_BYTES, POLL, Refusal, Request, Spawn, TIMEOUT_EXIT_CODE,
};
use super::coder_runtime_util::{dechunk, elapsed_ms, message};

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
        timeout: Option<Duration>,
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
        let outcome = self.start_and_wait(ctx, req, timeout, &id, &digest, spawned);
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
    /// Starts `id`, reports its pid, waits for it, reads its exit code and its logs (spec 230 R2, spec 241).
    fn start_and_wait(
        &self,
        ctx: &Ctx,
        req: &Request<'_>,
        timeout: Option<Duration>,
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
        let mut state = match self.json_call(
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
        // Polled, not waited on: a blocking wait cannot be interrupted at the deadline (spec 241 R1, R2).
        let deadline = timeout.map(|limit| started + limit);
        let mut timed_out = false;
        while state
            .pointer("/State/Running")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            if deadline.is_some_and(|at| Instant::now() >= at) {
                // `run` force-removes the container after this returns, so the stop only has to make the exit prompt.
                let stop = format!("/containers/{id}/stop?t=2");
                let _ = self.call(&mut || Ok(()), "POST", &stop, None);
                timed_out = true;
                break;
            }
            ctx.check()?;
            std::thread::sleep(POLL);
            state = self.state(ctx, &inspect)?;
        }
        let code = if timed_out {
            TIMEOUT_EXIT_CODE
        } else {
            // The container had started, so a failed wait is a fault of the run, not a refusal to spawn.
            let wait = format!("/containers/{id}/wait");
            if let Err(failure) =
                self.json_call(&mut || ctx.check(), "POST", &wait, None, "wait", is_2xx)?
            {
                return Err(PiiError::Internal(format!(
                    "coder runtime: {}",
                    failure.reason
                )));
            }
            exit_code(&state)
        };
        // Read before `run` removes the container: the logs go with it (spec 241 R3).
        let (output, stderr, logs_unreadable) = self.logs(id);
        Ok(Launch::Exited {
            code,
            pid,
            dur_ms: elapsed_ms(started),
            output,
            stderr,
            logs_unreadable,
        })
    }

    /// The container's inspect document, read now. A failed read is a fault of the run: the container had started.
    fn state(&self, ctx: &Ctx, inspect: &str) -> Result<Value, PiiError> {
        self.json_call(&mut || ctx.check(), "GET", inspect, None, "inspect", is_2xx)?
            .map_err(|failure| PiiError::Internal(format!("coder runtime: {}", failure.reason)))
    }

    /// The container's stdout and stderr, each kept to [`OUTPUT_KEEP_BYTES`] (spec 241 R3). A reply that is not a
    /// success leaves both empty and names the container, so the launch still reports its exit code (R5).
    fn logs(&self, id: &str) -> (String, String, Option<String>) {
        let path = format!("/containers/{id}/logs?stdout=1&stderr=1");
        match self.call(&mut || Ok(()), "GET", &path, None) {
            Ok(Reply::Answered { status, body }) if is_2xx(status) => {
                let (out, err) = demux(&body);
                (out, err, None)
            }
            _ => (String::new(), String::new(), Some(id.to_owned())),
        }
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
            Reply::Answered { status, body } => {
                let text = String::from_utf8_lossy(&body);
                if ok(status) {
                    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
                } else {
                    Err(Failure {
                        status: Some(status),
                        reason: format!("{what}: {}", message(&text)),
                    })
                }
            }
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
    Answered { status: u16, body: Vec<u8> },
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
fn parse_reply(raw: &[u8]) -> Result<(u16, Vec<u8>), String> {
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
    Ok((status, body))
}

/// Splits a container's log stream into its stdout and stderr, each kept to [`OUTPUT_KEEP_BYTES`] (spec 241 R3). The
/// API frames every chunk with an eight-byte header: the stream (1 stdout, 2 stderr), three zero bytes, and the
/// payload's length as a big-endian `u32`.
fn demux(raw: &[u8]) -> (String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut rest = raw;
    while let Some((head, tail)) = rest.split_first_chunk::<8>() {
        let [kind, _, _, _, a, b, c, d] = *head;
        let len = usize::try_from(u32::from_be_bytes([a, b, c, d])).unwrap_or(usize::MAX);
        let (payload, next) = tail.split_at(len.min(tail.len()));
        let sink = if kind == 2 { &mut err } else { &mut out };
        let room = OUTPUT_KEEP_BYTES.saturating_sub(sink.len());
        sink.extend_from_slice(payload.get(..room.min(payload.len())).unwrap_or_default());
        rest = next;
    }
    (
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

/// The exit code the runtime recorded for a container's inspect document; `-1` when it names none.
fn exit_code(state: &Value) -> i32 {
    state
        .pointer("/State/ExitCode")
        .and_then(Value::as_i64)
        .and_then(|c| i32::try_from(c).ok())
        .unwrap_or(-1)
}
