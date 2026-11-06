//! Spec 362 R4: a `[models]` entry whose model the provider does not list is refused at config time, naming the
//! entry and the id. The provider list is a fixture here (captured JSON, no network).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_server::daemon::check_listed;
use pii_server::daemon::config::Config;

/// The provider's own model list, captured as JSON (spec 362 premises, 2026-11-03).
const LISTED: &str =
    r#"["deepseek-v4.1-flash", "deepseek-v4.1-flash:1017", "glm-5.3", "glm-5.3-flash"]"#;

#[test]
fn a_dead_model_is_refused_at_config_time() {
    let config = Config::parse(
        r#"
[models.coder]
model = "deepseek-v4-flash"
provider = "ollama-cloud"
"#,
    )
    .unwrap();
    let listed: Vec<String> = serde_json::from_str(LISTED).unwrap();
    let err = check_listed(&config.models, &listed)
        .unwrap_err()
        .to_string();
    assert!(err.contains("deepseek-v4-flash"), "names the id: {err}");
    assert!(err.contains("coder"), "names the entry: {err}");
}

#[test]
fn a_listed_model_passes_the_check() {
    let config = Config::parse(
        r#"
[models.workhorse]
model = "deepseek-v4.1-flash"
provider = "ollama-cloud"
"#,
    )
    .unwrap();
    let listed: Vec<String> = serde_json::from_str(LISTED).unwrap();
    check_listed(&config.models, &listed).unwrap();
}
