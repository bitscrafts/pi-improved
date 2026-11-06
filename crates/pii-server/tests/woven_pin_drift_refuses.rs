//! Spec 219 R2: a fragment edited after the host pinned it is refused by name when the run is started, and no run
//! is recorded for it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_abi::RunStatus;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

/// The shipped guard fragment, byte for byte, so the pin is the real one.
const GUARD: &str = include_str!("../../../graphs/fragments/podman-guard.toml");

const HOST: &str = r#"name = "guarded"
entry = "build"

[[attach]]
id = "guard"
fragment = "podman-guard"
at = "run.start"
pin = "PIN"

[[node]]
id = "build"
kind = "transform"
value = "built"
"#;

fn hello() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

fn engine_in(dir: &std::path::Path, fragment: &str) -> std::sync::Arc<Engine> {
    let graphs = dir.join("graphs");
    std::fs::create_dir_all(graphs.join("fragments")).unwrap();
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    let pin = pii_graph::fragment::sha256_hex(GUARD.as_bytes());
    std::fs::write(graphs.join("guarded.toml"), HOST.replace("PIN", &pin)).unwrap();
    std::fs::write(graphs.join("fragments/podman-guard.toml"), fragment).unwrap();
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
        graph_dir: graphs,
        mqtt_tool_url: None,
    })
    .expect("engine")
}

#[tokio::test]
async fn an_edited_fragment_is_refused_before_any_run_is_recorded() {
    let tmp = tempfile::tempdir().unwrap();
    let edited = format!("{GUARD}\n# edited after the host pinned it\n");
    let engine = engine_in(tmp.path(), &edited);

    let err = engine
        .start_run("guarded", &serde_json::json!({ "runtime": "docker" }))
        .expect_err("a drifted fragment must refuse the run");
    assert!(
        err.to_string().contains("fragment.pin.drift"),
        "refusal names the rule: {err}"
    );

    for status in [
        RunStatus::Running,
        RunStatus::Suspended,
        RunStatus::Succeeded,
        RunStatus::Failed,
    ] {
        let runs = engine.store.runs_by_status(status).unwrap();
        assert!(runs.is_empty(), "no run was recorded: {runs:?}");
    }
}
