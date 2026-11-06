//! The agent can call `graph_find` through a running daemon (spec 388 R4).

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use pii_store::Store;

use super::*;

/// The agent's graph holds no memory capability and no catalogue capability: R4 and the guardrail say reading the
/// catalogue needs nothing a file read would not. The scripted model asks with `question`, as `pii graph find` does.
/// A fixture whose scripted agent answers once through `graph_find`, plus its daemon and log path.
fn ask_daemon() -> (Fixture, daemon_guard::DaemonGuard, std::path::PathBuf) {
    let fx = Fixture::new();
    let state = fx.state();
    let graph = "name = \"asks\"\nentry = \"ask\"\nexit = \"ask\"\ncaps = [\"fs_read\", \"fs_write\", \"proc\"]\n\n[[node]]\nid = \"ask\"\nkind = \"agent\"\nsystem = \"s\"\ntools = [\"graph_find\"]\ncaps = [\"fs_read\", \"fs_write\", \"proc\"]\n";
    std::fs::create_dir_all(state.join("graphs")).unwrap();
    std::fs::write(state.join("graphs/asks.toml"), graph).unwrap();
    let turns = serde_json::json!({"turns": [
        {"text": "asking", "tool_calls": [{"id": "c1", "name": "graph_find", "arguments": {"question": FLOW_QUESTION}}], "tokens_in": 1, "tokens_out": 1},
        {"text": "done", "tool_calls": [], "tokens_in": 1, "tokens_out": 1},
    ]});
    let script = fx.root.path().join("model.json");
    std::fs::write(&script, turns.to_string()).unwrap();
    let log = fx.root.path().join("daemon.log");
    let daemon = daemon_guard::spawn(
        fx.envs(&mut Command::new(env!("CARGO_BIN_EXE_pii")))
            .arg("daemon")
            .args(["--sandbox", "off", "--workspace"])
            .arg(fx.root.path())
            .arg("--log-file")
            .arg(&log)
            .env("PII_MODEL", format!("scripted:{}", script.display()))
            .stderr(Stdio::null()),
    );
    (fx, daemon, log)
}

#[test]
fn the_agent_can_call_graph_find() {
    let (fx, mut daemon, log) = ask_daemon();

    // A memory search answers only through the daemon, so its success means the socket is up. Every miss names its
    // reason on stderr, because under the gate the only evidence that survives is this test's own captured output.
    let up = wait_until(|| {
        let out = fx.pii(&["memory", "search", "up"]);
        if !out.status.success() {
            note(&format!(
                "daemon up: miss: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
            return false;
        }
        true
    });
    let indexed = up.then(|| combined(&fx.pii(&["graph", "index", "--dir", "graphs"])));
    let started = up.then(|| fx.pii(&["run", "start", "asks", "--input", r#"{"text":"hi"}"#]));
    let run_id = started.as_ref().and_then(|out| {
        let reply: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        reply["run_id"].as_str().map(str::to_owned)
    });
    let run_still_listed = |run: &str| {
        let listing = fx.pii(&["run", "ls", "--status", "running"]);
        listing.status.success() && String::from_utf8_lossy(&listing.stdout).contains(run)
    };
    let finished = run_id
        .as_deref()
        .is_some_and(|run| wait_until(|| !run_still_listed(run)));
    daemon.kill_now();

    if !up {
        let log_text =
            std::fs::read_to_string(&log).unwrap_or_else(|e| format!("daemon.log unreadable: {e}"));
        note(&format!("daemon.log after the timeout:\n{log_text}"));
        panic!("the daemon never answered; daemon.log follows on stderr");
    }
    let indexed = indexed.unwrap();
    assert!(
        indexed_count(&indexed).is_some_and(|n| n > 0),
        "the index must work beside a running daemon, which holds the store: {indexed}"
    );
    let run_id = run_id.unwrap_or_else(|| panic!("run start: {started:?}"));
    assert!(finished, "run {run_id} never finished");

    // `tools.call` is only emitted for a tool the set holds, so an `ok` here means the call was dispatched and answered.
    let log_text = std::fs::read_to_string(&log).unwrap();
    let dispatched = graph_find_outcomes(&log_text);
    assert_eq!(
        dispatched,
        ["ok"],
        "the graph_find dispatches in the daemon log"
    );

    let store = Store::open(&fx.state().join("store.redb")).unwrap();
    let journal = store
        .read_journal(&pii_abi::RunId::from(run_id.as_str()))
        .unwrap();
    let recorded = journal.iter().any(|(_, event)| {
        matches!(event, pii_abi::RunEvent::NodeObserved { .. })
            && serde_json::to_string(event).unwrap().contains("graph_find")
    });
    assert!(
        recorded,
        "the run's journal must record the graph_find call (R5 counts them from it): {journal:?}"
    );
}

/// Polls `done` for up to 120 seconds: inside the full gate the daemon's first answer took over 30 seconds under
/// the load, while every solo run has been under four (measured five times, 2026-11-06).
fn wait_until(mut done: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(120);
    while Instant::now() < end {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// The outcome of every `graph_find` dispatch the daemon logged, in order.
fn graph_find_outcomes(log_text: &str) -> Vec<String> {
    log_text
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|e| e["kind"] == "tools.call" && e["fields"]["tool"] == "graph_find")
        .map(|e| {
            e["fields"]["outcome"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}
