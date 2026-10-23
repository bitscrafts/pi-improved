//! Spec 241: a coder agent that runs in a container is bounded by the node's `timeout_ms`, and its output reaches the
//! journal. The runtime socket is a fake libpod the test serves itself, under `_tmp`, so the test counts the calls the
//! node makes, in order, and answers each one. Controls: a container that exits in time, a failing container, and a
//! node with no timeout are left as they were.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_graph::NodeRegistry;
use pii_observe::{Event, Observer, names};
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};
use pii_server::node_kinds::nodes::coder_runtime::{
    ContainerRuntime, Launch, PodmanApi, Request, TIMEOUT_EXIT_CODE,
};

/// The most output one stream keeps (spec 209's `OUTPUT_KEEP_BYTES`, private to the runtime).
const KEEP: usize = 256 * 1024;

/// What the fake container does: when it exits on its own and with what code (`None`: it never exits until it is
/// stopped), the log stream it serves, and the status its logs call answers with.
#[derive(Clone)]
struct Fake {
    exit: Option<(Duration, i64)>,
    logs: Vec<u8>,
    logs_status: u16,
}

/// The container as the fake has seen it, and every request line the node sent, in order.
#[derive(Default)]
struct World {
    started: Option<Instant>,
    stopped: bool,
    calls: Vec<String>,
}

/// One log frame, as the API sends a non-tty container's logs: stream (1 stdout, 2 stderr), three zero bytes, the
/// payload's big-endian length, then the payload.
fn frame(stream: u8, text: &[u8]) -> Vec<u8> {
    let mut out = vec![stream, 0, 0, 0];
    out.extend_from_slice(&u32::try_from(text.len()).unwrap().to_be_bytes());
    out.extend_from_slice(text);
    out
}

/// Whether the container is running, and its exit code once it is not.
fn state(fake: &Fake, w: &World) -> (bool, i64) {
    let Some(started) = w.started else {
        return (false, 0);
    };
    if w.stopped {
        return (false, 137);
    }
    match fake.exit {
        Some((after, code)) if started.elapsed() >= after => (false, code),
        _ => (true, 0),
    }
}

fn answer(line: &str, fake: &Fake, w: &mut World) -> (u16, Vec<u8>) {
    let json = |s: String| (200, s.into_bytes());
    if line.starts_with("GET /images/") && line.ends_with("/json HTTP/1.1") {
        return json(String::from(
            "{\"Digest\":\"sha256:cafebabe00000000000000000000000000000000\"}",
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
        let (running, code) = state(fake, w);
        return json(format!(
            "{{\"Id\":\"c0ffee\",\"State\":{{\"Running\":{running},\"Pid\":4242,\"ExitCode\":{code}}}}}"
        ));
    }
    if line.starts_with("POST /containers/c0ffee/stop") {
        w.stopped = true;
        return (204, Vec::new());
    }
    if line.starts_with("POST /containers/c0ffee/wait") {
        // Like libpod: the reply comes when the container has exited (bounded here so a test cannot hang).
        let limit = Instant::now() + Duration::from_secs(5);
        while state(fake, w).0 && Instant::now() < limit {
            std::thread::sleep(Duration::from_millis(5));
        }
        let (_, code) = state(fake, w);
        return json(format!("{{\"StatusCode\":{code},\"Error\":null}}"));
    }
    if line.starts_with("GET /containers/c0ffee/logs") {
        return (fake.logs_status, fake.logs.clone());
    }
    if line.starts_with("DELETE /containers/c0ffee") {
        return (204, Vec::new());
    }
    (404, b"{\"message\":\"no such route\"}".to_vec())
}

/// Serves the fake on `sock` for ten seconds, one request per connection, and returns the calls it saw.
fn serve(sock: &Path, fake: Fake) -> Arc<Mutex<World>> {
    let world = Arc::new(Mutex::new(World::default()));
    let listener = UnixListener::bind(sock).unwrap();
    listener.set_nonblocking(true).unwrap();
    let shared = Arc::clone(&world);
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
            shared.lock().calls.push(line.clone());
            let (status, body) = answer(&line, &fake, &mut shared.lock());
            let head = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = conn.write_all(head.as_bytes());
            let _ = conn.write_all(&body);
        }
    });
    world
}

/// The outcome of one direct launch against the fake: the launch, and the calls the node made in order.
struct Direct {
    code: i32,
    output: String,
    stderr: String,
    unreadable: Option<String>,
    calls: Vec<String>,
    took: Duration,
}

/// Launches a container through the socket runtime against `fake`, bounded by `timeout`.
fn run_direct(fake: Fake, timeout: Option<Duration>) -> Direct {
    let dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bounded-").unwrap();
    let sock = dir.path().join("p.sock");
    let world = serve(&sock, fake);
    let runtime = PodmanApi { socket: sock };
    let argv = vec![String::from("agent")];
    let req = Request {
        image: "pii-coder:test",
        workdir: "/srv/work",
        network: "none",
        run_id: "run-241",
        skills_dir: None,
        agent_argv: &argv,
        env: &[],
    };
    let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
    let started = Instant::now();
    let launch = runtime.run(&ctx, &req, timeout, &mut |_| {}).unwrap();
    let took = started.elapsed();
    let calls = world.lock().calls.clone();
    match launch {
        Launch::Exited {
            code,
            output,
            stderr,
            logs_unreadable,
            ..
        } => Direct {
            code,
            output,
            stderr,
            unreadable: logs_unreadable,
            calls,
            took,
        },
        Launch::Refused(r) => panic!("refused: {r:?}"),
    }
}

/// The index of the first call that starts with `prefix`; panics when the node never made it.
fn at(calls: &[String], prefix: &str) -> usize {
    calls
        .iter()
        .position(|c| c.starts_with(prefix))
        .unwrap_or_else(|| panic!("no call {prefix}: {calls:?}"))
}

fn has(calls: &[String], prefix: &str) -> bool {
    calls.iter().any(|c| c.starts_with(prefix))
}

#[test]
fn a_container_that_outlives_its_timeout_is_stopped() {
    let run = run_direct(
        Fake {
            exit: None,
            logs: Vec::new(),
            logs_status: 200,
        },
        Some(Duration::from_millis(300)),
    );
    assert_eq!(run.code, TIMEOUT_EXIT_CODE, "a timeout exits 124");
    assert!(
        run.took < Duration::from_secs(3),
        "the deadline must interrupt the wait, took {:?}",
        run.took
    );
    assert!(
        at(&run.calls, "POST /containers/c0ffee/stop")
            < at(&run.calls, "DELETE /containers/c0ffee"),
        "stop, then force removal: {:?}",
        run.calls
    );
}

#[test]
fn a_container_that_exits_in_time_is_untouched() {
    let run = run_direct(
        Fake {
            exit: Some((Duration::from_millis(50), 0)),
            logs: Vec::new(),
            logs_status: 200,
        },
        Some(Duration::from_secs(5)),
    );
    assert_eq!(run.code, 0, "near-miss: inside the timeout nothing changes");
    assert!(
        !has(&run.calls, "POST /containers/c0ffee/stop"),
        "a container that exits in time is never stopped: {:?}",
        run.calls
    );
}

#[test]
fn no_timeout_waits_for_the_container() {
    let run = run_direct(
        Fake {
            exit: Some((Duration::from_millis(400), 0)),
            logs: Vec::new(),
            logs_status: 200,
        },
        None,
    );
    assert_eq!(run.code, 0, "no timeout: the container's own exit code");
    assert!(!has(&run.calls, "POST /containers/c0ffee/stop"));
}

#[test]
fn a_failing_container_keeps_its_exit_code() {
    let run = run_direct(
        Fake {
            exit: Some((Duration::from_millis(50), 3)),
            logs: Vec::new(),
            logs_status: 200,
        },
        Some(Duration::from_secs(5)),
    );
    assert_eq!(run.code, 3, "a failing container is not a timeout");
}

#[test]
fn the_containers_stdout_and_stderr_are_returned() {
    let mut logs = frame(1, b"out-line\n");
    logs.extend(frame(2, b"err-line\n"));
    let run = run_direct(
        Fake {
            exit: Some((Duration::from_millis(20), 0)),
            logs,
            logs_status: 200,
        },
        Some(Duration::from_secs(5)),
    );
    assert!(run.output.contains("out-line"), "stdout: {:?}", run.output);
    assert!(run.stderr.contains("err-line"), "stderr: {:?}", run.stderr);
    assert!(!run.output.contains("err-line"), "streams are kept apart");
}

#[test]
fn the_logs_are_read_before_the_container_is_removed() {
    let run = run_direct(
        Fake {
            exit: Some((Duration::from_millis(20), 0)),
            logs: frame(1, b"out-line\n"),
            logs_status: 200,
        },
        Some(Duration::from_secs(5)),
    );
    assert!(
        at(&run.calls, "GET /containers/c0ffee/logs") < at(&run.calls, "DELETE /containers/c0ffee"),
        "logs precede the force removal: {:?}",
        run.calls
    );
}

#[test]
fn a_timeout_still_returns_the_logs() {
    let run = run_direct(
        Fake {
            exit: None,
            logs: frame(1, b"hung-line\n"),
            logs_status: 200,
        },
        Some(Duration::from_millis(300)),
    );
    assert_eq!(run.code, TIMEOUT_EXIT_CODE);
    assert!(run.output.contains("hung-line"), "output: {:?}", run.output);
}

#[test]
fn the_logs_are_bounded() {
    let big = vec![b'x'; 5 * 1024 * 1024];
    let run = run_direct(
        Fake {
            exit: Some((Duration::from_millis(20), 0)),
            logs: frame(1, &big),
            logs_status: 200,
        },
        Some(Duration::from_secs(5)),
    );
    assert!(!run.output.is_empty(), "the output was read at all");
    assert!(
        run.output.len() <= KEEP,
        "kept {} bytes, the bound is {KEEP}",
        run.output.len()
    );
}

#[test]
fn unreadable_logs_do_not_hide_the_exit_code() {
    let run = run_direct(
        Fake {
            exit: Some((Duration::from_millis(20), 0)),
            logs: b"{\"message\":\"boom\"}".to_vec(),
            logs_status: 500,
        },
        Some(Duration::from_secs(5)),
    );
    assert_eq!(run.code, 0, "the exit code survives unreadable logs");
    assert!(run.output.is_empty(), "no logs, no output");
    assert_eq!(
        run.unreadable.as_deref(),
        Some("c0ffee"),
        "the launch names the container whose logs it could not read"
    );
    assert!(
        has(&run.calls, "DELETE /containers/c0ffee"),
        "still removed"
    );
}

// --- node level: the journal ---

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

/// The root context the coder node runs under: spawn and proc, the run's own agent binary allowed.
fn root_ctx(bin: &Path) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![String::from("/bin/sh")];
    argv0.extend(agent_binaries(bin));
    caps.proc_argv0 = ArgvScope::new(argv0);
    Ctx::root(caps, Budget::unlimited())
}

/// Runs a one-node `coder` graph whose container is the fake, and returns every event it journalled.
fn run_node(fake: Fake) -> Vec<Event> {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-node-").unwrap();
    let sock = dir.path().join("p.sock");
    let _world = serve(&sock, fake);
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-store-").unwrap();
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
    let store = pii_store::Store::open(&store_dir.path().join("store.redb")).unwrap();
    let log = Arc::new(Rec::default());
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(
        CoderFactory::with_env(bin.path().to_path_buf(), BTreeMap::new(), log.clone())
            .with_runtime_socket(sock)
            .with_runs_dir(store_dir.path().join("runs")),
    ));
    let src = format!(
        "name = \"coder-bounded\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\nagent = \"pi\"\nmodel = \"m-one\"\nbrief = \"do it\"\nspec = \"specs/241-x.md\"\nworkdir = \"{}\"\ncontainer_image = \"pii-coder:test\"\ncontainer_runtime = \"socket\"\n\n[[node]]\nid = \"done\"\nkind = \"transform\"\nvalue = \"done\"\n\n[[edge]]\nfrom = \"code\"\nport = \"ok\"\nto = \"done\"\nkind = \"data\"\n",
        work.path().display()
    );
    let spec = pii_graph::load_toml_with_registry(&src, &reg).unwrap();
    let run_id = store.next_run_id().unwrap();
    pii_graph::run::run_with_validate(
        &store,
        &spec,
        &reg,
        &root_ctx(bin.path()),
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap();
    log.0.lock().clone()
}

#[test]
fn a_secret_in_the_container_log_is_redacted_in_the_journal() {
    let events = run_node(Fake {
        exit: Some((Duration::from_millis(20), 0)),
        logs: frame(1, b"agent-said-hello sk-live-FAKESECRET123456789\n"),
        logs_status: 200,
    });
    let exited = events
        .iter()
        .find(|e| e.kind == names::CODER_EXITED)
        .expect("coder.exited is journalled");
    let journal = serde_json::to_string(exited).unwrap();
    // Non-vacuous: the log reached the journal at all, so the absence below means something.
    assert!(
        journal.contains("agent-said-hello"),
        "the container's log reaches coder.exited: {journal}"
    );
    assert!(
        !journal.contains("FAKESECRET"),
        "the token must not reach the journal: {journal}"
    );
}

#[test]
fn unreadable_logs_journal_one_warning_naming_the_container() {
    let events = run_node(Fake {
        exit: Some((Duration::from_millis(20), 0)),
        logs: Vec::new(),
        logs_status: 500,
    });
    let warnings: Vec<&Event> = events
        .iter()
        .filter(|e| e.kind == names::CODER_LOGS_UNREADABLE)
        .collect();
    assert_eq!(warnings.len(), 1, "one warning: {events:?}");
    let text = serde_json::to_string(warnings[0]).unwrap();
    assert!(
        text.contains("c0ffee"),
        "the warning names the container: {text}"
    );
}
