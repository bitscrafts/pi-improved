//! Project-local config layered over the global `~/.pii/pii.toml` (the CLAUDE.md model: a global file, a
//! project file, and the project file wins). Global-only settings are refused in the project file, not
//! silently dropped, so a repository cannot widen the sandbox or the network grants the daemon enforces.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_server::daemon::config::Config;

const GLOBAL: &str = r#"
[core]
model = "llamacpp-server:granite-4.2-3b"
runtime_dir = "/run/user/0/pii"
"#;

#[test]
fn a_project_model_overrides_the_global_model() {
    let global = Config::parse(GLOBAL).unwrap();
    let project = Config::parse("[core]\nmodel = \"llamacpp-server:laya\"\n").unwrap();
    let effective = Config::merge_project(global, Some(project)).unwrap();
    assert_eq!(
        effective.core.model.as_deref(),
        Some("llamacpp-server:laya")
    );
}

#[test]
fn an_absent_project_file_leaves_the_global_config_unchanged() {
    let global = Config::parse(GLOBAL).unwrap();
    let effective = Config::merge_project(global, None).unwrap();
    // Config is not PartialEq (it holds non-comparable sections), so compare the fields a project can change.
    assert_eq!(
        effective.core.model.as_deref(),
        Some("llamacpp-server:granite-4.2-3b")
    );
    assert_eq!(
        effective.core.runtime_dir.as_deref(),
        Some("/run/user/0/pii")
    );
    assert_eq!(
        effective.model_tier,
        [] as [pii_server::daemon::config::ModelTierSection; 0]
    );
}

#[test]
fn a_global_only_setting_in_the_project_file_is_refused() {
    // runtime_dir decides where the daemon writes; a repository must not move it.
    let global = Config::parse(GLOBAL).unwrap();
    let project = Config::parse("[core]\nruntime_dir = \"/tmp/elsewhere\"\n").unwrap();
    let err = Config::merge_project(global, Some(project)).unwrap_err();
    assert!(
        format!("{err}").contains("runtime_dir"),
        "the refusal names the setting: {err}"
    );
}

#[test]
fn a_project_cannot_widen_the_network_grants() {
    let global = Config::parse(GLOBAL).unwrap();
    let project = Config::parse("[net]\ngrants = [\"evil.example:443\"]\n").unwrap();
    let err = Config::merge_project(global, Some(project)).unwrap_err();
    assert!(
        format!("{err}").contains("net"),
        "the refusal names [net]: {err}"
    );
}

#[test]
fn an_unknown_key_in_the_project_file_is_an_error() {
    // Parsing is strict (deny_unknown_fields), so a typo in a project file fails, it is not ignored.
    assert!(Config::parse("[core]\nmodle = \"x\"\n").is_err());
}

#[test]
fn a_model_tier_in_the_project_file_is_taken_as_written() {
    // Tiers are routing, so a project may define its own; the project replaces the list, it does not merge.
    let global =
        Config::parse("[[model_tier]]\nname = \"fast\"\nmodel = \"llamacpp-server:a\"\n").unwrap();
    let project =
        Config::parse("[[model_tier]]\nname = \"fast\"\nmodel = \"llamacpp-server:b\"\n").unwrap();
    let effective = Config::merge_project(global, Some(project)).unwrap();
    assert_eq!(
        effective.model_tier.len(),
        1,
        "the project list replaces the global list"
    );
    assert_eq!(effective.model_tier[0].model, "llamacpp-server:b");
}
