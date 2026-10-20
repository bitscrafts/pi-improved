//! Spec 230: a `coder` node with `container_runtime = "socket"` starts its container through the podman REST API
//! over the runtime socket, not by exec'ing podman. The socket is a fake runtime the test serves itself, under
//! `_tmp`, so the test can read every request the node makes and answer each one.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use pii_abi::Terminal;
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_graph::NodeRegistry;
use pii_observe::{Event, Observer, names};
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

/// What the fake runtime answers: the create status (201 normally), the pid and exit code it reports for the
/// container, and whether it serves at all.
struct Fake {
    create_status: u16,
    pid: i64,
    exit_code: i64,
}

/// Serves the podman REST paths the node uses on `sock`, one request per connection, and returns the request
/// lines it saw. It stops after ten seconds so a node that never calls it cannot hang the test.
fn serve(sock: &Path, fake: Fake) -> Arc<Mutex<Vec<String>>> {
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
            // Read until the headers and the declared body are both in.
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
            let (status, body) = answer(&line, &fake);
            let reply = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = conn.write_all(reply.as_bytes());
        }
    });
    seen
}

fn answer(line: &str, fake: &Fake) -> (u16, String) {
    if line.starts_with("GET /images/") && line.ends_with("/json HTTP/1.1") {
        return (
            200,
            String::from("{\"Digest\":\"sha256:cafebabe00000000000000000000000000000000\"}"),
        );
    }
    if line == "POST /containers/create HTTP/1.1" {
        return (
            fake.create_status,
            if fake.create_status == 201 {
                String::from("{\"Id\":\"c0ffee\",\"Warnings\":[]}")
            } else {
                String::from("{\"cause\":\"boom\",\"message\":\"runtime broke\",\"response\":500}")
            },
        );
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
            format!(
                "{{\"Id\":\"c0ffee\",\"State\":{{\"Running\":false,\"Pid\":{},\"ExitCode\":{}}}}}",
                fake.pid, fake.exit_code
            ),
        );
    }
    if line.starts_with("DELETE /containers/c0ffee") {
        return (204, String::new());
    }
    (404, String::from("{\"message\":\"no such route\"}"))
}

/// A root context with SPAWN + PROC and the run scope's programs, as the coder tests build it.
fn root_ctx(bin: &Path) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![String::from("/bin/sh")];
    argv0.extend(agent_binaries(bin));
    caps.proc_argv0 = ArgvScope::new(argv0);
    Ctx::root(caps, Budget::unlimited())
}

/// Runs a one-node `coder` graph over the socket at `sock` and returns the terminal and the events.
fn run_over_socket(sock: &Path, work: &Path) -> (Terminal, Vec<Event>) {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-store-").unwrap();
    let store = pii_store::Store::open(&store_dir.path().join("store.redb")).unwrap();
    let log = Arc::new(Rec::default());
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(
        CoderFactory::with_env(bin.path().to_path_buf(), BTreeMap::new(), log.clone())
            .with_runtime_socket(sock.to_path_buf())
            .with_runs_dir(store_dir.path().join("runs")),
    ));
    let src = format!(
        "name = \"coder-socket\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\nagent = \"pi\"\nmodel = \"m-one\"\nbrief = \"do it\"\nspec = \"specs/230-x.md\"\nworkdir = \"{}\"\ncontainer_image = \"pii-coder:test\"\ncontainer_runtime = \"socket\"\n\n[[node]]\nid = \"done\"\nkind = \"transform\"\nvalue = \"done\"\n\n[[node]]\nid = \"refused\"\nkind = \"transform\"\nvalue = \"refused\"\n\n[[edge]]\nfrom = \"code\"\nport = \"ok\"\nto = \"done\"\nkind = \"data\"\n\n[[edge]]\nfrom = \"code\"\nport = \"err\"\nto = \"refused\"\nkind = \"data\"\n",
        work.display()
    );
    let spec = pii_graph::load_toml_with_registry(&src, &reg).unwrap();
    let run_id = store.next_run_id().unwrap();
    let terminal = pii_graph::run::run_with_validate(
        &store,
        &spec,
        &reg,
        &root_ctx(bin.path()),
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap();
    let events = log.0.lock().clone();
    (terminal, events)
}

fn sock_in(dir: &Path) -> PathBuf {
    dir.join("p.sock")
}

fn kinds(events: &[Event]) -> Vec<&str> {
    events.iter().map(|e| e.kind).collect()
}

fn field<'a>(e: &'a Event, key: &str) -> Option<&'a serde_json::Value> {
    e.fields.get(key)
}

/// The node starts the container through the API: create, start, wait, inspect and remove, in that order, and
/// the journal names the container and its pid (`coder.spawned`) and the exit code (`coder.exited`).
#[test]
fn coder_starts_through_the_runtime_socket() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-sock-").unwrap();
    let sock = sock_in(dir.path());
    let seen = serve(
        &sock,
        Fake {
            create_status: 201,
            pid: 4242,
            exit_code: 7,
        },
    );
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();

    let (terminal, events) = run_over_socket(&sock, work.path());

    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "got {terminal:?}"
    );
    let spawned = events
        .iter()
        .find(|e| e.kind == names::CODER_SPAWNED)
        .expect("coder.spawned journalled");
    assert_eq!(
        field(spawned, "container").and_then(serde_json::Value::as_str),
        Some("c0ffee"),
        "the journal names the container"
    );
    assert_eq!(
        field(spawned, "pid").and_then(serde_json::Value::as_i64),
        Some(4242),
        "the journal names the pid the runtime reports"
    );
    assert_eq!(
        field(spawned, "runtime").and_then(serde_json::Value::as_str),
        Some("socket"),
        "the resolved runtime is named"
    );
    // Spec 232 R4: the runtime's own digest reaches the journal, so the trace's image slot
    // carries what actually ran, never an empty string.
    assert_eq!(
        field(spawned, "image_digest").and_then(serde_json::Value::as_str),
        Some("sha256:cafebabe00000000000000000000000000000000"),
        "the digest the runtime inspected is journalled"
    );
    let exited = events
        .iter()
        .find(|e| e.kind == names::CODER_EXITED)
        .expect("coder.exited journalled");
    assert_eq!(
        field(exited, "rc").and_then(serde_json::Value::as_i64),
        Some(7),
        "the rc is the container's ExitCode"
    );
    assert_eq!(
        field(exited, "pid").and_then(serde_json::Value::as_i64),
        Some(4242)
    );
    let lines = seen.lock().clone();
    let at = |needle: &str| {
        lines
            .iter()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("no {needle} in {lines:?}"))
    };
    assert!(
        at("POST /containers/create") < at("POST /containers/c0ffee/start")
            && at("POST /containers/c0ffee/start") < at("POST /containers/c0ffee/wait")
            && at("POST /containers/c0ffee/wait") < at("DELETE /containers/c0ffee"),
        "create, start, wait, remove in order: {lines:?}"
    );
    assert!(
        !kinds(&events).contains(&names::CODER_IMAGE_MISSING),
        "a live runtime never reports a missing image"
    );
}

/// Control (a): no socket at the named path is a refusal before any spawn, and the journal names the path.
#[test]
fn absent_socket_refuses_before_any_spawn() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-sock-").unwrap();
    let sock = sock_in(dir.path());
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();

    let (terminal, events) = run_over_socket(&sock, work.path());

    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "the refusal routes to err: {terminal:?}"
    );
    let unavailable = events
        .iter()
        .find(|e| e.kind == names::CODER_RUNTIME_UNAVAILABLE)
        .expect("coder.runtime.unavailable journalled");
    assert_eq!(
        field(unavailable, "socket").and_then(serde_json::Value::as_str),
        Some(sock.display().to_string().as_str()),
        "the path is in the event"
    );
    let k = kinds(&events);
    assert!(!k.contains(&names::CODER_SPAWNED), "nothing spawned: {k:?}");
    assert!(
        !k.contains(&names::CODER_IMAGE_MISSING),
        "a runtime failure is never the image: {k:?}"
    );
}

/// Control (b): a 500 on create fails the node before any spawn, and the status is quoted.
#[test]
fn a_500_on_create_refuses_with_the_status() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-sock-").unwrap();
    let sock = sock_in(dir.path());
    let seen = serve(
        &sock,
        Fake {
            create_status: 500,
            pid: 0,
            exit_code: 0,
        },
    );
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();

    let (_, events) = run_over_socket(&sock, work.path());

    let unavailable = events
        .iter()
        .find(|e| e.kind == names::CODER_RUNTIME_UNAVAILABLE)
        .expect("coder.runtime.unavailable journalled");
    assert_eq!(
        field(unavailable, "status").and_then(serde_json::Value::as_i64),
        Some(500),
        "the HTTP status is quoted"
    );
    assert!(
        field(unavailable, "reason")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|r| r.contains("runtime broke")),
        "the runtime's own message is quoted: {unavailable:?}"
    );
    let k = kinds(&events);
    assert!(!k.contains(&names::CODER_SPAWNED), "no spawn: {k:?}");
    assert!(
        seen.lock().iter().all(|l| !l.contains("/start")),
        "no start after a refused create: {:?}",
        seen.lock()
    );
}
