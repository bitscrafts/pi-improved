//! Spec 263: a run's captured output keeps its head AND its tail within the one bound. The head carries claude's
//! `system/init` line (session and model); the tail carries its `result` line (cost, turns, error). Both capture paths
//! are driven here: the process reader through [`drain_stdout`], and the container path through a fake libpod socket.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::io::{Cursor, Read, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use pii_core::{Budget, CapSet, Ctx};
use pii_server::node_kinds::nodes::agent_claude::{RunFacts, run_facts};
use pii_server::node_kinds::nodes::coder_runtime::{
    ContainerRuntime, Launch, PodmanApi, Request, drain_stdout,
};

/// The most output one stream keeps (spec 209's `OUTPUT_KEEP_BYTES`, private to the runtime).
const KEEP: usize = 256 * 1024;

/// The marker's type, which every line-based reader skips.
const MARK: &str = "{\"type\":\"pii_output_gap\"";

fn init_line() -> String {
    "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"sess-263\",\"model\":\"claude-haiku-5-5\"}\n".to_owned()
}

fn result_line() -> String {
    "{\"type\":\"result\",\"subtype\":\"error\",\"is_error\":true,\"api_error_status\":401,\"result\":\"LAST\",\"num_turns\":9,\"total_cost_usd\":0.25,\"session_id\":\"sess-263\"}\n".to_owned()
}

/// A claude-shaped stream of about `size` bytes: an init line, filler lines, then `last`.
fn stream(size: usize, last: &str) -> Vec<u8> {
    let mut out = init_line().into_bytes();
    let mut n = 0_u64;
    while out.len() + last.len() < size {
        let line =
            format!("{{\"type\":\"assistant\",\"n\":{n},\"text\":\"filler filler filler\"}}\n");
        out.extend_from_slice(line.as_bytes());
        n += 1;
    }
    out.extend_from_slice(last.as_bytes());
    out
}

/// Short lines, cut to exactly `size` bytes.
fn lines_of(size: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut n = 0_u64;
    while out.len() < size {
        out.extend_from_slice(format!("line {n}\n").as_bytes());
        n += 1;
    }
    out.truncate(size);
    out
}

/// Feeds `bytes` through the process path and returns what the run kept.
fn keep(bytes: Vec<u8>) -> String {
    drain_stdout(Cursor::new(bytes))
}

/// A reader that hands out at most `step` bytes per read, like a pipe with small writes.
struct Chunked {
    data: Vec<u8>,
    pos: usize,
    step: usize,
}

impl Read for Chunked {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let rest = &self.data[self.pos..];
        let n = rest.len().min(self.step).min(buf.len());
        buf[..n].copy_from_slice(&rest[..n]);
        self.pos += n;
        Ok(n)
    }
}

/// The dropped count the marker line states, and the marker line's length with its newline.
fn marker_of(out: &str) -> (u64, usize) {
    let start = out.find(MARK).expect("a marker line");
    let end = start + out[start..].find('\n').expect("the marker ends its line") + 1;
    let json: serde_json::Value = serde_json::from_str(&out[start..end - 1]).unwrap();
    (json["dropped_bytes"].as_u64().unwrap(), end - start)
}

#[test]
fn a_small_output_is_untouched() {
    let bytes = lines_of(1_000);
    let out = keep(bytes.clone());
    assert_eq!(out.as_bytes(), bytes.as_slice());
    assert!(!out.contains(MARK), "no marker under the bound");
}

#[test]
fn an_output_exactly_at_the_bound_is_untouched() {
    let bytes = lines_of(KEEP);
    let out = keep(bytes.clone());
    assert_eq!(out.len(), KEEP);
    assert_eq!(
        out.as_bytes(),
        bytes.as_slice(),
        "the control: no cut at the boundary"
    );
    assert!(!out.contains(MARK));
}

#[test]
fn one_byte_over_the_bound_is_cut_and_marked() {
    let out = keep(lines_of(KEEP + 1));
    let (dropped, _) = marker_of(&out);
    assert!(dropped >= 1, "dropped {dropped}");
    assert!(out.len() <= KEEP, "kept {} bytes", out.len());
}

#[test]
fn the_total_never_exceeds_the_bound() {
    let out = keep(stream(5 * 1024 * 1024, &result_line()));
    assert_ne!(out, "");
    assert!(
        out.len() <= KEEP,
        "kept {} bytes, the bound is {KEEP}",
        out.len()
    );
}

#[test]
fn the_head_survives_so_the_session_and_model_are_read() {
    let out = keep(stream(5 * 1024 * 1024, &result_line()));
    let facts: RunFacts = run_facts(&out);
    assert_eq!(facts.session_id.as_deref(), Some("sess-263"));
    assert_eq!(facts.model.as_deref(), Some("claude-haiku-5-5"));
}

#[test]
fn the_tail_survives_so_the_result_line_is_read() {
    let out = keep(stream(5 * 1024 * 1024, &result_line()));
    let facts = run_facts(&out);
    assert_eq!(facts.is_error, Some(true));
    assert_eq!(facts.api_error_status, Some(401));
    assert_eq!(facts.result_text.as_deref(), Some("LAST"));
    assert_eq!(facts.turns, Some(9));
    assert_eq!(facts.cost_usd, Some(0.25));
}

#[test]
fn the_dropped_count_is_exact() {
    let bytes = stream(5 * 1024 * 1024, &result_line());
    let total = bytes.len();
    let out = keep(bytes);
    let (dropped, marker_len) = marker_of(&out);
    assert_eq!(
        out.len() - marker_len + usize::try_from(dropped).unwrap(),
        total
    );
}

#[test]
fn the_marker_is_a_json_object_with_a_type() {
    let out = keep(stream(5 * 1024 * 1024, &result_line()));
    let line = out
        .lines()
        .find(|l| l.contains(MARK))
        .expect("a marker line");
    let json: serde_json::Value = serde_json::from_str(line).expect("the marker is JSON");
    assert_eq!(json["type"], "pii_output_gap");
}

#[test]
fn the_result_does_not_depend_on_the_chunking() {
    let data = stream(3 * 1024 * 1024, &result_line());
    let read = |step: usize| {
        drain_stdout(Chunked {
            data: data.clone(),
            pos: 0,
            step,
        })
    };
    let whole = read(data.len());
    assert_eq!(read(7), whole, "7 bytes per read");
    assert_eq!(read(1_009), whole, "1,009 bytes per read");
    assert!(whole.len() <= KEEP);
}

/// A reader that returns `good` bytes and then fails.
struct FailsAfter {
    good: Cursor<Vec<u8>>,
}

impl Read for FailsAfter {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.good.read(buf)? {
            0 => Err(std::io::Error::other("the pipe broke")),
            n => Ok(n),
        }
    }
}

#[test]
fn a_reader_error_keeps_what_was_read() {
    let bytes = lines_of(5_000);
    let out = drain_stdout(FailsAfter {
        good: Cursor::new(bytes.clone()),
    });
    assert_eq!(out.as_bytes(), bytes.as_slice());
}

#[test]
fn a_single_enormous_line_is_not_erased() {
    let line: Vec<u8> = (0..1024 * 1024)
        .map(|i| b'a' + u8::try_from(i % 26).unwrap())
        .collect();
    let out = keep(line.clone());
    assert_ne!(out, "");
    assert!(out.len() <= KEEP, "kept {} bytes", out.len());
    assert_eq!(
        out.as_bytes()[..16],
        line[..16],
        "starts with the line's first bytes"
    );
    assert_eq!(
        out.as_bytes()[out.len() - 16..],
        line[line.len() - 16..],
        "ends with its last bytes"
    );
}

/// The container fake: a libpod socket that serves one container which exits at once, with `logs` as its log stream.
/// It is the same idiom as `coder_container_bounded.rs`, which this file cannot share.
fn frame(stream: u8, text: &[u8]) -> Vec<u8> {
    let mut out = vec![stream, 0, 0, 0];
    out.extend_from_slice(&u32::try_from(text.len()).unwrap().to_be_bytes());
    out.extend_from_slice(text);
    out
}

#[derive(Default)]
struct World {
    started: Option<Instant>,
}

fn answer(line: &str, logs: &[u8], w: &mut World) -> (u16, Vec<u8>) {
    let json = |s: String| (200, s.into_bytes());
    if line.starts_with("GET /images/") && line.ends_with("/json HTTP/1.1") {
        return json(String::from(
            "{\"Digest\":\"sha256:cafebabe00000000000000000000000000000000000000000000000000000000\"}",
        ));
    }
    if line == "POST /containers/create HTTP/1.1" {
        return (201, b"{\"Id\":\"c0ffee\",\"Warnings\":[]}".to_vec());
    }
    if line.starts_with("POST /containers/c0ffee/start") {
        w.started.get_or_insert_with(Instant::now);
        return (204, Vec::new());
    }
    if line.starts_with("GET /containers/c0ffee/json") {
        let running = w
            .started
            .is_none_or(|s| s.elapsed() < Duration::from_millis(20));
        return json(format!(
            "{{\"Id\":\"c0ffee\",\"State\":{{\"Running\":{running},\"Pid\":4242,\"ExitCode\":0}}}}"
        ));
    }
    if line.starts_with("POST /containers/c0ffee/wait") {
        while w
            .started
            .is_some_and(|s| s.elapsed() < Duration::from_millis(20))
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        return json(String::from("{\"StatusCode\":0,\"Error\":null}"));
    }
    if line.starts_with("GET /containers/c0ffee/logs") {
        return (200, logs.to_vec());
    }
    if line.starts_with("DELETE /containers/c0ffee") {
        return (204, Vec::new());
    }
    (404, b"{\"message\":\"no such route\"}".to_vec())
}

/// Serves the fake on `sock` for ten seconds, one request per connection.
fn serve(sock: &Path, logs: Vec<u8>) {
    let listener = UnixListener::bind(sock).unwrap();
    listener.set_nonblocking(true).unwrap();
    let world = Arc::new(Mutex::new(World::default()));
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
            let (status, body) = answer(&line, &logs, &mut world.lock());
            let head = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = conn.write_all(head.as_bytes());
            let _ = conn.write_all(&body);
        }
    });
}

/// Runs one container whose log stream is `logs`, and returns the stdout the launch kept.
fn container_output(logs: Vec<u8>) -> String {
    let dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-output-tail-").unwrap();
    let sock = dir.path().join("p.sock");
    serve(&sock, logs);
    let runtime = PodmanApi { socket: sock };
    let argv = vec![String::from("agent")];
    let req = Request {
        image: "pii-coder:test",
        workdir: "/srv/work",
        network: "none",
        run_id: "run-263",
        skills_dir: None,
        agent_argv: &argv,
        env: &[],
    };
    let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
    match runtime
        .run(&ctx, &req, Some(Duration::from_secs(5)), &mut |_| {})
        .unwrap()
    {
        Launch::Exited { output, .. } => output,
        Launch::Refused(r) => panic!("refused: {r:?}"),
    }
}

#[test]
fn the_container_path_keeps_the_tail_too() {
    let logs = frame(1, &stream(5 * 1024 * 1024, &result_line()));
    let out = container_output(logs);
    assert!(
        out.contains("\"result\":\"LAST\""),
        "the result line is kept"
    );
    assert!(
        out.len() <= KEEP,
        "kept {} bytes, the bound is {KEEP}",
        out.len()
    );
    assert_eq!(run_facts(&out).is_error, Some(true));
}

#[test]
fn memory_stays_bounded_for_a_huge_stream() {
    /// Generates `total` bytes of claude-shaped lines without storing them: one block is repeated.
    struct Generated {
        block: Vec<u8>,
        left: usize,
    }
    impl Read for Generated {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.left.min(buf.len()).min(self.block.len());
            buf[..n].copy_from_slice(&self.block[..n]);
            self.left -= n;
            Ok(n)
        }
    }
    let block = stream(8 * 1024, "");
    let out = drain_stdout(Generated {
        block,
        left: 512 * 1024 * 1024,
    });
    assert_ne!(out, "");
    assert!(out.len() <= KEEP, "kept {} bytes", out.len());
}
