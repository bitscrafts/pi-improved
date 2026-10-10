//! Spec 129: the local model kind (`llamacpp-server:`) and the retired `ollama:` refusal.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::observed_provider;
use std::sync::Arc;

fn log() -> (Arc<pii_observe::SwitchObserver>, Arc<pii_observe::Recorder>) {
    let log = Arc::new(pii_observe::SwitchObserver::new());
    let rec = Arc::new(pii_observe::Recorder::new());
    log.attach(rec.clone());
    (log, rec)
}

/// Spec 129 R1: `ollama:` is refused, names `llamacpp-server:`, builds no
/// provider and is counted/emitted.
#[test]
fn an_ollama_model_is_refused_pointing_at_llamacpp_server() {
    let (log, rec) = log();
    let err = match observed_provider("ollama:granite4.2:3b", &log, None) {
        Ok(_) => panic!("ollama: must be refused, no provider built"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("llamacpp-server:"), "{err}");
    assert_eq!(
        rec.events_named(pii_observe::names::MODEL_OLLAMA_REFUSED)
            .len(),
        1,
        "the refusal is emitted"
    );
}

/// Spec 129 R2: a `llamacpp-server:` model builds a provider whose egress
/// scope is exactly the configured host (offline: nothing is dialled).
#[test]
fn a_llamacpp_server_model_resolves_to_the_configured_host() {
    let (log, _rec) = log();
    let (provider, host) = observed_provider(
        "llamacpp-server:granite-4.2-3b",
        &log,
        Some("http://10.10.10.217:8082/v1"),
    )
    .unwrap();
    assert_eq!(host.as_deref(), Some("10.10.10.217"));
    assert_eq!(provider.name(), "openai");
    // Unset: loopback, never an external host.
    let (_p, host) = observed_provider("llamacpp-server:m", &log, None).unwrap();
    assert_eq!(host.as_deref(), Some("127.0.0.1"));
}

/// Spec 129: the in-process GGUF kind stays host-less and transport-less —
/// no `NetService` scope is produced for it.
#[test]
fn the_in_process_gguf_kind_is_unchanged() {
    let (log, _rec) = log();
    // Feature off: refused as "not compiled in"; feature on: missing file.
    // Either way it never reaches the HTTP path, so it names no host.
    let err = match observed_provider("llamacpp:/nonexistent/m.gguf", &log, Some("http://h:1/v1")) {
        Ok(_) => panic!("a missing GGUF must not build"),
        Err(e) => e.to_string(),
    };
    assert!(!err.contains("llamacpp-server"), "{err}");
    assert_eq!(pii_model::egress_host("llamacpp:/nonexistent/m.gguf"), None);
}
