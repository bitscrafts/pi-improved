//! Spec 111 live smoke tests: one per provider kind. Every test is `#[ignore]`d
//! — they need network access and a real key, and the gate never runs them.
//!
//! Run by hand: `OPENAI_API_KEY=… cargo test -p pii-model --test live_smoke -- --ignored openai`
//! (likewise `ANTHROPIC_API_KEY`, `GEMINI_API_KEY`). The transport is the real
//! `pii-net` service with an open egress grant; the key is read from the
//! environment and never printed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use pii_core::{Budget, CapKind, CapSet, Ctx};
use pii_model::{ChatMessage, ChatRequest, Role};

fn live(spec: &str) -> String {
    let host = pii_model::egress_host(spec).expect("networked kind");
    let mut grants = pii_net::grant::StaticGrants::default();
    grants.insert(
        "*",
        &format!("http:{host}"),
        pii_net::grant::NetGrant::open_local(),
    );
    let net = pii_net::NetService::new(pii_net::NetConfig {
        resolver: Arc::new(pii_net::SystemResolver),
        grants: Arc::new(grants),
        clock: Arc::new(pii_net::SystemClock),
    });
    let provider = pii_model::resolve_with_transport(spec, Arc::new(net), None).unwrap();
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::NET;
    let req = ChatRequest {
        model: spec.split_once(':').unwrap().1.to_owned(),
        messages: vec![ChatMessage::new(
            Role::User,
            "Reply with the single word: pong",
        )],
        tools: Vec::new(),
        max_tokens: 32,
        temperature: 0.0,
        num_ctx: 0,
    };
    let r = provider
        .chat(&Ctx::root(caps, Budget::unlimited()), &req)
        .unwrap();
    assert!(!r.text.is_empty(), "empty reply");
    r.text
}

/// Needs network + `OPENAI_API_KEY` (and optionally `OPENAI_BASE_URL`); model from `PII_LIVE_OPENAI_MODEL`.
#[test]
#[ignore = "live: needs network and OPENAI_API_KEY"]
fn live_openai() {
    let m = pii_core::env_var("PII_LIVE_OPENAI_MODEL")
        .ok_or(())
        .unwrap_or_else(|()| "gpt-4o-mini".into());
    live(&format!("openai:{m}"));
}

/// Needs network + `ANTHROPIC_API_KEY`; model from `PII_LIVE_ANTHROPIC_MODEL`.
#[test]
#[ignore = "live: needs network and ANTHROPIC_API_KEY"]
fn live_anthropic() {
    let m = pii_core::env_var("PII_LIVE_ANTHROPIC_MODEL")
        .ok_or(())
        .unwrap_or_else(|()| "claude-haiku-4-5-20251001".into());
    live(&format!("anthropic:{m}"));
}

/// Needs network + `GEMINI_API_KEY`; model from `PII_LIVE_GEMINI_MODEL`.
#[test]
#[ignore = "live: needs network and GEMINI_API_KEY"]
fn live_gemini() {
    let m = pii_core::env_var("PII_LIVE_GEMINI_MODEL")
        .ok_or(())
        .unwrap_or_else(|()| "gemini-2.0-flash".into());
    live(&format!("gemini:{m}"));
}
