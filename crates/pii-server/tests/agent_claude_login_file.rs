//! The claude arm's credential is either an env token or the login file `<HOME>/.claude/.credentials.json`. The host's
//! login is the file (`claude auth status`: loggedIn, claude.ai), so an env-only check refused the one agent that was
//! authenticated. Controls: no token and no file is still refused, an empty file is not a login, and the file under a
//! different HOME is not found.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use pii_observe::NullObserver;
use pii_server::node_kinds::nodes::agent_worker::{WorkerSpec, plan};

fn home(name: &str, file: Option<&str>) -> PathBuf {
    let h = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../_tmp")
        .join(format!("claude-home-{name}-{}", std::process::id()));
    fs::create_dir_all(h.join(".claude")).unwrap();
    if let Some(body) = file {
        fs::write(h.join(".claude/.credentials.json"), body).unwrap();
    }
    h
}

fn try_plan(env: &BTreeMap<String, String>) -> Result<(), String> {
    let spec = WorkerSpec::from_cfg(&serde_json::json!({
        "agent": "claude", "model": "haiku", "spec": "specs/x.md", "workdir": "/projects/pi-improved"
    }))
    .unwrap();
    plan(&spec, "do it", env, "direct", &NullObserver)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn env_home(h: &Path) -> BTreeMap<String, String> {
    BTreeMap::from([("HOME".to_owned(), h.display().to_string())])
}

#[test]
fn a_login_file_under_home_is_a_credential() {
    let h = home("ok", Some(r#"{"claudeAiOauth":{"accessToken":"x"}}"#));
    assert_eq!(try_plan(&env_home(&h)), Ok(()));
}

#[test]
fn no_token_and_no_file_is_still_refused() {
    let h = home("none", None);
    assert!(try_plan(&env_home(&h)).unwrap_err().contains("credentials"));
}

#[test]
fn an_empty_login_file_is_not_a_credential() {
    let h = home("empty", Some(""));
    assert!(try_plan(&env_home(&h)).is_err());
}

#[test]
fn a_file_without_an_oauth_block_is_not_a_credential() {
    let h = home("noblock", Some(r#"{"other":1}"#));
    assert!(try_plan(&env_home(&h)).is_err());
}

#[test]
fn an_unset_home_is_refused_not_guessed() {
    assert!(try_plan(&BTreeMap::new()).is_err());
}

#[test]
fn the_env_token_still_works_without_a_file() {
    let h = home("envtok", None);
    let mut env = env_home(&h);
    env.insert(
        "CLAUDE_CODE_OAUTH_TOKEN".to_owned(),
        "placeholder".to_owned(),
    );
    assert_eq!(try_plan(&env), Ok(()));
}
