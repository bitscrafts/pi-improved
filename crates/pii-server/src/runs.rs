//! Run registry (F2.3): spawn on `spawn_blocking`, return immediately
//! (< 50 ms, N4). The journal→bus bridge rides the run context's observer.

use std::sync::Arc;

use pii_abi::{PiiError, RunId};
use pii_graph::spec::GraphSpec;

use crate::Engine;
use crate::bus::BridgeObserver;

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
    // B6 (opus review): a graph's declared `[limits] max_tokens` used to be
    // parsed and then discarded — every run got `Budget::unlimited()`
    // regardless. `Budget::with_tokens` charges the SAME `Ctx::budget()`
    // the `agent` node already calls, so a graph that declares
    // `max_tokens = N` now actually enforces N.
    let ctx = pii_core::Ctx::with_observer_and_cancel(
        run_caps(
            &engine.cfg.workspace,
            scope_root.as_deref(),
            engine.ollama_net_host.as_deref(),
        ),
        pii_core::Budget::with_tokens(spec.limits.max_tokens),
        Arc::new(bridge),
        pii_core::TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
        cancel,
    );
    tokio::task::spawn_blocking(move || {
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
        // The Phase-1 `run` journals every event (bridge → bus) and writes
        // the terminal to the journal; nothing else writes redb.
        let terminal =
            match pii_graph::run::run(&engine.store, &spec, &engine.reg, &ctx, &run_id, input) {
                Ok(t) => Some(t),
                Err(e) => {
                    // No tracing dep wired yet (Phase-2-scale addition); stderr is the
                    // only channel for a run failure that predates journal write.
                    #[allow(clippy::print_stderr)]
                    {
                        eprintln!("pii: run {run_id} failed: {e}");
                    }
                    None
                }
            };
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

/// Continues a suspended run (F5.5/F5.9/F5.10: timer fire, approval resolve).
///
/// Runs on a blocking thread and returns at once, mirroring [`spawn_run`]'s
/// shape so a resume looks identical to a fresh run to every subscriber on
/// the bus.
///
/// # Errors
/// None currently; kept `Result` for symmetry with [`spawn_run`].
pub fn spawn_resume(engine: Arc<Engine>, run_id: RunId) -> Result<(), PiiError> {
    let cancel = pii_core::CancelToken::new();
    engine.register_run(&run_id, cancel.clone());
    let bridge = BridgeObserver {
        run_id: run_id.clone(),
        tx: engine.bus.subscribe_tx(),
    };
    // B6 (opus review): recover the graph's declared `max_tokens` the same
    // way `pii_graph::run::resume` does internally (header → graph body →
    // spec), so a resumed run enforces the same budget a fresh run would
    // rather than silently reverting to unlimited. Best-effort: if the
    // lookup fails here, `resume()` below will hit the same error and
    // report it properly — this path only affects which budget we start
    // with, not correctness of the resume itself.
    let max_tokens = engine
        .store
        .get_run(&run_id)
        .ok()
        .flatten()
        .and_then(|header| engine.store.get_graph(&header.graph).ok().flatten())
        .and_then(|bytes| GraphSpec::from_json(&bytes).ok())
        .map_or(u64::MAX, |spec| spec.limits.max_tokens);
    // R1 (spec 20, boss prerequisite): a run that STARTED with a confined
    // `scope_root` must RESUME with the SAME confinement — otherwise a scoped
    // heal/promotion run resumes workspace-wide fs_write, voiding the "human
    // approves promotions" contract. Recover it from the run's journal input
    // (the same source `pii_graph::run::resume` replays from), not a header.
    let scope_root = stored_run_scope_root(&engine.store, &run_id).ok().flatten();
    // R4 (spec 26): re-validate the recovered root. `spawn_run` validates on
    // start; `spawn_resume` must too, or a root that has since drifted outside
    // the workspace (or a path that no longer resolves) would resume a run with
    // confinement it was never granted. A stale/invalid root is a loud resume
    // failure, never a silently-widened scope.
    if let Some(root) = scope_root.as_deref() {
        validate_scope_root(&engine.cfg.workspace, root).map_err(|e| {
            PiiError::Invalid(format!(
                "resume {run_id}: refused (invalid scope_root): {e}"
            ))
        })?;
    }
    let ctx = pii_core::Ctx::with_observer_and_cancel(
        run_caps(
            &engine.cfg.workspace,
            scope_root.as_deref(),
            engine.ollama_net_host.as_deref(),
        ),
        pii_core::Budget::with_tokens(max_tokens),
        Arc::new(bridge),
        pii_core::TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
        cancel,
    );
    tokio::task::spawn_blocking(move || {
        let terminal = match pii_graph::run::resume(&engine.store, &engine.reg, &ctx, &run_id) {
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

/// Recovers a run's declared `scope_root` from its journal (spec 20 R1).
/// The run input (including `scope_root`) is content-addressed in the
/// `RunEvent::Started { input_hash }`; this reads it back so a RESUME can
/// restore the SAME write confinement the run started with. `Ok(None)` when
/// the run never had a `scope_root` (or the journal is unreadable — a bare
/// resume degrades to workspace-scoped, the pre-fix behavior, never wider).
fn stored_run_scope_root(
    store: &pii_store::Store,
    run_id: &RunId,
) -> Result<Option<std::path::PathBuf>, PiiError> {
    let journal = store.read_journal(run_id)?;
    Ok(journal.iter().find_map(|(_, ev)| match ev {
        pii_abi::RunEvent::Started { input_hash, .. } => store
            .get_graph(input_hash)
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|v| {
                v.get("scope_root")
                    .and_then(serde_json::Value::as_str)
                    .map(std::path::PathBuf::from)
            }),
        _ => None,
    }))
}

/// Root capabilities for a server-side run: fs read everything (or the
/// workspace for scoped runs), fs write scoped to the workspace — or to
/// `scope_root` alone when one is given (S1.2 heal confinement) — and the
/// `bash` tool's `/bin/sh` argv0 granted (F1, opus review). `net_host`
/// (2026-09-05 follow-up to B1) grants exactly `CapKind::NET` scoped to
/// that one host when the engine is running a live `ollama:` provider —
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
    caps.kinds = pii_core::CapKind::FS_READ | pii_core::CapKind::FS_WRITE | pii_core::CapKind::PROC;
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
    caps.proc_argv0 = pii_core::StringScope::new(vec![String::from("/bin/sh")]);
    if let Some(host) = net_host {
        caps.net_hosts = pii_core::StringScope::new(vec![host.to_owned()]);
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
    use tempfile::Builder;

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
        let dir = Builder::new()
            .prefix("heal-scope-")
            .tempdir_in("/projects/pi-improved/_tmp")
            .unwrap();
        let store = Store::open(&dir.path().join("store.redb")).unwrap();
        let run = RunId::from("r-scoped");
        // Put the run input (with scope_root) as a content-addressed graph.
        let input = serde_json::json!({ "prompt": "fix", "scope_root": "/projects/pi-improved/_tmp/heal-wt-1" });
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
            Some(std::path::PathBuf::from(
                "/projects/pi-improved/_tmp/heal-wt-1"
            )),
            "resume must recover the original scope_root"
        );
    }

    /// R1 (spec 20): a run that never had a `scope_root` recovers `None` (a
    /// bare resume stays workspace-scoped, never wider).
    #[test]
    fn stored_run_scope_root_none_when_unscoped() {
        use pii_abi::{RunEvent, RunHeader, RunId, RunStatus};
        use pii_store::Store;
        let dir = Builder::new()
            .prefix("heal-scope-")
            .tempdir_in("/projects/pi-improved/_tmp")
            .unwrap();
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
}
