//! Spec 186: a resume that finds a `post` whose key was journaled and never
//! confirmed refuses with `effect.unknown` and does not post.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use pii_abi::{NodeId, PiiError, RunEvent, RunHeader, RunStatus};
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

const POST: &str = r#"name = "post-once"
entry = "p"
caps = ["post"]

[[node]]
id = "p"
kind = "post"
channel = "discord"
to = "t1"
text = "hello"
"#;

fn engine_in(dir: &std::path::Path) -> Arc<Engine> {
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    Engine::new(EngineConfig {
        workspace: dir.to_path_buf(),
        store_path: dir.join("store.redb"),
        model: format!(
            "scripted:{}",
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/phase2/hello.json")
                .display()
        ),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        graph_dir: dir.join("graphs"),
    })
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn unconfirmed_effect_refuses() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "unconfirmed-").unwrap();
    let engine = engine_in(tmp.path());
    let spec = pii_graph::load_toml_with_registry(POST, &engine.reg).unwrap();
    let store = &engine.store;
    let graph = store.put_graph(&spec.to_json().unwrap()).unwrap();
    let input_hash = store.put_graph(b"null").unwrap();
    let id = store.next_run_id().unwrap();
    let mut header = RunHeader {
        run: id.clone(),
        graph,
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 0,
        created_ms: 0,
    };
    for ev in [
        RunEvent::Started {
            graph: spec.name,
            input_hash,
        },
        RunEvent::NodeStarted {
            node: NodeId::from("p"),
            at_ms: 0,
        },
        RunEvent::EffectStarted {
            node: NodeId::from("p"),
            key: "k1".into(),
        },
    ] {
        store.append_event(&mut header, &ev).unwrap();
    }
    let before = store.read_journal(&id).unwrap();

    let err = pii_server::engine::runs::spawn_resume(Arc::clone(&engine), id.clone(), None)
        .expect_err("an unconfirmed effect must not resume");
    assert!(
        matches!(&err, PiiError::Invalid(m) if m.contains("effect.unknown") && m.contains('p')),
        "{err:?}"
    );
    assert_eq!(store.read_journal(&id).unwrap(), before, "nothing appended");
    assert_eq!(
        store.get_run(&id).unwrap().unwrap().status,
        RunStatus::Running,
        "the run is left for the operator (retry or abandon)"
    );
}
