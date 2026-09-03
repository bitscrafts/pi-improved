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
) -> Result<(), PiiError> {
    let cancel = pii_core::CancelToken::new();
    engine.register_run(&run_id, cancel);
    let bridge = BridgeObserver {
        run_id: run_id.clone(),
        tx: engine.bus.subscribe_tx(),
    };
    let ctx = pii_core::Ctx::with_observer(
        root_caps(&engine.cfg.workspace),
        pii_core::Budget::unlimited(),
        Arc::new(bridge),
        pii_core::TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
    );
    tokio::task::spawn_blocking(move || {
        let input = serde_json::json!({ "prompt": prompt, "session": session_id });
        // The Phase-1 `run` journals every event (bridge → bus) and writes
        // the terminal to the journal; nothing else writes redb.
        if let Err(e) = pii_graph::run::run(&engine.store, &spec, &engine.reg, &ctx, &run_id, input)
        {
            // No tracing dep wired yet (Phase-2-scale addition); stderr is the
            // only channel for a run failure that predates journal write.
            #[allow(clippy::print_stderr)]
            {
                eprintln!("pii: run {run_id} failed: {e}");
            }
        }
        engine.session_finished(&run_id.to_string());
    });
    Ok(())
}

/// Root capabilities for a server-side run: fs read everything, fs write
/// scoped to the workspace (tools further attenuate per node), and the
/// `bash` tool's `/bin/sh` argv0 granted (F1, opus review).
fn root_caps(workspace: &std::path::Path) -> pii_core::CapSet {
    let mut caps = pii_core::CapSet::empty();
    caps.kinds = pii_core::CapKind::FS_READ | pii_core::CapKind::FS_WRITE | pii_core::CapKind::PROC;
    caps.fs_read = pii_core::PathScope::new(vec![std::path::PathBuf::from("/")]);
    caps.fs_write = pii_core::PathScope::new(vec![workspace.to_path_buf()]);
    caps.proc_argv0 = pii_core::StringScope::new(vec![String::from("/bin/sh")]);
    caps
}
