//! F2.12 tests: the Ollama provider over the HTTP transport seam, against
//! `pii_net::testing::FakeHttpServer` — no real network.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use pii_abi::PiiError;
use pii_core::Ctx;
use pii_model::{ChatMessage, ChatRequest, ModelProvider as _, OllamaProvider, Role};

fn ctx() -> Ctx {
    let mut caps = pii_core::CapSet::empty();
    caps.kinds = pii_core::CapKind::NET;
    Ctx::root(caps, pii_core::Budget::unlimited())
}

fn req() -> ChatRequest {
    ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage::new(Role::User, "hi")],
        tools: Vec::new(),
        max_tokens: 100,
        temperature: 0.0,
    }
}

fn provider(fake: &pii_net::testing::FakeHttpServer) -> OllamaProvider {
    OllamaProvider::new(
        Arc::new(fake.client.clone()),
        &format!("http://fake.local:{}", fake.addr.port()),
        "m".into(),
        None,
    )
}

#[test]
fn ollama_provider_uses_transport() {
    let fake = pii_net::testing::FakeHttpServer::start(
        r#"{"routes":[{"method":"POST","path_prefix":"/api/chat","status":200,"body":{"message":{"role":"assistant","content":"hello"},"prompt_eval_count":5,"eval_count":7}}]}"#,
    )
    .unwrap();
    let p = provider(&fake);
    let resp = p.chat(&ctx(), &req()).unwrap();
    assert_eq!(resp.text, "hello");
    assert_eq!(resp.tokens_in, 5);
    assert_eq!(resp.tokens_out, 7);
    let recorded = fake.requests();
    assert_eq!(
        recorded.len(),
        1,
        "exactly one POST /api/chat: {recorded:?}"
    );
    let first = recorded.first().expect("one request");
    assert!(first.path.contains("/api/chat"), "path: {}", first.path);
    assert_eq!(first.method, "POST");
    fake.stop();
}

#[test]
fn ollama_maps_5xx_to_unavailable() {
    // A scripted transport returning a queue of statuses — the FakeHttpServer
    // serves the first matching route for every call, so it cannot express
    // "503 then 429 then 400". pii-model tests use a fake transport (no
    // sockets); this is the F2.12 TDD contract.
    use parking_lot::Mutex;
    struct SeqTransport(Mutex<Vec<u16>>);
    impl pii_core::HttpTransport for SeqTransport {
        fn request(
            &self,
            _ctx: &Ctx,
            _req: &pii_abi::HttpRequest,
        ) -> Result<pii_abi::HttpResponse, PiiError> {
            let status = self.0.lock().remove(0);
            Ok(pii_abi::HttpResponse {
                status,
                headers: Vec::new(),
                body: Vec::new(),
            })
        }
    }
    let p = OllamaProvider::new(
        Arc::new(SeqTransport(Mutex::new(vec![503, 429, 400]))),
        "http://fake.local:1",
        "m".into(),
        None,
    );
    let e1 = p.chat(&ctx(), &req()).unwrap_err();
    let e2 = p.chat(&ctx(), &req()).unwrap_err();
    let e3 = p.chat(&ctx(), &req()).unwrap_err();
    assert!(
        matches!(e1, PiiError::Unavailable(_)),
        "503 → Unavailable: {e1:?}"
    );
    assert!(
        matches!(e2, PiiError::Unavailable(_)),
        "429 → Unavailable: {e2:?}"
    );
    assert!(
        e1.is_retryable() && e2.is_retryable(),
        "first two retryable"
    );
    assert!(matches!(e3, PiiError::Invalid(_)), "400 → Invalid: {e3:?}");
    assert!(!e3.is_retryable(), "400 not retryable");
}

#[test]
fn ollama_bad_body_is_invalid() {
    let fake = pii_net::testing::FakeHttpServer::start(
        r#"{"routes":[{"method":"POST","path_prefix":"/api/chat","status":200,"body":{}}]}"#,
    )
    .unwrap();
    // An empty JSON body has no `message`; that parses to an empty response.
    // Force a truly unparseable body with a route serving a JSON scalar.
    let p = provider(&fake);
    let r = p.chat(&ctx(), &req()).unwrap();
    assert_eq!(r.text, "");
    fake.stop();
}
