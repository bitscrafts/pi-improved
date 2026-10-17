//! Run registry (F2.3): spawn on `spawn_blocking`, return immediately
//! (< 50 ms, N4). The journal→bus bridge rides the run context's observer.

use std::sync::Arc;

use pii_abi::{PiiError, ResumeKey, RunId};
use pii_graph::spec::GraphSpec;

use crate::Engine;
use crate::engine::bus::BridgeObserver;

/// Spawns the graph on a blocking thread with a bus-bridging context and
/// returns at once (F2.3).
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
    engine.register_run(&run_id, cancel.clone());
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
        validate_scope_root(&engine.cfg.workspace, root)
    })?;
    // B6 (opus review): a graph's declared `[limits]` used to be parsed and
    // then discarded — every run got `Budget::unlimited()` regardless.
    // `Budget::new` charges the SAME `Ctx::budget()` the `agent` node
    // already calls, so `max_tokens`, `max_tool_calls`, `max_micro_usd`
    // and `max_wall_ms` are now all actually enforced.
    let ctx = pii_core::Ctx::with_observer_and_cancel(
        run_caps(
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
        with_trajectory(&engine, &run_id, Some(&session_id), bridge),
        pii_core::TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
        cancel,
    );
    // Spec 115 R4: the lifecycle clock starts when the run is accepted.
    let accepted = std::time::Instant::now();
    tokio::runtime::Handle::current().spawn_blocking(move || {
        // F5: merge any extra graph input (test_command/workspace) into the
        // run input so verify-fix graphs receive their params.
        let mut input = serde_json::json!({ "prompt": prompt, "session": session_id });
        if let Some(extra) = extra_input
            && let Some(obj) = extra.as_object()
        {
            for (k, v) in obj {
                input
                    .as_object_mut()
                    .map(|m| m.insert(k.clone(), v.clone()));
            }
        }
        // R4 (spec 33): captured before `input` moves into `run()` below —
        // a heal-fix run whose loop_budget is exhausted fails STRUCTURALLY
        // inside the engine's dispatch loop (no node runs to record it), so
        // this is the one ledger write that must happen post-run.
        let heal_input = input.clone();
        // The Phase-1 `run` journals every event (bridge → bus) and writes
        // the terminal to the journal; nothing else writes redb.
        let mut refused = false;
        let started = start_or_refuse(&engine, &spec, &ctx, &run_id, input);
        let terminal = match started {
            Ok(t) => Some(t),
            Err(e) => {
                // No tracing dep wired yet (Phase-2-scale addition); stderr is the
                // only channel for a run failure that predates journal write.
                #[allow(clippy::print_stderr)]
                {
                    eprintln!("pii: run {run_id} failed: {e}");
                }
                refused = refuse_start(&engine, &run_id, &spec.name, &e);
                None
            }
        };
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
/// not left to panic inside `spawn_blocking` and crash-loop the supervisor.
fn require_runtime() -> Result<(), PiiError> {
    tokio::runtime::Handle::try_current()
        .map(drop)
        .map_err(|_| PiiError::Invalid("spawn_run called outside a Tokio runtime".into()))
}

/// Runs the graph, or refuses it by name when a promotion quarantines it (spec 125).
/// Spec 131: the refusal is a decision; it is recorded, never gated on that.
fn start_or_refuse(
    engine: &Engine,
    spec: &GraphSpec,
    ctx: &pii_core::Ctx,
    run_id: &RunId,
    input: serde_json::Value,
) -> Result<pii_abi::Terminal, PiiError> {
    quarantine_of(engine, &spec.name).map_or_else(
        || pii_graph::run::run(&engine.store, spec, &engine.reg, ctx, run_id, input),
        |(promo, reason)| {
            crate::engine::refusal::record_quarantine_refusal(
                engine,
                &promo,
                &reason,
                run_id,
                ctx.now_ms(),
            );
            Err(PiiError::Denied(reason))
        },
    )
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
        Some(pii_abi::Terminal::Success { .. }) => "succeeded",
        Some(pii_abi::Terminal::Suspended { .. }) => "suspended",
        None if refused => "refused",
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
    let header = pii_abi::RunHeader {
        run: run_id.clone(),
        graph: graph.to_owned(),
        status: pii_abi::RunStatus::Failed,
        seq: 0,
        dur_ms: 0,
    };
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
    let scope_root = stored_run_scope_root(&engine.store, &run_id)?;
    // R4 (spec 26): re-validate the recovered root. `spawn_run` validates on
    // start; `spawn_resume` must too, or a root that has since drifted outside
    // the workspace (or a path that no longer resolves) would resume a run with
    // confinement it was never granted. A stale/invalid root is a loud resume
    // failure, never a silently-widened scope.
    if let Some(root) = scope_root.as_deref() {
        validate_scope_root(&engine.cfg.workspace, root)
            .map_err(|e| refuse("invalid scope_root", &e))?;
    }
    let cancel = pii_core::CancelToken::new();
    engine.register_run(&run_id, cancel.clone());
    let bridge = BridgeObserver {
        run_id: run_id.clone(),
        tx: engine.bus.subscribe_tx(),
    };
    let ctx = pii_core::Ctx::with_observer_and_cancel(
        run_caps(
            &engine.cfg.workspace,
            scope_root.as_deref(),
            engine.local_net_host.as_deref(),
        ),
        pii_core::Budget::new(max_tokens, max_tool_calls, max_micro_usd, max_wall_ms),
        with_trajectory(&engine, &run_id, None, bridge),
        pii_core::TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
        cancel,
    );
    tokio::task::spawn_blocking(move || {
        let terminal = match pii_graph::run::resume(
            &engine.store,
            &engine.reg,
            &ctx,
            &run_id,
            resume_key.as_ref(),
        ) {
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

/// The run context's observer: the bus bridge, preceded by the trajectory
/// bridge when a sink is attached (spec 103). The trajectory is written first
/// so it is on disk before a subscriber can see the terminal and exit.
fn with_trajectory(
    engine: &Engine,
    run_id: &RunId,
    session: Option<&str>,
    bridge: BridgeObserver,
) -> Arc<dyn pii_core::Observer> {
    let mut all: Vec<Arc<dyn pii_core::Observer>> = Vec::new();
    all.extend(engine.trajectory_observer(run_id, session));
    all.extend(engine.audit_observer());
    // Spec 108: every tool call (and bash deny-list hit) reaches the log.
    if !engine.observers_detached() {
        all.push(Arc::new(pii_core::ToolCallObserver::new(engine.log())));
    }
    all.push(Arc::new(bridge));
    Arc::new(pii_core::MultiObserver::new(all))
}

/// Recovers a run's declared `scope_root` from its journal (spec 20 R1).
/// The run input (including `scope_root`) is content-addressed in the
/// `RunEvent::Started { input_hash }`; this reads it back so a RESUME can
/// restore the SAME write confinement the run started with. `Ok(None)` when
/// the run never had a `scope_root` (no recorded input). A store or parse
/// error is `Err` — the resume is refused, never degraded (spec 78).
fn stored_run_scope_root(
    store: &pii_store::Store,
    run_id: &RunId,
) -> Result<Option<std::path::PathBuf>, PiiError> {
    let journal = store.read_journal(run_id)?;
    let started = journal.iter().find_map(|(_, ev)| match ev {
        pii_abi::RunEvent::Started { input_hash, .. } => Some(input_hash),
        _ => None,
    });
    let Some(input_hash) = started else {
        return Ok(None);
    };
    // Spec 78: a store/parse error on the recorded input is NOT "no scope".
    let Some(bytes) = store.get_graph(input_hash)? else {
        return Ok(None);
    };
    let input: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| PiiError::Internal(format!("run input unparsable: {e}")))?;
    Ok(input
        .get("scope_root")
        .and_then(serde_json::Value::as_str)
        .map(std::path::PathBuf::from))
}

/// Root capabilities for a server-side run: fs read everything (or the
/// workspace for scoped runs), fs write scoped to the workspace — or to
/// `scope_root` alone when one is given (S1.2 heal confinement) — and the
/// `bash` tool's `/bin/sh` argv0 granted (F1, opus review). `net_host`
/// (2026-09-05 follow-up to B1) grants exactly `CapKind::NET` scoped to
/// that one host when the engine is running a live networked provider —
/// without this, `dial()`'s `ctx.net()` gate denies every agent-node model
/// call before it reaches the (correctly configured) `NetService`/
/// `StaticGrants` chokepoint, since a run's OWN caps never carried `NET`
/// even though the transport's egress grant was wired correctly by
/// `Engine::new`. Tools further attenuate per node via `node_capset`.
fn run_caps(
    workspace: &std::path::Path,
    scope_root: Option<&std::path::Path>,
    net_host: Option<&str>,
) -> pii_core::CapSet {
    let mut caps = pii_core::CapSet::empty();
    caps.kinds = pii_core::CapKind::FS_READ
        | pii_core::CapKind::FS_WRITE
        | pii_core::CapKind::PROC
        | pii_core::CapKind::SPAWN
        | pii_core::CapKind::MEMORY_READ
        | pii_core::CapKind::MEMORY_WRITE
        | pii_core::CapKind::POST;
    if net_host.is_some() {
        caps.kinds |= pii_core::CapKind::NET;
    }
    caps.fs_read = scope_root.map_or_else(
        || pii_core::PathScope::new(vec![std::path::PathBuf::from("/")]),
        // A scoped (heal) run reads its worktree plus the repo read-only.
        |root| pii_core::PathScope::new(vec![root.to_path_buf(), workspace.to_path_buf()]),
    );
    caps.fs_write = pii_core::PathScope::new(vec![
        scope_root.map_or_else(|| workspace.to_path_buf(), std::path::Path::to_path_buf),
    ]);
    if let Some(root) = scope_root {
        // R1 (spec 31): a scoped (heal) run's worktree is a full checkout, so
        // without a deny list `specs/`, `xtask/`, `quality-gate.sh` and every
        // `#[test]` sit inside the allow prefix above — the cheapest "fix" for
        // any model would be deleting the assertion. `PROTECTED_RELATIVE_PATHS`
        // is shared with `xtask heal-collect`'s pre-write refusal check.
        caps.fs_write = caps.fs_write.with_deny(
            pii_core::PROTECTED_RELATIVE_PATHS
                .iter()
                .map(|p| root.join(p))
                .collect(),
        );
    }
    caps.proc_argv0 = pii_core::ArgvScope::new(vec![String::from("/bin/sh")]);
    if let Some(host) = net_host {
        caps.net_hosts = pii_core::HostScope::new(vec![host.to_owned()]);
    }
    caps
}

/// Rejects a `scope_root` that is not an absolute path inside the
/// workspace — a scope root outside the workspace would let a run's
/// `fs_write` cover arbitrary directories (an escalation for any graph whose
/// input is not human-authored, e.g. channel/router events).
fn validate_scope_root(
    workspace: &std::path::Path,
    root: &std::path::Path,
) -> Result<(), PiiError> {
    if !root.is_absolute() {
        return Err(PiiError::Invalid(format!(
            "scope_root must be an absolute path: {}",
            root.display()
        )));
    }
    // Canonicalize both sides: `..` tricks and symlinks must not smuggle a
    // root outside the workspace.
    let ws = dunce::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    let canon = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    if !canon.starts_with(&ws) {
        return Err(PiiError::Invalid(format!(
            "scope_root must be inside the workspace ({}): {}",
            ws.display(),
            root.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::{run_caps, validate_scope_root};
    use pii_core::Ctx;
    use std::path::PathBuf;
    use tempfile::Builder;

    /// Spec 107 R3: a denial raised inside a run reaches the engine's audit
    /// observer; with none attached the run observer is just the bus bridge.
    #[test]
    fn a_run_denial_reaches_the_attached_audit_observer() {
        use pii_core::{ObsEvent, RecordingObserver};
        use std::sync::Arc;
        let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "audit-").unwrap();
        let engine = crate::Engine::new(crate::EngineConfig {
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
        .unwrap();
        let run = pii_abi::RunId::from("r1");
        let bridge = || crate::engine::bus::BridgeObserver {
            run_id: run.clone(),
            tx: engine.bus.subscribe_tx(),
        };
        let denial = ObsEvent::Denial {
            principal: "p".into(),
            cap: "fs_write".into(),
        };
        // Control: nothing attached, nothing recorded anywhere.
        let rec = Arc::new(RecordingObserver::new());
        super::with_trajectory(&engine, &run, None, bridge()).event(&denial);
        assert!(rec.events().is_empty());
        engine.attach_audit(rec.clone());
        super::with_trajectory(&engine, &run, None, bridge()).event(&denial);
        assert!(
            matches!(rec.events().as_slice(), [ObsEvent::Denial { cap, .. }] if cap == "fs_write")
        );
    }

    fn ctx_for(caps: pii_core::CapSet) -> Ctx {
        Ctx::with_observer(
            caps,
            pii_core::Budget::unlimited(),
            std::sync::Arc::new(pii_core::NullObserver),
            pii_core::TraceCtx {
                run: pii_abi::RunId::from("r"),
                span: 0,
            },
        )
    }

    #[test]
    fn run_caps_none_is_workspace_scoped_write() {
        let ws = Builder::new()
            .prefix("heal-caps-")
            .tempdir_in("/tmp")
            .unwrap();
        let ctx = ctx_for(run_caps(ws.path(), None, None));
        // fs_write allows workspace paths…
        let ws_spec = ws.path().join("specs/x.md");
        assert!(ctx.fs().unwrap().write(&ws_spec, b"").is_ok());
        // …and denies paths outside the workspace PathScope.
        let denied = ctx
            .fs()
            .unwrap()
            .write(std::path::Path::new("/etc/passwd"), b"");
        assert!(matches!(denied, Err(pii_abi::PiiError::Denied(_))));
    }

    #[test]
    fn run_caps_scoped_write_is_worktree_only() {
        let ws = Builder::new()
            .prefix("heal-caps-")
            .tempdir_in("/tmp")
            .unwrap();
        let wt = ws.path().join("_tmp/heal-worktrees/s1");
        std::fs::create_dir_all(&wt).unwrap();
        let ctx = ctx_for(run_caps(ws.path(), Some(&wt), None));
        // The worktree is writable…
        assert!(ctx.fs().unwrap().write(&wt.join("x.rs"), b"").is_ok());
        // …but the main repo's specs/ is NOT (the mechanical guardrail:
        // a heal agent cannot edit specs/ because its fs_write scope is
        // the worktree only).
        let spec_write = ctx
            .fs()
            .unwrap()
            .write(&ws.path().join("specs/08-x.md"), b"");
        assert!(matches!(spec_write, Err(pii_abi::PiiError::Denied(_))));
    }

    /// T1 (spec 31, acceptance): a heal run's `fs_write` denies a protected
    /// path EVEN THOUGH it is inside the `scope_root` allow prefix — a
    /// plausible-but-wrong implementation that only filters candidate paths
    /// before calling `permits` (instead of putting the entries in
    /// `PathScope`'s own deny list) would let this through.
    #[test]
    fn fs_write_denies_protected_path_in_scope() {
        let ws = Builder::new()
            .prefix("heal-caps-")
            .tempdir_in("/tmp")
            .unwrap();
        let wt = ws.path().join("_tmp/heal-worktrees/s-protected");
        std::fs::create_dir_all(wt.join("specs")).unwrap();
        let ctx = ctx_for(run_caps(ws.path(), Some(&wt), None));
        let target = wt.join("specs/foo.md");
        let err = ctx.fs().unwrap().write(&target, b"x").unwrap_err();
        assert!(matches!(err, pii_abi::PiiError::Denied(_)), "got {err:?}");
        assert!(!target.exists(), "protected write must not touch disk");
    }

    /// T3 (spec 31): the protected-path deny list must not block ordinary
    /// source writes inside the same worktree.
    #[test]
    fn fs_write_still_allows_source() {
        let ws = Builder::new()
            .prefix("heal-caps-")
            .tempdir_in("/tmp")
            .unwrap();
        let wt = ws.path().join("_tmp/heal-worktrees/s-source");
        std::fs::create_dir_all(wt.join("crates/pii-core/src")).unwrap();
        let ctx = ctx_for(run_caps(ws.path(), Some(&wt), None));
        let target = wt.join("crates/pii-core/src/x.rs");
        assert!(ctx.fs().unwrap().write(&target, b"fn x() {}").is_ok());
    }

    #[test]
    fn run_caps_grants_net_only_when_host_given() {
        let ws = Builder::new()
            .prefix("heal-caps-")
            .tempdir_in("/tmp")
            .unwrap();
        // No host: NET must not be in the capability bitset at all.
        let no_net = run_caps(ws.path(), None, None);
        assert!(!no_net.kinds.contains(pii_core::CapKind::NET));
        // A host: NET is granted, scoped to exactly that host.
        let with_net = run_caps(ws.path(), None, Some("127.0.0.1"));
        assert!(with_net.kinds.contains(pii_core::CapKind::NET));
    }

    #[test]
    fn scope_root_must_be_absolute_and_inside_workspace() {
        let ws = Builder::new()
            .prefix("heal-caps-")
            .tempdir_in("/tmp")
            .unwrap();
        assert!(validate_scope_root(ws.path(), std::path::Path::new("relative/root")).is_err());
        assert!(validate_scope_root(ws.path(), std::path::Path::new("/etc")).is_err());
        assert!(validate_scope_root(ws.path(), std::path::Path::new("/tmp")).is_err());
        // A real directory inside the workspace is accepted.
        let inside = ws.path().join("_tmp/heal-worktrees/ok");
        std::fs::create_dir_all(&inside).unwrap();
        assert!(validate_scope_root(ws.path(), &inside).is_ok());
    }

    /// R1 (spec 20): `stored_run_scope_root` recovers the `scope_root` a run
    /// STARTED with from its journal, so a RESUME restores the same write
    /// confinement (a scoped run must resume scoped).
    #[test]
    fn stored_run_scope_root_recovers_from_journal() {
        use pii_abi::{RunEvent, RunHeader, RunId, RunStatus};
        use pii_store::Store;
        let dir =
            pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "heal-scope-").unwrap();
        let store = Store::open(&dir.path().join("store.redb")).unwrap();
        let run = RunId::from("r-scoped");
        // Put the run input (with scope_root) as a content-addressed graph.
        let input = serde_json::json!({ "prompt": "fix", "scope_root": concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp/heal-wt-1") });
        let input_hash = store
            .put_graph(&serde_json::to_vec(&input).unwrap())
            .unwrap();
        let mut header = RunHeader {
            run: run.clone(),
            graph: String::from("default"),
            status: RunStatus::Running,
            seq: 0,
            dur_ms: 0,
        };
        store
            .append_event(
                &mut header,
                &RunEvent::Started {
                    graph: String::from("default"),
                    input_hash,
                },
            )
            .unwrap();
        let recovered = super::stored_run_scope_root(&store, &run).unwrap();
        assert_eq!(
            recovered,
            Some(PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/_tmp/heal-wt-1"
            ))),
            "resume must recover the original scope_root"
        );
    }

    /// R1 (spec 20): a run that never had a `scope_root` recovers `None` (a
    /// bare resume stays workspace-scoped, never wider).
    #[test]
    fn stored_run_scope_root_none_when_unscoped() {
        use pii_abi::{RunEvent, RunHeader, RunId, RunStatus};
        use pii_store::Store;
        let dir =
            pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "heal-scope-").unwrap();
        let store = Store::open(&dir.path().join("store.redb")).unwrap();
        let run = RunId::from("r-plain");
        let input = serde_json::json!({ "prompt": "hi" });
        let input_hash = store
            .put_graph(&serde_json::to_vec(&input).unwrap())
            .unwrap();
        let mut header = RunHeader {
            run: run.clone(),
            graph: String::from("default"),
            status: RunStatus::Running,
            seq: 0,
            dur_ms: 0,
        };
        store
            .append_event(
                &mut header,
                &RunEvent::Started {
                    graph: String::from("default"),
                    input_hash,
                },
            )
            .unwrap();
        assert_eq!(super::stored_run_scope_root(&store, &run).unwrap(), None);
    }
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
            workspace: dir.path().to_path_buf(),
            store_path: dir.path().join("store.redb"),
            model: format!("scripted:{}", fixture.display()),
            sandbox: pii_tools::sandbox::SandboxProfileName::Off,
            graph_dir: dir.path().join("graphs"),
            route_policy: std::collections::BTreeMap::new(),
            tiers: Vec::new(),
            session_runs_cap: crate::DEFAULT_SESSION_RUNS_CAP,
            llama_host: None,
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
