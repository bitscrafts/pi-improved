//! Memory tools (spec 024 N1/N2): `memory_search` / `memory_put`, dialing
//! the daemon's control socket over UDS via `pii-proto`'s `client` feature.
//!
//! These are the ONE implementation shared by every `ToolSet::builtin`
//! consumer: `pii-mcp` (a separate process reaching the daemon-owned
//! `MemStore`) and `pii-server`'s own agent loop (which, for the same
//! reason every other tool call goes through `Tool::call`, dials its own
//! socket rather than reaching into `Arc<MemStore>` directly — one code
//! path, one capability gate, one bound).

use std::path::PathBuf;

use pii_abi::PiiError;
use pii_core::{CapKind, Ctx};
use pii_model::ToolSpec;
use pii_proto::method::Method;
use pii_proto::sys::PeerInfo;
use serde_json::{Value, json};

use crate::Tool;

/// Wall-clock bound on one daemon round trip (spec 12, R3): generous for a
/// local UDS hop, tight enough that a wedged daemon does not hang a turn.
const MEMORY_RPC_TIMEOUT_MS: u64 = 10_000;

fn client_info() -> PeerInfo {
    PeerInfo {
        name: String::from("pii-tools-memory"),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        build: String::from("tool"),
    }
}

/// Resolves the control socket path: `sock_override` (tests only) when set,
/// else the normal `pii_uds::runtime_dir("default")` resolution.
///
/// # Errors
/// [`PiiError::Unavailable`] when no runtime dir can be resolved (e.g.
/// `$PII_RUNTIME_DIR` unset and no config/XDG fallback applies).
fn resolve_sock(sock_override: Option<&PathBuf>) -> Result<PathBuf, PiiError> {
    if let Some(p) = sock_override {
        return Ok(p.clone());
    }
    let runtime_dir = pii_uds::runtime_dir("default").ok_or_else(|| {
        PiiError::Unavailable(String::from("no usable runtime dir (set PII_RUNTIME_DIR)"))
    })?;
    Ok(runtime_dir.join("control.sock"))
}

/// Dials `sock`, calls `method`, and returns the raw result. Runs on a
/// dedicated OS thread with its own current-thread tokio runtime so this
/// sync [`Tool::call`] never nests inside a caller's own tokio runtime
/// (which would panic: "Cannot start a runtime from within a runtime") —
/// the same hazard the agent loop already avoids for timeout-bound tools by
/// spawning a thread (`pii-server/src/agent.rs`).
///
/// # Errors
/// [`PiiError::Unavailable`] when the daemon cannot be reached or answers
/// with an RPC error; [`PiiError::Internal`] on a local runtime/thread fault.
fn call_daemon(method: Method, params: Value, sock: PathBuf) -> Result<Value, PiiError> {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| PiiError::Internal(format!("memory tool runtime: {e}")))?;
        rt.block_on(call_daemon_async(method, params, sock))
    })
    .join()
    .map_err(|_| PiiError::Internal(String::from("memory tool thread panicked")))?
}

async fn call_daemon_async(
    method: Method,
    params: Value,
    sock: PathBuf,
) -> Result<Value, PiiError> {
    let stream = pii_uds::client_connect(&sock).await?;
    let (r, w) = tokio::io::split(stream);
    let (client, _hello) = pii_proto::client::Client::connect(r, w, client_info())
        .await
        .map_err(|e| PiiError::Unavailable(format!("daemon connect: {e}")))?;
    let result = client
        .call(method, params)
        .await
        .map_err(|e| PiiError::Unavailable(format!("{}: {}", e.code, e.message)));
    client.shutdown();
    result
}

fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, PiiError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| PiiError::Invalid(format!("{key} must be a string")))
}

/// `memory_search { query, k? }` — MCP/agent-loop surface for `memory.search`
/// (spec 024 N1, closes C1; N2, closes C2). Requires `MEMORY_READ`.
#[derive(Default)]
pub struct MemorySearchTool {
    /// Explicit control-socket path (tests only); `None` uses the normal
    /// `pii_uds::runtime_dir` resolution.
    pub sock_override: Option<PathBuf>,
}

impl Tool for MemorySearchTool {
    fn name(&self) -> &'static str {
        "memory_search"
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: String::from("memory_search"),
            description: String::from(
                "Search Pii's persistent memory store for relevant recalled facts",
            ),
            schema: json!({"type":"object","required":["query"],"properties":{
                "query":{"type":"string"},"k":{"type":"integer"}}}),
        }
    }

    fn timeout(&self) -> Option<u64> {
        Some(MEMORY_RPC_TIMEOUT_MS)
    }

    fn call(&self, ctx: &Ctx, args: &Value) -> Result<Value, PiiError> {
        if !ctx.caps().kinds.contains(CapKind::MEMORY_READ) {
            return Err(PiiError::Denied(String::from(
                "memory_search needs the memory_read capability",
            )));
        }
        let query = require_str(args, "query")?;
        let k = args.get("k").and_then(Value::as_u64).unwrap_or(6);
        let sock = resolve_sock(self.sock_override.as_ref())?;
        call_daemon(Method::MemorySearch, json!({"q": query, "k": k}), sock)
    }
}

/// `memory_put { text, layer?, kind?, concepts?, importance? }` — MCP/
/// agent-loop surface for `memory.put` (spec 024 N1, closes C1; N2, closes
/// C2). Requires `MEMORY_WRITE`.
#[derive(Default)]
pub struct MemoryPutTool {
    /// Explicit control-socket path (tests only); `None` uses the normal
    /// `pii_uds::runtime_dir` resolution.
    pub sock_override: Option<PathBuf>,
}

impl Tool for MemoryPutTool {
    fn name(&self) -> &'static str {
        "memory_put"
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: String::from("memory_put"),
            description: String::from("Store a fact in Pii's persistent memory store"),
            schema: json!({"type":"object","required":["text"],"properties":{
                "text":{"type":"string"},
                "layer":{"type":"string"},
                "kind":{"type":"string"},
                "concepts":{"type":"array","items":{"type":"string"}},
                "importance":{"type":"number"}}}),
        }
    }

    fn timeout(&self) -> Option<u64> {
        Some(MEMORY_RPC_TIMEOUT_MS)
    }

    fn call(&self, ctx: &Ctx, args: &Value) -> Result<Value, PiiError> {
        if !ctx.caps().kinds.contains(CapKind::MEMORY_WRITE) {
            return Err(PiiError::Denied(String::from(
                "memory_put needs the memory_write capability",
            )));
        }
        let text = require_str(args, "text")?;
        let mut params = json!({"text": text});
        if let Some(obj) = params.as_object_mut() {
            if let Some(l) = args.get("layer").and_then(Value::as_str) {
                obj.insert(String::from("layer"), json!(l));
            }
            if let Some(k) = args.get("kind").and_then(Value::as_str) {
                obj.insert(String::from("kind"), json!(k));
            }
            if let Some(c) = args.get("concepts") {
                obj.insert(String::from("concepts"), c.clone());
            }
            if let Some(i) = args.get("importance").and_then(Value::as_f64) {
                obj.insert(String::from("importance"), json!(i));
            }
        }
        let sock = resolve_sock(self.sock_override.as_ref())?;
        call_daemon(Method::MemoryPut, params, sock)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{MemoryPutTool, MemorySearchTool};
    use crate::Tool;
    use pii_core::{Budget, CapKind, CapSet, Ctx};

    fn ctx_with(kinds: CapKind) -> Ctx {
        let mut caps = CapSet::empty();
        caps.kinds = kinds;
        Ctx::root(caps, Budget::unlimited())
    }

    /// Spec 024 N2: the calls are bounded, like every other tool.
    #[test]
    fn memory_tools_declare_a_timeout() {
        assert_eq!(MemorySearchTool::default().timeout(), Some(10_000));
        assert_eq!(MemoryPutTool::default().timeout(), Some(10_000));
    }

    /// Spec 024 N2: `memory_search` is denied without `MEMORY_READ`, and
    /// never dials the daemon to find that out.
    #[test]
    fn memory_search_denied_without_capability() {
        let ctx = ctx_with(CapKind::empty());
        let err = MemorySearchTool::default()
            .call(&ctx, &serde_json::json!({"query": "anything"}))
            .unwrap_err();
        assert!(matches!(err, pii_abi::PiiError::Denied(_)), "{err:?}");
    }

    /// Spec 024 N2: `memory_put` is denied without `MEMORY_WRITE`.
    #[test]
    fn memory_put_denied_without_capability() {
        let ctx = ctx_with(CapKind::MEMORY_READ);
        let err = MemoryPutTool::default()
            .call(&ctx, &serde_json::json!({"text": "a fact"}))
            .unwrap_err();
        assert!(matches!(err, pii_abi::PiiError::Denied(_)), "{err:?}");
    }

    /// Argument validation happens before the capability-gated daemon dial.
    #[test]
    fn memory_search_rejects_missing_query() {
        let ctx = ctx_with(CapKind::MEMORY_READ);
        let err = MemorySearchTool::default()
            .call(&ctx, &serde_json::json!({}))
            .unwrap_err();
        assert!(matches!(err, pii_abi::PiiError::Invalid(_)), "{err:?}");
    }
}
