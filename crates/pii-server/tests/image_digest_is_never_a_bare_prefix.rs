//! Spec 252: the socket runtime never journals a bare `sha256:` prefix as `image_digest`. The fixtures are the
//! measured reply of `GET /images/<ref>/json` on lnx (compat API: no `Digest` key, `RepoDigests` and `Id` only). The
//! socket is a fake runtime the test serves itself, under `_tmp`, so each case reads the digest the node would
//! journal and the refusal it would return.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use pii_core::{Budget, CapSet, Ctx};
use pii_server::node_kinds::nodes::coder_runtime::{
    ContainerRuntime, Launch, PodmanApi, Refusal, Request, Spawn,
};

const IMAGE: &str = "localhost/pii-agent:1";
const REPO_HEX: &str = "2684dee9d0fab97d03fac3f90e7fb774ede93297dbb52b5507142f337d06e7b8";
const ID_HEX: &str = "162ab4b23885f9344ee15534dc7673f11adeb4a00dd2f67f9f4a3e533e27950f";

/// Serves one inspect reply for the image, a clean container lifecycle after it, and records every request line.
/// It stops after ten seconds so a node that never calls it cannot hang the test.
fn serve(sock: &Path, inspect_body: String) -> Arc<Mutex<Vec<String>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let listener = UnixListener::bind(sock).unwrap();
    listener.set_nonblocking(true).unwrap();
    let log = Arc::clone(&seen);
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let Ok((mut conn, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            };
            conn.set_nonblocking(false).unwrap();
            let mut raw = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = conn.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    break;
                }
                raw.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&raw).into_owned();
                if let Some(end) = text.find("\r\n\r\n") {
                    let len = text[..end]
                        .lines()
                        .find_map(|l| l.strip_prefix("Content-Length: "))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if raw.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            let text = String::from_utf8_lossy(&raw).into_owned();
            let line = text.lines().next().unwrap_or_default().to_owned();
            log.lock().push(line.clone());
            let (status, body) = answer(&line, &inspect_body);
            let reply = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = conn.write_all(reply.as_bytes());
        }
    });
    seen
}

fn answer(line: &str, inspect_body: &str) -> (u16, String) {
    if line.starts_with("GET /images/") && line.ends_with("/json HTTP/1.1") {
        return (200, inspect_body.to_owned());
    }
    if line == "POST /containers/create HTTP/1.1" {
        return (201, String::from("{\"Id\":\"c0ffee\",\"Warnings\":[]}"));
    }
    if line.starts_with("POST /containers/c0ffee/start") {
        return (204, String::new());
    }
    if line.starts_with("POST /containers/c0ffee/wait") {
        return (200, String::from("{\"StatusCode\":0,\"Error\":null}"));
    }
    if line.starts_with("GET /containers/c0ffee/json") {
        return (
            200,
            String::from(
                "{\"Id\":\"c0ffee\",\"State\":{\"Running\":false,\"Pid\":4242,\"ExitCode\":0}}",
            ),
        );
    }
    if line.starts_with("DELETE /containers/c0ffee") {
        return (204, String::new());
    }
    (404, String::from("{\"message\":\"no such route\"}"))
}

/// What one run of the socket runtime did: the digest the node would journal, the refusal it returned (if any),
/// and the request lines the fake runtime saw.
struct Outcome {
    digest: Option<String>,
    refusal: Option<Refusal>,
    lines: Vec<String>,
}

/// Runs the socket runtime for [`IMAGE`] against a fake whose inspect reply is `inspect_body`.
fn run_with_reply(inspect_body: &str) -> Outcome {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "image-digest-").unwrap();
    let sock = dir.path().join("p.sock");
    let seen = serve(&sock, inspect_body.to_owned());
    let api = PodmanApi { socket: sock };
    let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
    let req = Request {
        image: IMAGE,
        workdir: "/tmp",
        network: "none",
        run_id: "r-252",
        skills_dir: None,
        agent_argv: &[],
        env: &[],
    };
    let mut digest = None;
    let mut spawned = |s: &Spawn| digest = Some(s.image_digest.clone());
    let launch = api
        .run(&ctx, &req, Some(Duration::from_secs(5)), &mut spawned)
        .unwrap();
    let refusal = match launch {
        Launch::Refused(r) => Some(r),
        Launch::Exited { .. } => None,
    };
    let lines = seen.lock().clone();
    Outcome {
        digest,
        refusal,
        lines,
    }
}

/// The repo digest the measured reply names for [`IMAGE`]: `RepoDigests` and `Id`, no `Digest` key.
fn measured_reply() -> String {
    format!(
        "{{\"Id\":\"sha256:{ID_HEX}\",\"RepoDigests\":[\"localhost/pii-agent@sha256:{REPO_HEX}\"]}}"
    )
}

/// A reply that names the image's repo digest, not its image id: the repo digest is the one journalled.
#[test]
fn repo_digest_is_used() {
    let out = run_with_reply(&measured_reply());
    assert_eq!(out.digest, Some(format!("sha256:{REPO_HEX}")));
    assert!(out.refusal.is_none());
}

/// No repo digest, so the image id is the digest: a full `sha256:` id, never an empty suffix.
#[test]
fn reply_without_digest_uses_the_image_id() {
    let body = format!("{{\"Id\":\"sha256:{ID_HEX}\",\"RepoDigests\":[]}}");
    let out = run_with_reply(&body);
    assert_eq!(out.digest, Some(format!("sha256:{ID_HEX}")));
    assert!(out.refusal.is_none());
}

/// Nothing to read a digest from: the node refuses the image, naming it, and creates no container.
#[test]
fn no_repo_digest_and_no_id_is_none() {
    let out = run_with_reply("{}");
    assert_eq!(
        out.digest, None,
        "no container may be journalled with a bare prefix"
    );
    match out.refusal {
        Some(Refusal::Image { image, reason }) => {
            assert_eq!(image, IMAGE);
            assert_eq!(reason, "image has no digest; pin it by digest");
        }
        other => panic!(
            "expected Refusal::Image, got {:?}",
            other.map(|r| r.reason())
        ),
    }
    assert!(
        !out.lines
            .iter()
            .any(|l| l.starts_with("POST /containers/create")),
        "a refused image must not create a container: {:?}",
        out.lines
    );
}

/// A `Digest` key that already spells the prefix is normalised: exactly one `sha256:` in the journalled value.
#[test]
fn a_prefixed_digest_is_not_doubled() {
    let body = format!("{{\"Digest\":\"sha256:{REPO_HEX}\"}}");
    let out = run_with_reply(&body);
    assert_eq!(out.digest, Some(format!("sha256:{REPO_HEX}")));
    assert!(out.refusal.is_none());
}
