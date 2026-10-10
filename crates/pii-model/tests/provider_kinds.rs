//! Spec 111: the `openai:`, `anthropic:` and `gemini:` provider kinds, offline,
//! against a recording `HttpTransport` — request shape, success, 401/429,
//! malformed body, egress denial, streaming, and both entry points.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::VecDeque;
use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::{
    FailoverReason, HttpMethod, HttpRequest, HttpResponse, PiiError, Recovery, classify_provider,
};
use pii_core::{Budget, CapSet, Ctx, HttpTransport};
use pii_model::{
    AnthropicProvider, ChatMessage, ChatRequest, GeminiProvider, ModelProvider, OpenAiProvider,
    Role, ToolCall, ToolSpec,
};
use pii_observe::{Recorder, names};

const KEY: &str = "sk-canary-0123456789abcdef";

fn ctx() -> Ctx {
    Ctx::root(CapSet::empty(), Budget::unlimited())
}

fn req() -> ChatRequest {
    ChatRequest {
        model: "m".into(),
        messages: vec![
            ChatMessage::new(Role::System, "be brief"),
            ChatMessage::new(Role::User, "hi"),
        ],
        tools: Vec::new(),
        max_tokens: 100,
        temperature: 0.5,
        num_ctx: 0,
    }
}

/// Records every request; serves queued responses; optionally chunks bodies.
struct Mock {
    seen: Mutex<Vec<HttpRequest>>,
    queue: Mutex<VecDeque<Result<HttpResponse, PiiError>>>,
    chunk: usize,
}

impl Mock {
    fn new(status: u16, body: &str) -> Arc<Self> {
        Self::with(vec![Ok(HttpResponse {
            status,
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
        })])
    }
    fn with(items: Vec<Result<HttpResponse, PiiError>>) -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            queue: Mutex::new(items.into()),
            chunk: 0,
        })
    }
    fn chunked(status: u16, body: &str, chunk: usize) -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            queue: Mutex::new(
                vec![Ok(HttpResponse {
                    status,
                    headers: Vec::new(),
                    body: body.as_bytes().to_vec(),
                })]
                .into(),
            ),
            chunk,
        })
    }
    fn first(&self) -> HttpRequest {
        self.seen.lock().first().cloned().expect("a request")
    }
    fn body_json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.first().body).unwrap()
    }
    fn header_names(&self) -> Vec<String> {
        self.first()
            .headers
            .iter()
            .map(|(k, _)| k.to_ascii_lowercase())
            .collect()
    }
    /// The VALUE of a header (case-insensitive name). Added after a mutation proved that asserting
    /// presence alone lets a wrong value pass: renaming the pinned `anthropic-version` to a bogus
    /// date left every test green.
    fn header(&self, name: &str) -> Option<String> {
        self.first()
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    }
}

impl HttpTransport for Mock {
    fn request(&self, _c: &Ctx, r: &HttpRequest) -> Result<HttpResponse, PiiError> {
        self.seen.lock().push(r.clone());
        self.queue.lock().pop_front().expect("queued response")
    }
    fn request_streaming(
        &self,
        c: &Ctx,
        r: &HttpRequest,
        on_chunk: &mut dyn FnMut(&[u8]),
    ) -> Result<HttpResponse, PiiError> {
        let resp = self.request(c, r)?;
        if self.chunk == 0 {
            on_chunk(&resp.body);
        } else {
            for part in resp.body.chunks(self.chunk) {
                on_chunk(part);
            }
        }
        Ok(resp)
    }
}

fn openai(m: &Arc<Mock>, rec: Option<&Arc<Recorder>>) -> OpenAiProvider {
    let p = OpenAiProvider::new(
        m.clone(),
        "https://api.example.com/v1/",
        "m".into(),
        Some(KEY.into()),
    );
    match rec {
        Some(r) => p.with_observer(r.clone()),
        None => p,
    }
}
fn anthropic(m: &Arc<Mock>, rec: Option<&Arc<Recorder>>) -> AnthropicProvider {
    let p = AnthropicProvider::new(m.clone(), "https://api.example.com", "m".into(), KEY.into());
    match rec {
        Some(r) => p.with_observer(r.clone()),
        None => p,
    }
}
fn gemini(m: &Arc<Mock>, rec: Option<&Arc<Recorder>>) -> GeminiProvider {
    let p = GeminiProvider::new(m.clone(), "https://api.example.com", "m".into(), KEY.into());
    match rec {
        Some(r) => p.with_observer(r.clone()),
        None => p,
    }
}

fn tool() -> ToolSpec {
    ToolSpec {
        name: "read".into(),
        description: "read a file".into(),
        schema: serde_json::json!({"type":"object","properties":{"p":{"type":"string"}}}),
    }
}

// ---------------------------------------------------------------- request shape

#[test]
fn openai_request_shape() {
    let m = Mock::new(
        200,
        r#"{"choices":[{"message":{"role":"assistant","content":"hello"}}],"usage":{"prompt_tokens":5,"completion_tokens":7}}"#,
    );
    let r = openai(&m, None).chat(&ctx(), &req()).unwrap();
    assert_eq!(
        (r.text.as_str(), r.tokens_in, r.tokens_out),
        ("hello", 5, 7)
    );
    let first = m.first();
    assert_eq!(first.method, HttpMethod::Post);
    assert_eq!(first.url, "https://api.example.com/v1/chat/completions");
    let names = m.header_names();
    assert!(names.contains(&"authorization".to_owned()), "{names:?}");
    assert!(names.contains(&"content-type".to_owned()), "{names:?}");
    assert_eq!(
        m.header("authorization").as_deref(),
        Some(format!("Bearer {KEY}").as_str()),
        "the bearer scheme and the key are part of the wire contract"
    );
    assert_eq!(
        m.body_json(),
        serde_json::json!({
            "model": "m",
            "messages": [
                {"role":"system","content":"be brief"},
                {"role":"user","content":"hi"}
            ],
            "max_tokens": 100,
            "temperature": 0.5,
            "stream": false
        })
    );
}

#[test]
fn openai_tools_and_tool_results_on_the_wire() {
    let m = Mock::new(
        200,
        r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"read","arguments":"{\"p\":\"a\"}"}}]}}]}"#,
    );
    let mut rq = req();
    rq.tools = vec![tool()];
    let mut asst = ChatMessage::new(Role::Assistant, "");
    asst.tool_calls = vec![ToolCall {
        id: "c0".into(),
        name: "read".into(),
        arguments: serde_json::json!({"p":"b"}),
    }];
    let mut res = ChatMessage::new(Role::Tool, "contents");
    res.tool_call_id = Some("c0".into());
    rq.messages.extend([asst, res]);
    let r = openai(&m, None).chat(&ctx(), &rq).unwrap();
    assert_eq!(r.tool_calls.len(), 1);
    assert_eq!(r.tool_calls[0].id, "c1");
    assert_eq!(r.tool_calls[0].arguments, serde_json::json!({"p":"a"}));
    let b = m.body_json();
    assert_eq!(b["tools"][0]["function"]["name"], "read");
    assert_eq!(
        b["messages"][2]["tool_calls"][0]["function"]["arguments"],
        "{\"p\":\"b\"}"
    );
    assert_eq!(b["messages"][3]["tool_call_id"], "c0");
}

#[test]
fn anthropic_request_shape() {
    let m = Mock::new(
        200,
        r#"{"content":[{"type":"text","text":"hello"}],"usage":{"input_tokens":3,"output_tokens":4}}"#,
    );
    let r = anthropic(&m, None).chat(&ctx(), &req()).unwrap();
    assert_eq!(
        (r.text.as_str(), r.tokens_in, r.tokens_out),
        ("hello", 3, 4)
    );
    let first = m.first();
    assert_eq!(first.method, HttpMethod::Post);
    assert_eq!(first.url, "https://api.example.com/v1/messages");
    let names = m.header_names();
    for h in ["x-api-key", "anthropic-version", "content-type"] {
        assert!(names.contains(&h.to_owned()), "{h} in {names:?}");
    }
    assert!(!names.contains(&"authorization".to_owned()));
    // The version is a protocol CONTRACT: presence is not enough (mutation-proof required).
    assert_eq!(
        m.header("anthropic-version").as_deref(),
        Some("2023-06-01"),
        "the pinned anthropic-version must be the value the API expects"
    );
    assert_eq!(m.header("x-api-key").as_deref(), Some(KEY));
    assert_eq!(
        m.body_json(),
        serde_json::json!({
            "model": "m",
            "max_tokens": 100,
            "temperature": 0.5,
            "system": "be brief",
            "messages": [{"role":"user","content":"hi"}]
        })
    );
}

#[test]
fn anthropic_tool_use_round_trip() {
    let m = Mock::new(
        200,
        r#"{"content":[{"type":"text","text":"ok"},{"type":"tool_use","id":"tu1","name":"read","input":{"p":"a"}}],"usage":{"input_tokens":1,"output_tokens":2}}"#,
    );
    let mut rq = req();
    rq.tools = vec![tool()];
    let mut res = ChatMessage::new(Role::Tool, "contents");
    res.tool_call_id = Some("tu0".into());
    rq.messages.push(res);
    let r = anthropic(&m, None).chat(&ctx(), &rq).unwrap();
    assert_eq!(r.text, "ok");
    assert_eq!(r.tool_calls[0].id, "tu1");
    assert_eq!(r.tool_calls[0].arguments, serde_json::json!({"p":"a"}));
    let b = m.body_json();
    assert_eq!(b["tools"][0]["input_schema"]["type"], "object");
    assert_eq!(b["messages"][1]["role"], "user");
    assert_eq!(b["messages"][1]["content"][0]["type"], "tool_result");
    assert_eq!(b["messages"][1]["content"][0]["tool_use_id"], "tu0");
}

#[test]
fn gemini_request_shape() {
    let m = Mock::new(
        200,
        r#"{"candidates":[{"content":{"parts":[{"text":"hello"}]}}],"usageMetadata":{"promptTokenCount":6,"candidatesTokenCount":8}}"#,
    );
    let r = gemini(&m, None).chat(&ctx(), &req()).unwrap();
    assert_eq!(
        (r.text.as_str(), r.tokens_in, r.tokens_out),
        ("hello", 6, 8)
    );
    let first = m.first();
    assert_eq!(first.method, HttpMethod::Post);
    // The key travels in a header, never in the URL (URLs are logged and echoed in errors).
    assert_eq!(
        first.url,
        "https://api.example.com/v1beta/models/m:generateContent"
    );
    assert!(!first.url.contains(KEY));
    let names = m.header_names();
    assert!(names.contains(&"x-goog-api-key".to_owned()), "{names:?}");
    assert_eq!(m.header("x-goog-api-key").as_deref(), Some(KEY));
    assert_eq!(
        m.body_json(),
        serde_json::json!({
            "contents": [{"role":"user","parts":[{"text":"hi"}]}],
            "systemInstruction": {"parts":[{"text":"be brief"}]},
            "generationConfig": {"temperature":0.5,"maxOutputTokens":100}
        })
    );
}

#[test]
fn gemini_function_call_round_trip() {
    let m = Mock::new(
        200,
        r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"read","args":{"p":"a"}}}]}}]}"#,
    );
    let mut rq = req();
    rq.tools = vec![tool()];
    let r = gemini(&m, None).chat(&ctx(), &rq).unwrap();
    assert_eq!(r.tool_calls[0].name, "read");
    assert_eq!(r.tool_calls[0].arguments, serde_json::json!({"p":"a"}));
    assert_eq!(r.tokens_in, 0, "unreported counts stay 0, never estimated");
    assert_eq!(
        m.body_json()["tools"][0]["functionDeclarations"][0]["name"],
        "read"
    );
}

// ---------------------------------------------------------------- error mapping

type Build = fn(&Arc<Mock>, Option<&Arc<Recorder>>) -> Box<dyn ModelProvider>;

fn kinds() -> Vec<(&'static str, Build)> {
    vec![
        ("openai", |m, r| Box::new(openai(m, r))),
        ("anthropic", |m, r| Box::new(anthropic(m, r))),
        ("gemini", |m, r| Box::new(gemini(m, r))),
    ]
}

#[test]
fn status_401_is_auth_and_names_an_event() {
    for (kind, build) in kinds() {
        let rec = Arc::new(Recorder::new());
        // The upstream body echoes the key: it must be scrubbed everywhere.
        let m = Mock::new(
            401,
            &format!(r#"{{"error":{{"message":"bad key {KEY}"}}}}"#),
        );
        let e = build(&m, Some(&rec)).chat(&ctx(), &req()).unwrap_err();
        let PiiError::Provider(f) = &e else {
            panic!("{kind}: {e:?}")
        };
        assert_eq!(f.status, Some(401), "{kind}");
        assert_eq!(f.provider, kind);
        assert_eq!(classify_provider(f), FailoverReason::Auth, "{kind}");
        assert!(!e.is_retryable(), "{kind}: 401 is not retryable");
        assert_eq!(rec.events_named(names::MODEL_PROVIDER_AUTH_FAILED).len(), 1);
        assert!(
            !format!("{e:?}{:?}", rec.events()).contains(KEY),
            "{kind} leaked"
        );
    }
}

#[test]
fn status_429_is_rate_limit_retryable_and_names_an_event() {
    for (kind, build) in kinds() {
        let rec = Arc::new(Recorder::new());
        let m = Mock::new(429, r#"{"error":{"message":"rate limit reached"}}"#);
        let e = build(&m, Some(&rec)).chat(&ctx(), &req()).unwrap_err();
        let PiiError::Provider(f) = &e else {
            panic!("{kind}: {e:?}")
        };
        let verdict = classify_provider(f);
        assert_eq!(verdict, FailoverReason::RateLimit, "{kind}");
        assert_eq!(verdict.recovery(), Recovery::RetrySame, "{kind}");
        assert!(e.is_retryable(), "{kind}");
        assert_eq!(
            rec.events_named(names::MODEL_PROVIDER_RATE_LIMITED).len(),
            1,
            "{kind}"
        );
    }
}

#[test]
fn malformed_body_is_invalid_and_names_an_event() {
    for (kind, build) in kinds() {
        let rec = Arc::new(Recorder::new());
        let m = Mock::new(200, "this is not json");
        let e = build(&m, Some(&rec)).chat(&ctx(), &req()).unwrap_err();
        assert!(matches!(e, PiiError::Invalid(_)), "{kind}: {e:?}");
        assert_eq!(rec.events_named(names::MODEL_PROVIDER_BAD_BODY).len(), 1);
    }
}

#[test]
fn egress_denial_is_reported_with_the_host_not_the_credential() {
    for (kind, build) in kinds() {
        let rec = Arc::new(Recorder::new());
        let m = Mock::with(vec![Err(PiiError::Denied(
            "net: egress to api.example.com not allowed".into(),
        ))]);
        let e = build(&m, Some(&rec)).chat(&ctx(), &req()).unwrap_err();
        assert!(matches!(e, PiiError::Denied(_)), "{kind}: {e:?}");
        let ev = rec.events_named(names::MODEL_PROVIDER_EGRESS_DENIED);
        assert_eq!(ev.len(), 1, "{kind}");
        let dump = format!("{ev:?}");
        assert!(dump.contains("api.example.com"), "{kind}: {dump}");
        assert!(!dump.contains(KEY));
    }
}

// ---------------------------------------------------------------- streaming

const SSE: &str = "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"hel\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n\
data: {\"choices\":[],\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":3}}\n\n\
data: [DONE]\n\n";

#[test]
fn openai_streams_sse_across_split_chunks_and_emits_partial_tokens() {
    let rec = Arc::new(Recorder::new());
    // 7-byte chunks split every line mid-JSON.
    let m = Mock::chunked(200, SSE, 7);
    let mut got = Vec::new();
    let r = openai(&m, Some(&rec))
        .chat_streaming(&ctx(), &req(), &mut |d| got.push(d.to_owned()))
        .unwrap();
    assert_eq!(got, vec!["hel", "lo"]);
    assert_eq!(
        (r.text.as_str(), r.tokens_in, r.tokens_out),
        ("hello", 2, 3)
    );
    assert_eq!(m.body_json()["stream"], true);
    assert_eq!(m.body_json()["stream_options"]["include_usage"], true);
    assert_eq!(
        rec.events_named(names::MODEL_PROVIDER_STREAM_DELTA).len(),
        2
    );
}

#[test]
fn anthropic_and_gemini_refuse_streaming_rather_than_buffer() {
    let m = Mock::new(200, "{}");
    let e = anthropic(&m, None)
        .chat_streaming(&ctx(), &req(), &mut |_| {})
        .unwrap_err();
    assert!(e.to_string().contains("streaming unsupported"), "{e}");
    let e = gemini(&m, None)
        .chat_streaming(&ctx(), &req(), &mut |_| {})
        .unwrap_err();
    assert!(e.to_string().contains("streaming unsupported"), "{e}");
    assert!(m.seen.lock().is_empty(), "no request may be sent");
}

// ---------------------------------------------------------------- entry points

#[test]
fn provider_from_spec_refuses_transportless_kinds_clearly() {
    for kind in ["openai", "anthropic", "gemini"] {
        let e = pii_model::provider_from_spec(&format!("{kind}:m"))
            .err()
            .expect("refused");
        assert!(e.to_string().contains("needs a transport"), "{kind}: {e}");
    }
}

#[test]
fn unknown_kind_message_lists_every_supported_kind() {
    let e = pii_model::provider_from_spec("bogus:x")
        .err()
        .unwrap()
        .to_string();
    for k in [
        "scripted",
        "llamacpp-server",
        "openai",
        "anthropic",
        "gemini",
        "llamacpp",
    ] {
        assert!(e.contains(k), "{k} missing from: {e}");
    }
}

#[test]
fn resolve_with_transport_reaches_every_new_kind() {
    let t: Arc<dyn HttpTransport> = Mock::new(200, "{}");
    // Keyless openai-compatible is valid (local servers); the others need a key
    // from the environment, so the injected lookup stands in for it.
    let env = |n: &str| match n {
        "ANTHROPIC_API_KEY" | "GEMINI_API_KEY" => Some(KEY.to_owned()),
        _ => None,
    };
    assert_eq!(
        pii_model::openai::provider_from_lookup("openai:m", t.clone(), &env, None)
            .unwrap()
            .name(),
        "openai"
    );
    assert_eq!(
        pii_model::anthropic::provider_from_lookup("anthropic:m", t.clone(), &env, None)
            .unwrap()
            .name(),
        "anthropic"
    );
    assert_eq!(
        pii_model::gemini::provider_from_lookup("gemini:m", t.clone(), &env, None)
            .unwrap()
            .name(),
        "gemini"
    );
    // The public dispatcher recognises the kinds (never "unknown model spec kind").
    for spec in ["openai:m", "anthropic:m", "gemini:m"] {
        if let Err(e) = pii_model::resolve_with_transport(spec, t.clone(), None) {
            assert!(
                !e.to_string().contains("unknown model spec kind"),
                "{spec}: {e}"
            );
        }
    }
}

#[test]
fn missing_key_and_empty_model_are_refused_by_name_without_values() {
    let t: Arc<dyn HttpTransport> = Mock::new(200, "{}");
    let none = |_: &str| None::<String>;
    let e = pii_model::anthropic::provider_from_lookup("anthropic:m", t.clone(), &none, None)
        .err()
        .unwrap()
        .to_string();
    assert!(e.contains("ANTHROPIC_API_KEY"), "{e}");
    let e = pii_model::gemini::provider_from_lookup("gemini:m", t.clone(), &none, None)
        .err()
        .unwrap()
        .to_string();
    assert!(e.contains("GEMINI_API_KEY"), "{e}");
    assert!(
        pii_model::openai::provider_from_lookup("openai:", t, &none, None).is_err(),
        "empty model refused"
    );
}

#[test]
fn openai_base_url_is_configurable_and_defaults_to_openai() {
    let t: Arc<dyn HttpTransport> = Mock::new(200, "{}");
    let seen = |base: Option<&'static str>| {
        let m = Mock::new(200, r#"{"choices":[{"message":{"content":"x"}}]}"#);
        let env = move |n: &str| {
            (n == "OPENAI_BASE_URL")
                .then(|| base.map(str::to_owned))
                .flatten()
        };
        let p = pii_model::openai::provider_from_lookup("openai:m", m.clone(), &env, None).unwrap();
        p.chat(&ctx(), &req()).unwrap();
        m.first().url
    };
    drop(t);
    assert_eq!(seen(None), "https://api.openai.com/v1/chat/completions");
    assert_eq!(
        seen(Some("http://127.0.0.1:8080/v1")),
        "http://127.0.0.1:8080/v1/chat/completions"
    );
}

#[test]
fn the_new_names_are_registered_in_all() {
    for n in [
        names::MODEL_PROVIDER_AUTH_FAILED,
        names::MODEL_PROVIDER_RATE_LIMITED,
        names::MODEL_PROVIDER_BAD_BODY,
        names::MODEL_PROVIDER_EGRESS_DENIED,
        names::MODEL_PROVIDER_STREAM_DELTA,
    ] {
        assert!(names::ALL.contains(&n), "{n} missing from ALL");
    }
}

#[test]
fn egress_host_is_only_defined_for_networked_kinds() {
    assert_eq!(pii_model::egress_host("scripted:/x"), None);
    assert_eq!(pii_model::egress_host("llamacpp:m"), None);
    assert_eq!(pii_model::egress_host("nonsense"), None);
    for spec in ["llamacpp-server:m", "openai:m", "anthropic:m", "gemini:m"] {
        let h = pii_model::egress_host(spec).expect(spec);
        assert!(!h.is_empty() && !h.contains(['/', ':', '@']), "{spec}: {h}");
    }
}

/// Spec 129: `llamacpp-server:<model>` is an HTTP kind — the request goes to the
/// configured `[llama] host` (offline: a mock transport, the real server is not
/// reachable from CI), and the scoped egress host is that host only.
#[test]
fn a_llamacpp_server_model_resolves_to_the_configured_host() {
    let host = "http://10.10.10.217:8082/v1";
    let m = Mock::new(200, r#"{"choices":[{"message":{"content":"x"}}]}"#);
    let p = pii_model::llamacpp_server::provider(
        "llamacpp-server:granite-4.2-3b",
        m.clone(),
        Some(host),
        None,
    )
    .unwrap();
    p.chat(&ctx(), &req()).unwrap();
    let seen = m.first();
    assert_eq!(seen.url, "http://10.10.10.217:8082/v1/chat/completions");
    assert!(
        !seen
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("authorization")),
        "a local server takes no key"
    );
    assert_eq!(
        pii_model::egress_host_with("llamacpp-server:granite-4.2-3b", Some(host)).as_deref(),
        Some("10.10.10.217")
    );
    // No configured host: loopback, never an external default.
    assert_eq!(
        pii_model::egress_host("llamacpp-server:m").as_deref(),
        Some("127.0.0.1")
    );
    // The dispatcher recognises the kind and passes the host through.
    let m2 = Mock::new(200, r#"{"choices":[{"message":{"content":"x"}}]}"#);
    let p2 =
        pii_model::resolve_with_transport("llamacpp-server:granite-4.2-3b", m2.clone(), Some(host))
            .unwrap();
    p2.chat(&ctx(), &req()).unwrap();
    assert_eq!(
        m2.first().url,
        "http://10.10.10.217:8082/v1/chat/completions"
    );
}

/// Spec 129: the in-process GGUF kind is untouched — no host, no transport, and
/// its refusal never confuses it with the server kind.
#[test]
fn the_in_process_gguf_kind_is_unchanged() {
    assert_eq!(pii_model::egress_host("llamacpp:/m/model.gguf"), None);
    assert_eq!(
        pii_model::egress_host_with("llamacpp:/m/model.gguf", Some("http://h:1/v1")),
        None
    );
    let e = pii_model::llamacpp_server::provider(
        "llamacpp:/m/model.gguf",
        Mock::new(200, "{}"),
        None,
        None,
    )
    .err()
    .unwrap()
    .to_string();
    assert!(e.contains("llamacpp-server:"), "{e}");
}
