//! Spec 305: a run id `Engine::start_run` has handed back is already in the store. The header and the `Started` event are written
//! before the id is returned, so a caller that loses the daemon after the acknowledgement finds the run, not an unknown id.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use pii_abi::{RunEvent, RunId, RunStatus, Terminal};
use pii_proto::event::Event;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

/// The first node waits on a timer, so the run is still in flight when the id comes back. The timer is short: runtime teardown waits
/// for the blocking threads a parked run holds.
const BLOCKING: &str = r#"name = "blocking"
entry = "a"
caps = []

[[node]]
id = "a"
kind = "wait"
timer_ms = 1500

[[node]]
id = "b"
kind = "transform"
value = "y"

[[edge]]
from = "a"
port = "fired"
to = "b"
kind = "data"
"#;

/// Two transforms: it finishes at once.
const FINE: &str = r#"name = "fine"
entry = "a"
caps = []

[[node]]
id = "a"
kind = "transform"
value = "x"

[[node]]
id = "b"
kind = "transform"
value = "y"

[[edge]]
from = "a"
port = "ok"
to = "b"
kind = "data"
"#;

/// A cycle the run rejects (the run journal refuses any cycle, Phase-1 boundary), so the start is refused before its first journal write.
const CYCLIC: &str = r#"name = "cyclic"
entry = "a"
caps = []

[[node]]
id = "a"
kind = "transform"
value = "x"
join = "any"
loop_budget = 2

[[node]]
id = "b"
kind = "transform"
value = "y"

[[edge]]
from = "a"
port = "ok"
to = "b"
kind = "data"

[[edge]]
from = "b"
port = "ok"
to = "a"
kind = "data"
"#;

fn hello() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

/// A graph that exists and is REFUSED at load for a reason that is not a matter of opinion: its edge
/// leaves a port the node does not have (V1). The control below needs a refused start that still hands
/// back an id, which only a graph that loads-then-refuses can give; a missing graph returns `NotFound`
/// instead. It used to borrow the one-armed-`any` refusal, which spec 380 turned into a warning.
const REFUSED_START: &str = r#"name = "refused-start"
entry = "a"
caps = []

[[node]]
id = "a"
kind = "transform"
value = "x"

[[node]]
id = "b"
kind = "transform"
value = "y"

[[edge]]
from = "a"
port = "nope"
to = "b"
"#;

fn engine_in(dir: &Path) -> Arc<Engine> {
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    std::fs::write(dir.join("graphs/blocking.toml"), BLOCKING).unwrap();
    std::fs::write(dir.join("graphs/fine.toml"), FINE).unwrap();
    std::fs::write(dir.join("graphs/cyclic.toml"), CYCLIC).unwrap();
    std::fs::write(dir.join("graphs/refused-start.toml"), REFUSED_START).unwrap();
    Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir.join("ws"),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", hello().display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        models: pii_server::daemon::empty_model_roles(),
        graph_dir: dir.join("graphs"),
        mqtt_tool_url: None,
    })
    .expect("engine")
}

fn started_count(engine: &Engine, run: &RunId) -> usize {
    engine
        .store
        .read_journal(run)
        .unwrap()
        .iter()
        .filter(|(_, e)| matches!(e, RunEvent::Started { .. }))
        .count()
}

/// Waits (a condition, not a sleep) until the run leaves `Running`.
fn wait_terminal(engine: &Engine, run: &RunId) -> RunStatus {
    for _ in 0..6000 {
        if let Ok(Some(h)) = engine.store.get_run(run)
            && h.status != RunStatus::Running
        {
            return h.status;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("run {run} never left Running");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_acknowledged_run_is_in_the_store() {
    let d = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "ack-store-").unwrap();
    let e = engine_in(d.path());
    for _ in 0..200 {
        let run = e
            .start_run("blocking", &serde_json::json!({}))
            .expect("the graph starts");
        let h = e
            .store
            .get_run(&run)
            .unwrap()
            .unwrap_or_else(|| panic!("{run} is acknowledged but not in the store"));
        assert_eq!(h.status, RunStatus::Running, "{run}");
        assert_eq!(started_count(&e, &run), 1, "{run} has no Started event");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn control_a_refused_start_still_resolves() {
    let d = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "ack-refused-").unwrap();
    let e = engine_in(d.path());
    let run = e
        .start_run("refused-start", &serde_json::json!({}))
        .expect("a refused start still hands back its id");
    let h = e
        .store
        .get_run(&run)
        .unwrap()
        .unwrap_or_else(|| panic!("{run} is acknowledged but not in the store"));
    assert_eq!(h.status, RunStatus::Failed, "{run}");
    let journal = e.store.read_journal(&run).unwrap();
    assert!(
        journal.iter().any(|(_, ev)| matches!(
            ev,
            RunEvent::Finished {
                terminal: Terminal::Failure { error },
                ..
            } if !error.is_empty()
        )),
        "a refused run journals a terminal Failed with its reason: {journal:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_started_is_never_written() {
    let d = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "ack-once-").unwrap();
    let e = engine_in(d.path());
    let run = e
        .start_run("fine", &serde_json::json!({}))
        .expect("the graph starts");
    assert_eq!(wait_terminal(&e, &run), RunStatus::Succeeded, "{run}");
    assert_eq!(
        started_count(&e, &run),
        1,
        "{run} wrote Started more than once"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_is_registered_before_it_can_finish() {
    let d = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "ack-register-").unwrap();
    let e = engine_in(d.path());
    for _ in 0..50 {
        let log = Arc::new(parking_lot::Mutex::new(Vec::<&'static str>::new()));
        let mut rx = e.bus.subscribe();
        let seen = Arc::new(parking_lot::Mutex::new(None::<String>));
        let watcher = {
            let (log, seen) = (Arc::clone(&log), Arc::clone(&seen));
            std::thread::spawn(move || {
                // Wait for the id the callback recorded, then for that run's terminal event on the bus.
                let id = loop {
                    let found = seen.lock().clone();
                    if let Some(id) = found {
                        break id;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                };
                while let Ok(se) = rx.blocking_recv() {
                    if let Event::RunFinished { run_id, .. } = se.event
                        && run_id == id
                    {
                        log.lock().push("finished");
                        break;
                    }
                }
            })
        };
        let registered = AtomicBool::new(false);
        let run = e
            .start_run_with("fine", &serde_json::json!({}), |id| {
                log.lock().push("registered");
                *seen.lock() = Some(id.to_string());
                registered.store(true, Ordering::SeqCst);
            })
            .expect("the graph starts");
        assert!(registered.load(Ordering::SeqCst), "the callback never ran");
        watcher.join().unwrap();
        assert_eq!(
            *log.lock(),
            vec!["registered", "finished"],
            "{run} finished before it was registered"
        );
    }
}

/// Prints the median and the max start time, as the contract asks.
#[allow(clippy::print_stdout, clippy::disallowed_macros)]
#[tokio::test(flavor = "multi_thread")]
async fn the_start_stays_fast() {
    let d = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "ack-fast-").unwrap();
    let e = engine_in(d.path());
    let mut took = Vec::new();
    for _ in 0..50 {
        let t = Instant::now();
        let run = e
            .start_run("blocking", &serde_json::json!({}))
            .expect("the graph starts");
        took.push(t.elapsed());
        // The point of the bound: the start does not wait for the run. The graph
        // blocks, so the run is still going when the id comes back.
        let h = e
            .store
            .get_run(&run)
            .unwrap()
            .expect("header is in the store");
        assert_eq!(h.status, RunStatus::Running, "{run} was waited for");
    }
    took.sort();
    let median = took[took.len() / 2];
    let max = *took.last().unwrap();
    println!("start_run: median {median:?}, max {max:?}");
    // A wall-clock maximum is environment-dependent (the gate's coverage build is
    // instrumented and shares the machine): the median is the stable figure, and
    // the maximum only catches a start that hangs.
    assert!(
        median < Duration::from_millis(50),
        "median start_run {median:?}"
    );
    assert!(max < Duration::from_secs(5), "start_run hung: {max:?}");
}
