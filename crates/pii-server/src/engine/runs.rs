//! Run registry (F2.3): spawn on the run pool (spec 308), return immediately
//! (< 50 ms, N4). The journal→bus bridge rides the run context's observer.

use std::sync::Arc;

use pii_abi::{PiiError, ResumeKey, RunId};
use pii_graph::spec::GraphSpec;

use crate::Engine;
use crate::engine::bus::BridgeObserver;
use crate::engine::run_pool::skip_queued;

pub use super::runs_cancel::{KILL_WAIT, cancel_run, cancel_run_with};

/// Spawns the graph on a blocking thread with a bus-bridging context and
/// returns at once (F2.3).
///
/// Spec 305 R1: the run is validated and journalled (header and `Started`)
/// before this returns, on the calling thread.
///
/// # Errors
/// None currently; kept `Result` for the F2.3 contract.
pub fn spawn_run(
    engine: Arc<Engine>,
    spec: GraphSpec,
    run_id: RunId,
    prompt: String,
    session_id: String,
    extra_input: Option<serde_json::Value>,
) -> Result<(), PiiError> {
    require_runtime()?;
    let cancel = pii_core::CancelToken::new();
    let claim = engine.register_queued_run(&run_id, cancel.clone());
    let bridge = BridgeObserver {
        run_id: run_id.clone(),
        tx: engine.bus.subscribe_tx(),
    };
    // Slice 1 (S1.2): an optional `scope_root` in the run input confines the
    // run's fs_write PathScope to a disposable worktree — the heal agent's
    // fs/edit tools then CANNOT touch anything outside it (a capability,
    // not a policy). The scope is read BEFORE the ctx is built; invalid
    // roots (outside the workspace) are rejected here, surfacing as a
    // `run.start` error rather than a silently permissive run.
    let scope_root = extra_input
        .as_ref()
        .and_then(|v| v.get("scope_root"))
        .and_then(serde_json::Value::as_str)
        .map(std::path::PathBuf::from);
    scope_root.as_deref().map_or(Ok(()), |root| {
        crate::engine::run_scope::validate_scope_root(&engine.cfg.workspace, root)
    })?;
    // B6 (opus review): a graph's declared `[limits]` used to be parsed and
    // then discarded — every run got `Budget::unlimited()` regardless.
    // `Budget::new` charges the SAME `Ctx::budget()` the `agent` node
    // already calls, so `max_tokens`, `max_tool_calls`, `max_micro_usd`
    // and `max_wall_ms` are now all actually enforced.
    let ctx = pii_core::Ctx::with_observer_and_cancel(
        crate::engine::run_scope::run_caps(
            &engine.cfg.workspace,
            scope_root.as_deref(),
            engine.local_net_host.as_deref(),
        ),
        pii_core::Budget::new(
            spec.limits.max_tokens,
            spec.limits.max_tool_calls,
            spec.limits.max_micro_usd,
            spec.limits.max_wall_ms,
        ),
        crate::engine::run_scope::with_trajectory(&engine, &run_id, Some(&session_id), bridge),
        pii_core::TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
        cancel.clone(),
    );
    // Spec 115 R4: the lifecycle clock starts when the run is accepted.
    let accepted = std::time::Instant::now();
    let input = run_input(prompt, &session_id, extra_input);
    // R4 (spec 33): captured before `input` moves into `prepare` below —
    // a heal-fix run whose loop_budget is exhausted fails STRUCTURALLY
    // inside the engine's dispatch loop (no node runs to record it), so
    // this is the one ledger write that must happen post-run.
    let heal_input = input.clone();
    // Spec 305 R1: the header and `Started` are durable before this returns, so the caller's id is
    // already in the store. A refused start is journalled `Failed` here too (spec 115 R1).
    let (started, refused) = match prepare(&engine, &spec, &ctx, &run_id, input) {
        Ok(started) => (Some(started), false),
        Err(e) => {
            report_run_failed(&run_id, &e);
            (None, refuse_start(&engine, &run_id, &spec.name, &e))
        }
    };
    let pool = Arc::clone(&engine.run_pool);
    pool.submit(move || {
        if skip_queued(&engine, &run_id, &cancel, &claim) {
            return;
        }
        let terminal = execute_or_log(&engine, &spec, &ctx, &run_id, started);
        // Spec 221: a run whose agent call was refused is journalled `Refused`, though its terminal may read success.
        let refused = refused
            || matches!(engine.store.get_run(&run_id), Ok(Some(h)) if h.status == pii_abi::RunStatus::Refused);
        engine
            .latency
            .record(outcome_word(terminal.as_ref(), refused), accepted.elapsed());
        crate::node_kinds::nodes::heal::record_loop_budget_escalation(
            &engine.heals,
            &spec.name,
            &heal_input,
            terminal.as_ref(),
            run_id.as_str(),
            ctx.now_ms(),
        );
        // F20 (opus review): record the assistant reply in the session so
        // session.state is a full snapshot, not just the user's prompts.
        if let Some(t) = terminal {
            engine.record_assistant(&session_id, &t);
        }
        engine.session_finished(&run_id.to_string());
        engine.unregister_run(&run_id.to_string());
    });
    Ok(())
}

/// Spec 178: a start from a plain thread (the cron re-arm) is refused by name,
/// not left to panic on a run thread and crash-loop the supervisor.
fn require_runtime() -> Result<(), PiiError> {
    tokio::runtime::Handle::try_current()
        .map(drop)
        .map_err(|_| PiiError::Invalid("spawn_run called outside a Tokio runtime".into()))
}

/// The run input: the prompt and session, with any extra graph input merged over them. F5: `test_command`/`workspace`
/// reach verify-fix graphs this way.
fn run_input(
    prompt: String,
    session_id: &str,
    extra_input: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut input = serde_json::Map::from_iter([
        ("prompt".to_owned(), serde_json::Value::from(prompt)),
        ("session".to_owned(), serde_json::Value::from(session_id)),
    ]);
    if let Some(serde_json::Value::Object(extra)) = extra_input {
        input.extend(extra);
    }
    serde_json::Value::Object(input)
}

/// Runs a journalled run to its terminal, or `None` when it cannot finish (logged to stderr).
fn execute_or_log(
    engine: &Engine,
    spec: &GraphSpec,
    ctx: &pii_core::Ctx,
    run_id: &RunId,
    started: Option<pii_graph::run::Started>,
) -> Option<pii_abi::Terminal> {
    let started = started?;
    // The Phase-1 `execute` journals every event (bridge → bus) and writes
    // the terminal to the journal; nothing else writes redb.
    match pii_graph::run::execute(&engine.store, spec, &engine.reg, ctx, run_id, started) {
        Ok(t) => Some(t),
        Err(e) => {
            report_run_failed(run_id, &e);
            None
        }
    }
}

/// The one place a run failure that predates the journal write is reported.
/// No tracing dep is wired yet (Phase-2-scale addition); stderr is the only
/// channel for it, and two call sites share this line instead of each
/// carrying a raw print (the observability lint budgets this file at two).
fn report_run_failed(run_id: &RunId, e: &PiiError) {
    #[allow(clippy::print_stderr)]
    {
        eprintln!("pii: run {run_id} failed: {e}");
    }
}

/// Validates and journals the run, or refuses it by name when a promotion quarantines it (spec 125).
/// Spec 131: the refusal is a decision; it is recorded, never gated on that.
/// Spec 305 R1: on `Ok` the header and `Started` are already durable.
fn prepare(
    engine: &Engine,
    spec: &GraphSpec,
    ctx: &pii_core::Ctx,
    run_id: &RunId,
    input: serde_json::Value,
) -> Result<pii_graph::run::Started, PiiError> {
    if let Some((promo, reason)) = quarantine_of(engine, &spec.name) {
        crate::engine::refusal::record_quarantine_refusal(
            engine,
            &promo,
            &reason,
            run_id,
            ctx.now_ms(),
        );
        return Err(PiiError::Denied(reason));
    }
    pii_graph::run::start(&engine.store, spec, &engine.reg, ctx, run_id, input, true)
}

/// Spec 125 R1: the named reason when a promotion in effect quarantines `graph`.
/// Read-only: expiry and outcome are advanced by the daemon's sweep, not here.
fn quarantine_of(engine: &Engine, graph: &str) -> Option<(pii_learn::Promotion, String)> {
    let ctx = engine.daemon_ctx()?;
    let promo = pii_learn::quarantining_promotion(&ctx.runtime_dir, graph, learn_now_secs())?;
    let reason = pii_learn::quarantine_refusal(&promo, graph);
    Some((promo, reason))
}

/// Wall-clock epoch seconds, for the promotion lifecycle (spec 125).
pub fn learn_now_secs() -> u64 {
    use pii_core::Clock as _;
    pii_core::SystemClock.now_ms() / 1000
}

/// The latency-histogram outcome for a run's end (spec 115 R4).
fn outcome_word(terminal: Option<&pii_abi::Terminal>, refused: bool) -> &'static str {
    match terminal {
        _ if refused => "refused",
        Some(pii_abi::Terminal::Success { .. }) => "succeeded",
        Some(pii_abi::Terminal::Suspended { .. }) => "suspended",
        _ => "failed",
    }
}

/// Spec 115 R1: a run refused before its first journal write (validation,
/// node build, store put) has already handed its id to the caller, who polls
/// `run.get` on it. Journal a terminal `Failed` carrying the reason so the id
/// resolves instead of answering `NotFound` forever, tell the bus subscribers,
/// and emit [`pii_observe::names::RUN_START_REFUSED`]. A run that DID start
/// (header present) is left to the engine's own terminal handling.
fn refuse_start(engine: &Engine, run_id: &RunId, graph: &str, err: &PiiError) -> bool {
    if !matches!(engine.store.get_run(run_id), Ok(None)) {
        return false;
    }
    let mut header = pii_abi::RunHeader {
        run: run_id.clone(),
        graph: graph.to_owned(),
        status: pii_abi::RunStatus::Failed,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 0,
        created_ms: 0,
    };
    {
        use pii_core::Clock as _;
        engine
            .store
            .stamp_origin(&mut header, pii_core::SystemClock.now_ms());
    }
    fail_run(engine, header, graph, err);
    true
}

/// Ends the run `header` names as `Failed` with `err` as the reason: the
/// terminal journal event, the bus broadcast and the refusal log line. Shared
/// by a refused start and a refused resume, so both reach the same terminal.
fn fail_run(engine: &Engine, mut header: pii_abi::RunHeader, graph: &str, err: &PiiError) {
    let run_id = header.run.clone();
    header.status = pii_abi::RunStatus::Failed;
    let terminal = pii_abi::Terminal::Failure {
        error: err.to_string(),
    };
    let finished = pii_abi::RunEvent::Finished {
        terminal,
        output_hash: String::new(),
    };
    if let Err(e) = engine.store.append_event(&mut header, &finished) {
        engine.report_unhandled("run.start.refused", &e.to_string());
    } else if let Some(ev) = crate::engine::bus::map_event(&run_id, &finished) {
        engine.bus.broadcast(ev);
    }
    engine.log().event(
        &pii_observe::Event::new(
            "engine",
            pii_observe::Level::Error,
            pii_observe::names::RUN_START_REFUSED,
        )
        .field("run", run_id.as_str())
        .field("graph", graph)
        .field("error", err.to_string())
        .in_run(pii_observe::RunId(run_id.to_string())),
    );
}

/// Closes a recorded run as `Failed` with the operator's `reason` (spec 186).
///
/// Appended to the journal, broadcast on the bus, visible to `run.list` beside
/// the successful runs. A run still in flight in this process is refused.
///
/// # Errors
/// [`PiiError::NotFound`] for an unknown run, [`PiiError::Invalid`] for one in
/// flight, store errors from journaling.
pub fn abandon_run(engine: &Engine, run_id: &RunId, reason: &str) -> Result<(), PiiError> {
    close_failed(
        engine,
        run_id,
        format!("abandoned: {reason}"),
        "cancel it before abandoning",
    )
}

/// Appends a `Failure` terminal with `error` to a recorded run that is not in flight.
pub(super) fn close_failed(
    engine: &Engine,
    run_id: &RunId,
    error: String,
    in_flight_hint: &str,
) -> Result<(), PiiError> {
    let header = engine
        .store
        .get_run(run_id)?
        .ok_or_else(|| PiiError::NotFound(format!("run '{run_id}' not found")))?;
    if engine.is_in_flight(run_id.as_str()) {
        return Err(PiiError::Invalid(format!(
            "run '{run_id}' is in flight; {in_flight_hint}"
        )));
    }
    let mut header = header;
    header.status = pii_abi::RunStatus::Failed;
    // Spec 310 R1: a run closed from outside its process records how long it was open; a duration its own finish set stays.
    if header.dur_ms == 0 && header.created_ms != 0 {
        use pii_core::Clock as _;
        let open_ms = pii_core::SystemClock
            .now_ms()
            .saturating_sub(header.created_ms);
        header.dur_ms = u32::try_from(open_ms).unwrap_or(u32::MAX);
    }
    let terminal = pii_abi::Terminal::Failure { error };
    let output_hash = serde_json::to_vec(&terminal)
        .map(|b| pii_store::hash(&b))
        .unwrap_or_default();
    let finished = pii_abi::RunEvent::Finished {
        terminal,
        output_hash,
    };
    engine.store.append_event(&mut header, &finished)?;
    if let Some(ev) = crate::engine::bus::map_event(run_id, &finished) {
        engine.bus.broadcast(ev);
    }
    Ok(())
}

/// Continues a suspended run (F5.5/F5.9/F5.10: timer fire, approval resolve).
///
/// Runs on a blocking thread and returns at once, mirroring [`spawn_run`]'s
/// shape so a resume looks identical to a fresh run to every subscriber on
/// the bus.
///
/// `resume_key` is the specific key the caller knows just fired (e.g. the
/// scheduler's due timer wheel entry — spec 30 D1); `None` for a bare
/// resume (approval resolve, manual RPC) that has no single known key.
///
/// # Errors
/// None currently; kept `Result` for symmetry with [`spawn_run`].
pub fn spawn_resume(
    engine: Arc<Engine>,
    run_id: RunId,
    resume_key: Option<ResumeKey>,
) -> Result<(), PiiError> {
    spawn_resume_node(engine, run_id, resume_key, None)
}

/// [`spawn_resume`], or with `retry_node` set a re-run of that ONE node.
///
/// Spec 186, `pii run retry`: the operator's explicit acceptance of an
/// `effect.unknown`, so the unknown-effect refusal does not apply to it.
///
/// # Errors
/// As [`spawn_resume`].
// Kept as one function: the recoveries below are one refusal sequence (spec 78).
#[allow(clippy::too_many_lines)]
pub fn spawn_resume_node(
    engine: Arc<Engine>,
    run_id: RunId,
    resume_key: Option<ResumeKey>,
    retry_node: Option<pii_abi::NodeId>,
) -> Result<(), PiiError> {
    // Spec 78: every recovery below FAILS the resume on error (a confined
    // run whose scope or budget cannot be read must never start), and the
    // run is registered only after they all succeed so a refused resume
    // leaves no handle behind.
    //
    // B6 (opus review): recover the graph's declared limits the same way
    // `pii_graph::run::resume` does internally (header → graph body → spec),
    // so a resumed run enforces the same budget a fresh run would. An Err or
    // a missing header/graph/spec is a refusal, never an unlimited budget.
    let refuse = |what: &str, e: &dyn std::fmt::Display| {
        PiiError::Invalid(format!("resume {run_id}: refused ({what}): {e}"))
    };
    let header = engine
        .store
        .get_run(&run_id)
        .map_err(|e| refuse("run header unreadable", &e))?
        .ok_or_else(|| PiiError::NotFound(format!("run '{run_id}' not found")))?;
    // Spec 196: a finished run is never resumed (a cancelled run's timer used to re-run it).
    if matches!(
        header.status,
        pii_abi::RunStatus::Succeeded | pii_abi::RunStatus::Failed | pii_abi::RunStatus::Refused
    ) {
        let status = format!("{:?}", header.status);
        engine.log().event(
            &pii_observe::Event::new(
                "engine",
                pii_observe::Level::Warn,
                pii_observe::names::RUN_RESUME_REFUSED,
            )
            .field("run", run_id.as_str())
            .field("status", status.as_str())
            .in_run(pii_observe::RunId(run_id.to_string())),
        );
        return Err(PiiError::Invalid(format!(
            "resume {run_id}: refused (run already finished: {status})"
        )));
    }
    let graph_bytes = engine
        .store
        .get_graph(&header.graph)
        .map_err(|e| refuse("graph unreadable", &e))?
        .ok_or_else(|| refuse("graph missing", &header.graph))?;
    let spec = GraphSpec::from_json(&graph_bytes).map_err(|e| refuse("graph unparsable", &e))?;
    // Spec 148: the SAME check a start consults. A refused resume ends the run
    // `Failed` with the reason in its journal; it is neither left in flight
    // nor deleted, and no handle is registered.
    if let Some((promo, reason)) = quarantine_of(&engine, &spec.name) {
        let denied = PiiError::Denied(reason.clone());
        crate::engine::refusal::record_quarantine_refusal(
            &engine,
            &promo,
            &reason,
            &run_id,
            learn_now_secs() * 1000,
        );
        fail_run(&engine, header, &spec.name, &denied);
        return Err(denied);
    }
    // Spec 186: an effect started and never confirmed is unknown. Refuse
    // HERE, synchronously; the run is left for the operator (retry/abandon).
    if retry_node.is_none() {
        let journal = engine
            .store
            .read_journal(&run_id)
            .map_err(|e| refuse("journal unreadable", &e))?;
        if let Some(unknown) = pii_graph::effect::refusal(&pii_graph::effect::scan(&journal)) {
            return Err(unknown);
        }
    }
    let limits = spec.limits;
    let (max_tokens, max_tool_calls, max_micro_usd, max_wall_ms) = (
        limits.max_tokens,
        limits.max_tool_calls,
        limits.max_micro_usd,
        limits.max_wall_ms,
    );
    // R1 (spec 20, boss prerequisite): a run that STARTED with a confined
    // `scope_root` must RESUME with the SAME confinement — otherwise a scoped
    // heal/promotion run resumes workspace-wide fs_write, voiding the "human
    // approves promotions" contract. Recover it from the run's journal input
    // (the same source `pii_graph::run::resume` replays from), not a header.
    // A store error here propagates (spec 78); only `Ok(None)` means "no scope".
    let scope_root = crate::engine::run_scope::stored_run_scope_root(&engine.store, &run_id)?;
    // R4 (spec 26): re-validate the recovered root. `spawn_run` validates on
    // start; `spawn_resume` must too, or a root that has since drifted outside
    // the workspace (or a path that no longer resolves) would resume a run with
    // confinement it was never granted. A stale/invalid root is a loud resume
    // failure, never a silently-widened scope.
    if let Some(root) = scope_root.as_deref() {
        crate::engine::run_scope::validate_scope_root(&engine.cfg.workspace, root)
            .map_err(|e| refuse("invalid scope_root", &e))?;
    }
    let cancel = pii_core::CancelToken::new();
    let claim = engine.register_queued_run(&run_id, cancel.clone());
    let bridge = BridgeObserver {
        run_id: run_id.clone(),
        tx: engine.bus.subscribe_tx(),
    };
    let ctx = pii_core::Ctx::with_observer_and_cancel(
        crate::engine::run_scope::run_caps(
            &engine.cfg.workspace,
            scope_root.as_deref(),
            engine.local_net_host.as_deref(),
        ),
        pii_core::Budget::new(max_tokens, max_tool_calls, max_micro_usd, max_wall_ms),
        crate::engine::run_scope::with_trajectory(&engine, &run_id, None, bridge),
        pii_core::TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
        cancel.clone(),
    );
    let pool = Arc::clone(&engine.run_pool);
    pool.submit(move || {
        if skip_queued(&engine, &run_id, &cancel, &claim) {
            return;
        }
        let result = retry_node.as_ref().map_or_else(
            || {
                pii_graph::run::resume(
                    &engine.store,
                    &engine.reg,
                    &ctx,
                    &run_id,
                    resume_key.as_ref(),
                )
            },
            |node| pii_graph::run::retry(&engine.store, &engine.reg, &ctx, &run_id, node),
        );
        let terminal = match result {
            Ok(t) => Some(t),
            Err(e) => {
                #[allow(clippy::print_stderr)]
                {
                    eprintln!("pii: resume {run_id} failed: {e}");
                }
                None
            }
        };
        let _ = terminal;
        engine.session_finished(&run_id.to_string());
        engine.unregister_run(&run_id.to_string());
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use pii_core::Ctx;

    mod crash {
        use super::*;
        /// A node kind whose handler panics on purpose (spec 135).
        struct PanicFactory;
        struct PanicNode;
        impl pii_graph::node::GraphNode for PanicNode {
            fn run(
                &self,
                _ctx: &Ctx,
                _input: &pii_abi::NodeInput,
            ) -> Result<pii_abi::Outcome, pii_abi::PiiError> {
                #[allow(clippy::panic)] // the point of the test: a node that panics
                {
                    panic!("boom on purpose");
                }
            }
        }
        impl pii_graph::node::NodeFactory for PanicFactory {
            fn kind(&self) -> &'static str {
                "panics"
            }
            fn ports(
                &self,
                _cfg: &serde_json::Value,
            ) -> Result<Vec<pii_abi::PortName>, pii_abi::PiiError> {
                Ok(vec![pii_abi::PortName::from("out")])
            }
            fn required_caps(&self) -> pii_core::CapKind {
                pii_core::CapKind::empty()
            }
            fn effects(&self) -> pii_abi::Effects {
                pii_abi::Effects::PURE
            }
            fn build(
                &self,
                _cfg: &serde_json::Value,
            ) -> Result<Box<dyn pii_graph::node::GraphNode>, pii_abi::PiiError> {
                Ok(Box::new(PanicNode))
            }
        }

        /// Spec 135 R1-R5: a panicking node ends the run `Failed`, naming the node
        /// and the panic, records the crash event, and the daemon thread pool
        /// survives to serve a following run.
        #[tokio::test(flavor = "multi_thread")]
        async fn a_panicking_node_leaves_a_terminal_run() {
            use pii_core::{ObsEvent, RecordingObserver};
            // The finding's gate: a terminal state within 5 s.
            const BOUND: std::time::Duration = std::time::Duration::from_secs(5);
            let (_dir, mut engine) = fault_engine("crash-");
            std::sync::Arc::get_mut(&mut engine)
                .expect("sole owner")
                .reg
                .register(std::sync::Arc::new(PanicFactory));
            let rec = std::sync::Arc::new(RecordingObserver::new());
            engine.attach_audit(rec.clone());
            let spec = pii_graph::load_toml_with_registry(
            "name = \"crashy\"\nentry = \"bad\"\ncaps = []\n\n[[node]]\nid = \"bad\"\nkind = \"panics\"\n",
            &engine.reg,
        )
        .unwrap();
            let run = pii_abi::RunId::from("r-crash");
            super::super::spawn_run(
                std::sync::Arc::clone(&engine),
                spec,
                run.clone(),
                "go".into(),
                "s".into(),
                None,
            )
            .unwrap();
            let deadline = std::time::Instant::now() + BOUND;
            let header = loop {
                if let Ok(Some(h)) = engine.store.get_run(&run)
                    && h.status == pii_abi::RunStatus::Failed
                {
                    break h;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "run never reached Failed within {BOUND:?}"
                );
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            };
            assert_eq!(header.status, pii_abi::RunStatus::Failed);
            let journal = format!("{:?}", engine.store.read_journal(&run).unwrap());
            assert!(
                journal.contains("node 'bad' panicked: boom on purpose"),
                "{journal}"
            );
            assert!(
                rec.events().iter().any(|e| matches!(e,
                ObsEvent::NodePanicked { node, message, .. }
                    if node == "bad" && message == "boom on purpose")),
                "crash event missing: {:?}",
                rec.events()
            );
        }
    }

    /// Builds a bare engine over a fresh store in the crate `_tmp`.
    fn fault_engine(tag: &str) -> (tempfile::TempDir, std::sync::Arc<crate::Engine>) {
        let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), tag).unwrap();
        let fixture = dir.path().join("fixture.json");
        std::fs::write(&fixture, r#"{"turns":[]}"#).unwrap();
        let engine = crate::Engine::new(crate::EngineConfig {
            grant_ceiling: None,
            workspace: dir.path().to_path_buf(),
            store_path: dir.path().join("store.redb"),
            model: format!("scripted:{}", fixture.display()),
            sandbox: pii_tools::sandbox::SandboxProfileName::Off,
            graph_dir: dir.path().join("graphs"),
            route_policy: std::collections::BTreeMap::new(),
            tiers: Vec::new(),
            session_runs_cap: crate::DEFAULT_SESSION_RUNS_CAP,
            llama_host: None,
            mqtt_tool_url: None,
        })
        .unwrap();
        (dir, engine)
    }

    /// Seeds a resumable run: a content-addressed graph body, a header whose
    /// `graph` names it, and a journal `Started` event — enough for
    /// `spawn_resume` to get past the header/graph/budget recovery.
    fn seed_real_run(engine: &crate::Engine, run: &pii_abi::RunId) {
        seed_run_with_graph(engine, run, &graph_json(1_000_000));
    }

    /// Graph body with FINITE limits (spec 92 R2): never `u64::MAX`, so a
    /// resume that ignores the recovered limits is distinguishable.
    fn graph_json(max_tokens: u64) -> Vec<u8> {
        let mut spec = pii_graph::load_toml(
            "name = \"resume-probe\"\nentry = \"step\"\ncaps = []\n\n[[node]]\nid = \"step\"\nkind = \"transform\"\nvalue = \"ok\"\n",
        )
        .unwrap();
        spec.limits.max_tokens = max_tokens;
        spec.limits.max_tool_calls = 50;
        spec.limits.max_micro_usd = 5_000_000;
        serde_json::to_vec(&spec).unwrap()
    }

    fn seed_run_with_graph(engine: &crate::Engine, run: &pii_abi::RunId, graph_body: &[u8]) {
        use pii_abi::{RunEvent, RunHeader, RunStatus};
        let graph_hash = engine.store.put_graph(graph_body).unwrap();
        let input_hash = engine
            .store
            .put_graph(br#"{"prompt":"resume probe"}"#)
            .unwrap();
        let mut header = RunHeader {
            run: run.clone(),
            graph: graph_hash.clone(),
            status: RunStatus::Running,
            seq: 0,
            dur_ms: 0,
            revision: String::new(),
            process_start_ms: 0,
            created_ms: 0,
        };
        engine
            .store
            .append_event(
                &mut header,
                &RunEvent::Started {
                    graph: graph_hash,
                    input_hash,
                },
            )
            .unwrap();
    }

    /// Spec 78 R1: a store error while recovering the run's scope must fail
    /// the resume, and no run handle may be registered.
    #[tokio::test(flavor = "current_thread")]
    async fn resume_fail_closed_on_scope_store_fault() {
        let (_dir, engine) = fault_engine("resume-fail-scope-");
        let run = pii_abi::RunId::from("r-fault-scope");
        // Seed a REAL run (header + resolvable graph + journal) so the resume
        // reaches the scope step; without it the header lookup refuses first
        // and this test would pass for a reason its name does not claim.
        seed_real_run(&engine, &run);
        engine.store.set_fail_read_journal_for_test(true);
        let res = super::spawn_resume(std::sync::Arc::clone(&engine), run, None);
        let err = res.expect_err("scope-store fault must fail the resume");
        assert!(
            err.to_string().contains("injected read_journal fault"),
            "the scope-store error itself must propagate, not be swallowed into \"no scope\": {err}"
        );
        assert!(
            engine.runs.lock().is_empty(),
            "no run may be registered after a refused resume"
        );
    }

    /// Spec 78 R2 / spec 92 R1: a `get_run` (header) fault must fail the
    /// resume, never fall back to an unlimited budget.
    #[tokio::test(flavor = "current_thread")]
    async fn resume_fail_closed_on_run_header_fault() {
        let (_dir, engine) = fault_engine("resume-fail-header-");
        let run = pii_abi::RunId::from("r-fault-header");
        engine.store.set_fail_get_run_for_test(true);
        let res = super::spawn_resume(std::sync::Arc::clone(&engine), run, None);
        assert!(res.is_err(), "run-header fault must fail the resume");
        assert!(engine.runs.lock().is_empty());
    }

    /// Spec 92 R1: the GRAPH BODY step. An unreadable body must refuse the
    /// resume (distinct from the header fault above) and register no run.
    #[tokio::test(flavor = "current_thread")]
    async fn resume_fail_closed_on_graph_body_fault() {
        let (_dir, engine) = fault_engine("resume-fail-body-");
        let run = pii_abi::RunId::from("r-fault-body");
        seed_real_run(&engine, &run);
        engine.store.set_fail_get_graph_for_test(true);
        let err = super::spawn_resume(std::sync::Arc::clone(&engine), run, None)
            .expect_err("unreadable graph body must refuse the resume");
        assert!(
            err.to_string().contains("refused (graph unreadable)")
                && err.to_string().contains("injected get_graph fault"),
            "{err}"
        );
        assert!(engine.runs.lock().is_empty());
    }

    /// Spec 92 R1: an unparsable graph body refuses the resume.
    #[tokio::test(flavor = "current_thread")]
    async fn resume_fail_closed_on_graph_body_unparsable() {
        let (_dir, engine) = fault_engine("resume-fail-parse-");
        let run = pii_abi::RunId::from("r-fault-parse");
        seed_run_with_graph(&engine, &run, b"this is not a graph");
        let err = super::spawn_resume(std::sync::Arc::clone(&engine), run, None)
            .expect_err("unparsable graph body must refuse the resume");
        assert!(
            err.to_string().contains("refused (graph unparsable)"),
            "{err}"
        );
        assert!(engine.runs.lock().is_empty());
    }

    /// Spec 92 R2: the resumed run must carry the RECOVERED limits. The graph
    /// declares `max_tokens: 0`; the engine fails a run whose token budget is
    /// exhausted before dispatching anything, so a resume that forced
    /// `u64::MAX` would NOT fail with this error.
    #[tokio::test(flavor = "current_thread")]
    async fn resume_carries_recovered_finite_limits() {
        let (_dir, engine) = fault_engine("resume-limits-");
        let run = pii_abi::RunId::from("r-limits");
        seed_run_with_graph(&engine, &run, &graph_json(0));
        super::spawn_resume(std::sync::Arc::clone(&engine), run.clone(), None).unwrap();
        let mut found = false;
        for _ in 0..100 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            let journal = engine.store.read_journal(&run).unwrap();
            found = journal
                .iter()
                .any(|(_, e)| format!("{e:?}").contains("budget exhausted: Tokens"));
            if found {
                break;
            }
        }
        assert!(
            found,
            "resumed run must enforce the recovered max_tokens=0: {:?}",
            engine.store.read_journal(&run).unwrap()
        );
    }
}
