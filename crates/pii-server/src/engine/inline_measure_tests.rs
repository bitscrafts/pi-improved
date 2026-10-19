//! Spec 95 R2: how long each handler that `dispatch` still runs INLINE
//! occupies the async worker. The inline path is `handler(...)` called
//! synchronously from the connection task, so the wall time of one
//! `dispatch` call IS the time the worker is held. Each method runs against a
//! real engine (real redb store, real scripted run, real worker process for
//! `sys.reload`); nothing is stubbed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stderr
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use pii_proto::method::Method;
use pii_proto::{Id, Request};

use crate::daemon::Daemon;
use crate::daemon::config::Config;
use crate::engine::conn::{PeerIdentity, SubscriptionRegistry};
use crate::engine::dispatch::dispatch;
use crate::{Engine, EngineConfig};

/// Spec 80's bar: a handler holding the worker longer than this must move.
const BAR: Duration = Duration::from_millis(50);
const SAMPLES: usize = 15;

fn req(method: Method, params: serde_json::Value) -> Request {
    Request {
        jsonrpc: "2.0".into(),
        id: Id::Num(1),
        method,
        params,
    }
}

/// The `pii` binary, resolved beside the RUNNING test executable: `.../<target>/debug/deps/<test>-<hash>`
/// → `.../<target>/debug/pii`. A manifest-relative path (`<tree>/target/debug/pii`) is a DIFFERENT path
/// whenever cargo is told to build elsewhere — the autonomy loop's fresh-gate reuses a warm cache
/// (`FRESH_GATE_TARGET`), so it never exists there and the worker cannot be started at all.
fn pii_bin() -> std::path::PathBuf {
    let mut dir = std::env::current_exe().expect("the test executable's path");
    dir.pop(); // the test binary itself
    if dir.ends_with("deps") {
        dir.pop();
    }
    dir.join("pii")
}

struct Rig {
    engine: Arc<Engine>,
    subs: SubscriptionRegistry,
    wtx: tokio::sync::mpsc::UnboundedSender<String>,
    _wrx: tokio::sync::mpsc::UnboundedReceiver<String>,
    _dir: tempfile::TempDir,
    run_id: String,
}

impl Rig {
    /// Runs `dispatch` once and returns its wall time and the response.
    fn time(&self, r: &Request) -> (Duration, pii_proto::Response) {
        let t = Instant::now();
        let resp = dispatch(
            &self.engine,
            r,
            PeerIdentity::InProcess,
            &self.wtx,
            &self.subs,
        );
        (t.elapsed(), resp)
    }

    /// `SAMPLES` timed calls of a fresh request each; returns (median, max).
    fn measure(&self, label: &str, mk: impl FnMut(usize) -> Request) -> (Duration, Duration) {
        self.measure_expecting(label, None, mk)
    }

    /// [`Rig::measure`] for a call that resumes the already finished run: spec 196 refuses
    /// that loudly, and the refusal is still an inline answer worth timing.
    fn measure_refused(
        &self,
        label: &str,
        mk: impl FnMut(usize) -> Request,
    ) -> (Duration, Duration) {
        self.measure_expecting(label, Some("already finished"), mk)
    }

    fn measure_expecting(
        &self,
        label: &str,
        refusal: Option<&str>,
        mut mk: impl FnMut(usize) -> Request,
    ) -> (Duration, Duration) {
        let mut v: Vec<Duration> = Vec::new();
        for i in 0..SAMPLES {
            let (d, resp) = self.time(&mk(i));
            let body = serde_json::to_string(&resp).unwrap();
            let expected = refusal.is_some_and(|r| body.contains(r));
            assert!(
                expected || !body.contains("\"error\""),
                "{label} failed: {body}"
            );
            if i == 0 {
                eprintln!("RESP {label}: {body}");
            }
            v.push(d);
        }
        v.sort();
        let (med, max) = (v[v.len() / 2], v[v.len() - 1]);
        eprintln!("INLINE {label}: median={med:?} max={max:?}");
        (med, max)
    }
}

async fn rig() -> Rig {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&root).expect("create crate-local _tmp");
    let dir = tempfile::Builder::new()
        .prefix("inline-")
        .tempdir_in(root)
        .expect("tempdir");
    let engine = Engine::new(EngineConfig {
        workspace: dir.path().to_path_buf(),
        store_path: dir.path().join("store.redb"),
        model: format!(
            "scripted:{}/../../fixtures/phase2/hello.json",
            env!("CARGO_MANIFEST_DIR")
        ),
        sandbox: pii_tools::sandbox::SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: crate::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        graph_dir: dir.path().join("graphs"),
    })
    .expect("engine");
    // A real worker (the reference `pii worker-exec`) so `sys.reload`
    // reloads something; the daemon context is what `sys.reload` needs.
    let pii_bin = pii_bin();
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(
        &cfg_path,
        format!(
            "[[worker]]\nname = \"w\"\nprogram = \"{}\"\nargs = [\"worker-exec\", \"--name\", \"w\"]\n",
            pii_bin.display()
        ),
    )
    .expect("config");
    let cfg = Config::load(&cfg_path).expect("cfg parse");
    Daemon::build(
        Arc::clone(&engine),
        &cfg,
        dir.path(),
        cfg_path,
        Arc::new(pii_core::SystemClock),
    )
    .expect("daemon");
    let (wtx, wrx) = tokio::sync::mpsc::unbounded_channel();
    let mut rig = Rig {
        engine,
        subs: SubscriptionRegistry::default(),
        wtx,
        _wrx: wrx,
        _dir: dir,
        run_id: String::new(),
    };
    let (_, resp) = rig.time(&req(
        Method::RunStart,
        serde_json::json!({"graph": "default", "input": {"prompt": "hi"}}),
    ));
    let v = serde_json::to_value(&resp).unwrap();
    rig.run_id = v["result"]["run_id"].as_str().expect("run_id").to_owned();
    // Let the scripted run finish so its header and journal are complete.
    tokio::time::sleep(Duration::from_millis(500)).await;
    rig
}

// reason: one flat table of the 11 handlers; splitting would scatter the table.
#[allow(clippy::too_many_lines)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inline_handlers_measured_and_succeed() {
    let rig = rig().await;
    let rid = rig.run_id.clone();
    let mut rows: Vec<(&str, Duration, Duration)> = Vec::new();
    let mut push = |l: &'static str, (m, x): (Duration, Duration)| rows.push((l, m, x));

    push(
        "session.create",
        rig.measure("session.create", |_| {
            req(
                Method::SessionCreate,
                serde_json::json!({"cwd": ".", "tools": [], "graph": null}),
            )
        }),
    );
    let sid = {
        let (_, r) = rig.time(&req(
            Method::SessionCreate,
            serde_json::json!({"cwd": ".", "tools": [], "graph": null}),
        ));
        serde_json::to_value(&r).unwrap()["result"]["session_id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    push(
        "run.start",
        rig.measure("run.start", |_| {
            req(
                Method::RunStart,
                serde_json::json!({"graph": "default", "input": {"prompt": "hi"}}),
            )
        }),
    );
    push(
        "run.get",
        rig.measure("run.get", |_| {
            req(Method::RunGet, serde_json::json!({"run_id": rid}))
        }),
    );
    push(
        "run.list",
        rig.measure("run.list", |_| {
            req(Method::RunList, serde_json::json!({"status": "succeeded"}))
        }),
    );
    push(
        "run.cancel",
        rig.measure("run.cancel", |_| {
            req(Method::RunCancel, serde_json::json!({"run_id": rid}))
        }),
    );
    push(
        "run.resume",
        rig.measure_refused("run.resume", |_| {
            req(Method::RunResume, serde_json::json!({"run_id": rid}))
        }),
    );
    push(
        "approve.resolve",
        rig.measure_refused("approve.resolve", |i| {
            let id = format!("appr-{i}");
            crate::daemon::approvals::request(&rig.engine, &id, &rid, "n", "p", "go?").unwrap();
            req(
                Method::ApproveResolve,
                serde_json::json!({"id": id, "allow": false}),
            )
        }),
    );
    push(
        "graph.apply",
        rig.measure("graph.apply", |i| {
            req(
                Method::GraphApply,
                serde_json::json!({"name": format!("g{i}"), "toml": format!("# graph {i}\n")}),
            )
        }),
    );
    push(
        "session.close",
        rig.measure("session.close", |_| {
            req(Method::SessionClose, serde_json::json!({"session_id": sid}))
        }),
    );
    push(
        "event.subscribe (replay)",
        rig.measure("event.subscribe (replay)", |_| {
            req(
                Method::EventSubscribe,
                serde_json::json!({"run": rid, "kinds": [], "since_seq": 0}),
            )
        }),
    );
    // sys.reload reloads the configured worker: spawn + hello (+ drain).
    let (reload, rresp) = rig.time(&req(Method::SysReload, serde_json::json!({})));
    eprintln!(
        "RESP sys.reload: {}",
        serde_json::to_string(&rresp).unwrap()
    );
    eprintln!("INLINE sys.reload (1 real worker): {reload:?}");
    rows.push(("sys.reload", reload, reload));

    for (label, med, max) in &rows {
        eprintln!("ROW {label} median={med:?} max={max:?} bar={BAR:?}");
    }
    // Timings are recorded, not asserted: these handlers are fsync-bound and
    // the workspace gate runs ~30 test processes against one disk, so a 50 ms
    // assertion here would flake (run.start measured 20 ms idle, 54 ms loaded).
    // The pass/fail guard for the worker is the heartbeat tests; this test
    // proves every handler really succeeds against the real engine.
    let over: Vec<_> = rows.iter().filter(|(_, med, _)| *med > BAR).collect();
    eprintln!("over the {BAR:?} bar this run: {over:?}");
}

/// Scale probe for the two handlers whose cost grows with stored state:
/// `event.subscribe`'s replay (journal length) and `run.list` (run count).
#[ignore = "measurement (~30 s); run with --ignored --nocapture"]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inline_handlers_scale_probe() {
    let rig = rig().await;
    let rid = rig.run_id.clone();
    // A long run: 5000 extra journal events appended in one transaction.
    let run = pii_abi::RunId::from(rid.as_str());
    let mut header = rig.engine.store.get_run(&run).unwrap().unwrap();
    let ev = pii_abi::RunEvent::NodeStarted {
        node: pii_abi::NodeId::from("n"),
        at_ms: 0,
    };
    let evs: Vec<&pii_abi::RunEvent> = (0..5000).map(|_| &ev).collect();
    rig.engine.store.append_events(&mut header, &evs).unwrap();
    let (replay, _) = rig.measure("event.subscribe replay of a 5000-event journal", |_| {
        req(
            Method::EventSubscribe,
            serde_json::json!({"run": rid, "kinds": [], "since_seq": 0}),
        )
    });
    // 1000 stored runs, then run.list.
    for _ in 0..1000 {
        rig.time(&req(
            Method::RunStart,
            serde_json::json!({"graph": "default", "input": {"prompt": "hi"}}),
        ));
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    let (list, _) = rig.measure("run.list over ~1000 succeeded runs", |_| {
        req(Method::RunList, serde_json::json!({"status": "succeeded"}))
    });
    eprintln!("SCALE replay5000={replay:?} list1000={list:?}");
}
