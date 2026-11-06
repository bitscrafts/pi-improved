//! Spec 175: the daemon must find the graphs the installer placed, not run
//! with a silently empty graph set.

#![allow(clippy::unwrap_used)]

use pii_server::resolve_graph_dir;

#[test]
fn runtime_graphs_win_when_present() {
    let t = tempfile::tempdir().unwrap();
    let rt = t.path().join("run");
    std::fs::create_dir_all(rt.join("graphs")).unwrap();
    let store = t.path().join("state/store.redb");
    std::fs::create_dir_all(t.path().join("state/graphs")).unwrap();
    assert_eq!(resolve_graph_dir(&rt, &store), rt.join("graphs"));
}

#[test]
fn falls_back_to_the_installers_state_dir() {
    let t = tempfile::tempdir().unwrap();
    let rt = t.path().join("run");
    std::fs::create_dir_all(&rt).unwrap();
    let store = t.path().join("state/store.redb");
    std::fs::create_dir_all(t.path().join("state/graphs")).unwrap();
    assert_eq!(
        resolve_graph_dir(&rt, &store),
        t.path().join("state/graphs")
    );
}

#[test]
fn nothing_installed_keeps_the_runtime_path() {
    let t = tempfile::tempdir().unwrap();
    let rt = t.path().join("run");
    let store = t.path().join("state/store.redb");
    assert_eq!(resolve_graph_dir(&rt, &store), rt.join("graphs"));
}

// Spec 176: a `[[cron]]` whose graph does not start at a timer is re-run the
// moment it finishes, so the daemon refuses it at start.
mod cron_refusal {
    use std::sync::Arc;

    use pii_server::daemon::Daemon;
    use pii_server::daemon::config::Config;
    use pii_server::{Engine, EngineConfig};
    use pii_tools::sandbox::SandboxProfileName;

    const NOT_A_TIMER: &str = "name = \"loopy\"\nentry = \"a\"\n\n[[node]]\nid = \"a\"\nkind = \"transform\"\nvalue = \"x\"\n";
    const A_TIMER: &str = "name = \"ticky\"\nentry = \"tick\"\n\n[[node]]\nid = \"tick\"\nkind = \"timer\"\ncron = \"0 9 * * *\"\n\n[[node]]\nid = \"b\"\nkind = \"transform\"\nvalue = \"x\"\n\n[[edge]]\nfrom = \"tick\"\nport = \"fired\"\nto = \"b\"\nkind = \"data\"\n";

    fn build(graph: &str, body: &str) -> Result<(), pii_core::PiiError> {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path();
        std::fs::create_dir_all(dir.join("graphs")).unwrap();
        std::fs::create_dir_all(dir.join("ws")).unwrap();
        std::fs::write(dir.join("graphs").join(format!("{graph}.toml")), body).unwrap();
        let engine = Engine::new(EngineConfig {
            grant_ceiling: None,
            workspace: dir.join("ws"),
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
            models: pii_server::daemon::empty_model_roles(),
            graph_dir: dir.join("graphs"),
            mqtt_tool_url: None,
        })
        .unwrap();
        let mut cfg = Config::parse(&format!(
            "[[cron]]\nname = \"entry-one\"\ngraph = \"{graph}\"\nschedule = \"x\"\n"
        ))
        .unwrap();
        cfg.core.durable_dir = Some(dir.join("durable").display().to_string());
        Daemon::build(
            engine,
            &cfg,
            dir,
            dir.join("pii.toml"),
            Arc::new(pii_core::SystemClock),
        )
        .map(|_| ())
    }

    #[test]
    fn a_cron_whose_graph_is_not_a_timer_is_refused_by_name() {
        let e = build("loopy", NOT_A_TIMER).unwrap_err().to_string();
        assert!(e.contains("entry-one"), "{e}");
        assert!(e.contains("timer"), "{e}");
    }

    #[test]
    fn a_cron_whose_graph_is_a_timer_is_accepted() {
        build("ticky", A_TIMER).unwrap();
    }

    #[test]
    fn the_shipped_digest_graph_is_a_timer() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let body = std::fs::read_to_string(root.join("graphs/discord-digest.toml")).unwrap();
        let e = build("discord-digest", &body);
        assert!(e.is_ok(), "{e:?}");
    }
}
