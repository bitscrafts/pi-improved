//! A run's scope and capability set (F2.3, specs 20/31/107).
//!
//! The helpers `spawn_run`/`spawn_resume` call these while building the run's
//! [`pii_core::Ctx`], split out of `runs.rs` so each file keeps its own
//! concern. Here: the observer stack, the `scope_root` recovery/validation and
//! the root capability set. Nothing here spawns, journals or broadcasts.

use std::sync::Arc;

use pii_abi::{PiiError, RunId};

use crate::Engine;
use crate::engine::bus::BridgeObserver;

/// The run context's observer: the bus bridge, preceded by the trajectory
/// bridge when a sink is attached (spec 103). The trajectory is written first
/// so it is on disk before a subscriber can see the terminal and exit.
pub(super) fn with_trajectory(
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
pub(super) fn stored_run_scope_root(
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
pub(super) fn run_caps(
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
    // spec 207: the run admits the agent binaries; dispatch hands them only to `coder` nodes.
    // spec 207/215: the run admits the agent binaries and the criteria table's programs; dispatch hands them only to
    // `coder` nodes.
    caps.proc_argv0 = pii_core::ArgvScope::new(crate::node_kinds::nodes::coder::run_programs(
        std::path::Path::new(crate::node_kinds::nodes::coder::AGENT_BIN_DIR),
        std::path::Path::new(crate::node_kinds::nodes::coder_container::PODMAN_BIN),
        &crate::node_kinds::nodes::criteria::production(),
    ));
    if let Some(host) = net_host {
        caps.net_hosts = pii_core::HostScope::new(vec![host.to_owned()]);
    }
    caps
}

/// Rejects a `scope_root` that is not an absolute path inside the
/// workspace — a scope root outside the workspace would let a run's
/// `fs_write` cover arbitrary directories (an escalation for any graph whose
/// input is not human-authored, e.g. channel/router events).
pub(super) fn validate_scope_root(
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
            grant_ceiling: None,
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
            revision: String::new(),
            process_start_ms: 0,
            created_ms: 0,
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
            revision: String::new(),
            process_start_ms: 0,
            created_ms: 0,
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
