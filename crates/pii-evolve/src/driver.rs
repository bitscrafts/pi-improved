//! The boss's outer loop, converted to a thin UDS launcher (spec 27, R2′).
//!
//! The loop itself is now a real pii graph (`starter/boss-watch.toml`) run
//! on the daemon: `run_cycle` loads `state.md`, bumps `cycle_count`, sends
//! `run.start` for that graph over the daemon's control socket, records the
//! launched `run_id` back into `state.md`, and returns. `pii-evolve` spawns
//! no subprocess and shells out to nothing — the daemon (`pii-server`) owns
//! detection, gating, and the approval ledger.

use std::path::PathBuf;

use pii_core::PiiError;
use pii_observe::{Event, Level, NullObserver, Observer, names};
use pii_proto::client::Client;
use pii_proto::method::Method;
use pii_proto::run::{RunStartParams, RunStartResult};
use pii_proto::sys::PeerInfo;

use crate::state::StateDoc;

/// The default graph the boss watch loop launches.
const DEFAULT_GRAPH: &str = "starter/boss-watch";

/// Launch configuration for one boss-watch dispatch.
#[derive(Debug, Clone)]
pub struct BossLaunch {
    /// The daemon's UDS control socket (`<runtime_dir>/control.sock`).
    pub socket: PathBuf,
    /// The boss's durable state directory (`state.md` lives here).
    pub state_dir: PathBuf,
    /// The graph to launch.
    pub graph: String,
}

impl BossLaunch {
    /// A launch config for `state_dir` against `socket`, launching
    /// [`DEFAULT_GRAPH`].
    #[must_use]
    pub fn new(socket: PathBuf, state_dir: PathBuf) -> Self {
        Self {
            socket,
            state_dir,
            graph: String::from(DEFAULT_GRAPH),
        }
    }
}

/// One boss-watch run launched on the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchedRun {
    /// The daemon-minted run id.
    pub run_id: String,
    /// The cycle this launch recorded in `state.md`.
    pub cycle: u64,
}

fn client_info() -> PeerInfo {
    PeerInfo {
        name: String::from("pii-evolve"),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        build: String::from("boss"),
    }
}

/// Launches one boss-watch cycle on a running daemon.
///
/// Loads `state.md`, bumps `cycle_count`, sends `run.start` for `cfg.graph`
/// with `input: {"state_dir", "cycle"}`, records the launched run id +
/// `phase = "watching"` back into `state.md`, and returns the launched run.
///
/// # Errors
/// Returns [`PiiError`] on `state.md` I/O failure, a socket connect/
/// handshake failure, or an RPC error from the daemon.
pub fn run_cycle(cfg: &BossLaunch) -> Result<LaunchedRun, PiiError> {
    run_cycle_observed(cfg, &NullObserver)
}

/// [`run_cycle`], reporting [`names::EVOLVE_CYCLE_STARTED`], then either
/// [`names::EVOLVE_CYCLE_FINISHED`] or [`names::EVOLVE_LAUNCH_FAILED`] to
/// `observer`.
///
/// # Errors
/// As [`run_cycle`].
pub fn run_cycle_observed(
    cfg: &BossLaunch,
    observer: &dyn Observer,
) -> Result<LaunchedRun, PiiError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| PiiError::Internal(format!("tokio runtime: {e}")))?;
    let result = rt.block_on(run_cycle_async(cfg, observer));
    if let Err(e) = &result {
        observer.event(
            &Event::new("evolve", Level::Error, names::EVOLVE_LAUNCH_FAILED)
                .field("error", e.to_string()),
        );
    }
    result
}

async fn run_cycle_async(
    cfg: &BossLaunch,
    observer: &dyn Observer,
) -> Result<LaunchedRun, PiiError> {
    let mut state = StateDoc::load(&cfg.state_dir)?;
    let cycle: u64 = state
        .properties
        .get("cycle_count")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
        + 1;

    observer.event(
        &Event::new("evolve", Level::Info, names::EVOLVE_CYCLE_STARTED)
            .field("cycle", cycle)
            .field("graph", cfg.graph.as_str()),
    );
    let started = launch(cfg, cycle).await?;

    state
        .properties
        .insert(String::from("cycle_count"), cycle.to_string());
    state
        .properties
        .insert(String::from("last_run_id"), started.run_id.clone());
    state.phase = String::from("watching");
    state.save(&cfg.state_dir)?;
    observer.event(
        &Event::new("evolve", Level::Info, names::EVOLVE_CYCLE_FINISHED)
            .field("cycle", cycle)
            .field("run_id", started.run_id.as_str()),
    );

    Ok(LaunchedRun {
        run_id: started.run_id,
        cycle,
    })
}

/// Connects to the daemon and sends one `run.start`, returning its result.
async fn launch(cfg: &BossLaunch, cycle: u64) -> Result<RunStartResult, PiiError> {
    let stream = tokio::net::UnixStream::connect(&cfg.socket)
        .await
        .map_err(|e| PiiError::Unavailable(format!("connect {}: {e}", cfg.socket.display())))?;
    let (r, w) = tokio::io::split(stream);
    let (client, _hello) = Client::connect(r, w, client_info())
        .await
        .map_err(|e| PiiError::Unavailable(format!("handshake: {e}")))?;

    let params = RunStartParams {
        graph: cfg.graph.clone(),
        input: serde_json::json!({
            "state_dir": cfg.state_dir.display().to_string(),
            "cycle": cycle,
        }),
        caps: None,
    };
    let params_json = serde_json::to_value(&params)
        .map_err(|e| PiiError::Internal(format!("run.start params: {e}")))?;
    let result = client.call(Method::RunStart, params_json).await;
    client.shutdown();
    let result = result
        .map_err(|e| PiiError::Unavailable(format!("run.start: {} {}", e.code, e.message)))?;
    serde_json::from_value(result).map_err(|e| PiiError::Internal(format!("run.start result: {e}")))
}
