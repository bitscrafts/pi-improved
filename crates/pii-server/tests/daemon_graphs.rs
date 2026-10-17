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
