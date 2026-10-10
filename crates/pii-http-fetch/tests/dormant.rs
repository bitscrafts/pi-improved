//! Spec 109 R5: `pii-http-fetch` is DORMANT by decision — no crate depends on it,
//! so no shipped `ToolSet` can reach it and its `fetch.*` events cannot fire.
//! Giving an agent outbound HTTP is an owner-level security decision, not a
//! side effect of an observability spec. This test pins the decision: whoever
//! wires the tool must come here, delete this test, and extend the daemon-log
//! wiring tests (`pii-server/tests/log_wiring.rs`) to cover `fetch.outcome`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

#[test]
fn no_crate_depends_on_pii_http_fetch() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut dependents = Vec::new();
    for entry in std::fs::read_dir(crates).unwrap().flatten() {
        let manifest = entry.path().join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        if entry.file_name() == "pii-http-fetch" {
            continue;
        }
        if text
            .lines()
            .any(|l| l.trim_start().starts_with("pii-http-fetch"))
        {
            dependents.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    assert!(
        dependents.is_empty(),
        "pii-http-fetch is wired into {dependents:?}: it is no longer dormant — \
         delete this test and cover fetch.outcome in the daemon log wiring tests"
    );
}
