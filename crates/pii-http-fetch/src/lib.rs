//! `pii-http-fetch`: credential-free GET-only external fetch, routed
//! exclusively through the `pii-net` chokepoint (spec 38, Phase 5.1).
//!
//! The tool surface is deliberately tiny: `{ "url": "..." }`. Anything that
//! could smuggle credentials or mutate state — custom headers, request
//! bodies, non-GET methods, auth of any kind — is refused before the
//! chokepoint is ever touched. Every accepted fetch and every refusal is
//! appended to a deterministic audit log (`FEED: <event> <detail>` lines),
//! replayable from inputs alone.
//!
//! **Dormant by decision (spec 109 R5):** no crate depends on this one, so no
//! shipped `ToolSet` can reach it and its `fetch.*` events cannot fire. Wiring
//! outbound HTTP into the agent is an owner-level security decision;
//! `tests/dormant.rs` pins that nothing wires it by accident.

pub mod audit;
pub mod fetch;
mod observe;

use pii_abi::PiiError;
use pii_core::Ctx;
use pii_core::HttpTransport as _;
use pii_model::ToolSpec;
use pii_observe::Observer;
use serde_json::{Value, json};

/// A callable tool (the `pii-tools::Tool` contract, re-declared here so this
/// crate does not pull the whole tool-set tree; identical signature).
pub trait Tool: Send + Sync {
    /// Tool name (also the spec key).
    fn name(&self) -> &'static str;
    /// The `ToolSpec` exposed to the model.
    fn spec(&self) -> ToolSpec;
    /// Executes the tool.
    ///
    /// # Errors
    /// [`PiiError::Denied`] on capability/scope denial, [`PiiError::Invalid`]
    /// for bad arguments.
    fn call(&self, ctx: &Ctx, args: &Value) -> Result<Value, PiiError>;
}

pub use fetch::FetchArgs;

/// The `http_fetch` tool (spec 38): GET-only, header-free, chokepoint-routed.
pub struct HttpFetchTool {
    /// The transport seam — always a clone of the process `NetService`, so
    /// every byte passes the single chokepoint (§4.3.5).
    transport: pii_net::NetService,
    /// Grant key the fetch uses (`http:<host>`), matching `NetService::request`.
    audit: audit::AuditLog,
    observer: Option<std::sync::Arc<dyn Observer>>,
}

impl HttpFetchTool {
    /// Builds the tool over the shared chokepoint and an audit sink.
    #[must_use]
    pub fn new(transport: pii_net::NetService, audit: audit::AuditLog) -> Self {
        Self {
            transport,
            audit,
            observer: None,
        }
    }
}

impl Tool for HttpFetchTool {
    fn name(&self) -> &'static str {
        "http_fetch"
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: String::from("http_fetch"),
            description: String::from(
                "Fetch a public URL (GET only, no headers/auth/secrets). \
                 Routes through the pii-net chokepoint.",
            ),
            schema: json!({"type":"object","required":["url"],"properties":{
                "url":{"type":"string","description":"absolute http(s) URL"}}}),
        }
    }

    /// Executes one credential-free GET.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] for a non-GET method, any headers, a body, or a
    /// malformed URL; [`PiiError::Denied`] when the chokepoint refuses the
    /// target; [`PiiError::Unavailable`] when the peer fails.
    fn call(&self, ctx: &Ctx, args: &Value) -> Result<Value, PiiError> {
        let args = match FetchArgs::parse(args) {
            Ok(a) => a,
            Err(e) => {
                // Audit the refusal with whatever URL the caller offered —
                // never let a refusal go unlogged.
                let url = args.get("url").and_then(Value::as_str).unwrap_or("?");
                let _ = self.audit.append(&format!("FEED: DENIED {url} reason={e}"));
                return Err(e);
            }
        };
        let req = args.to_request();
        let outcome = match self.transport.request(ctx, &req) {
            Ok(resp) => Ok((resp.status, resp.body.len())),
            Err(e) => Err(e),
        };
        match &outcome {
            Ok((status, bytes)) => {
                let _ = self.audit.append(&format!(
                    "FEED: GET {} -> status={status} bytes_out=0 bytes_in={bytes} headers=none method=GET body=none",
                    args.url
                ));
            }
            Err(e) => {
                let _ = self
                    .audit
                    .append(&format!("FEED: DENIED {} reason={e}", args.url));
            }
        }
        self.observe_outcome(&args.url, &outcome);
        let (status, bytes_in) = outcome?;
        Ok(json!({"status": status, "bytes_in": bytes_in}))
    }
}
