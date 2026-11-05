//! `pii-proto`: the wire contract between clients and the daemon.
//!
//! Wire types are duplicated from `pii-abi` deliberately: this crate must not
//! inherit the engine's dependency tree (`pii-core`, `pii-abi`, `tokio` by
//! default, `axum`). Frames are newline-delimited JSON (JSON-RPC 2.0 shape).

pub mod event;
pub mod method;
pub mod run;
pub mod session;
pub mod sys;

#[cfg(feature = "client")]
pub mod client;

use serde::{Deserialize, Serialize};

use crate::method::Method;

/// The protocol version every `sys.hello` must agree on (Phase 3: worker,
/// timer and memory methods join the wire vocabulary — a breaking change,
/// hence the bump to `pii.v2`).
pub const PROTOCOL_VERSION: &str = "pii.v2";

/// `queue.add` params (spec 352 R1): one work item, as the queue stores it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueAddParams {
    /// Stable identifier; must not already be in the queue.
    #[serde(default)]
    pub id: String,
    /// Spec path the coder is pointed at; must exist on the daemon's machine.
    #[serde(default)]
    pub spec: String,
    /// One-line title.
    #[serde(default)]
    pub title: String,
    /// Criteria commands; at least one.
    #[serde(default)]
    pub criteria: Vec<String>,
    /// Paths the item is expected to touch.
    #[serde(default)]
    pub touches: Vec<String>,
}

/// One row of `queue.list` (spec 352 R3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueRow {
    /// The item's id.
    pub id: String,
    /// `pending`, `running`, `done` or `parked`.
    pub state: String,
    /// The item's title.
    pub title: String,
    /// The item's spec path.
    pub spec: String,
    /// Spec 397 R4: how long a run holding this row has been suspended, in ms; absent when it is not suspended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspended_ms: Option<u64>,
    /// Spec 397 R4: the deadline (`deadline_ms`) the suspension is held against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
}

/// A JSON-RPC id: number or string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Id {
    /// Numeric id.
    Num(u64),
    /// String id.
    Str(String),
}

/// JSON-RPC error object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcError {
    /// Numeric error code.
    pub code: i32,
    /// Human-readable message.
    pub message: String,
    /// Optional structured data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Error codes: JSON-RPC reserved plus Pii's application range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// Malformed JSON.
    ParseError = -32700,
    /// Not a valid request object.
    InvalidRequest = -32600,
    /// Unknown method.
    MethodNotFound = -32601,
    /// Bad params.
    InvalidParams = -32602,
    /// Server internal error.
    Internal = -32603,
    /// Hello protocol skew — hard refusal, then close.
    ProtocolMismatch = 100,
    /// Not authorised (peercred or unauthenticated call) — then close.
    Unauthorized = 101,
    /// Referenced object does not exist.
    NotFound = 102,
    /// Capability denial.
    Denied = 103,
    /// Budget exhausted.
    BudgetBreach = 104,
    /// Operation cancelled.
    Cancelled = 105,
    /// Resource limit hit (e.g. subscriptions).
    Busy = 106,
}

impl From<ErrorCode> for i32 {
    fn from(c: ErrorCode) -> Self {
        c as Self
    }
}

impl RpcError {
    /// Builds an error from a code and message.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            data: None,
        }
    }

    /// Like [`RpcError::new`] with structured `data`.
    #[must_use]
    pub fn with_data(code: ErrorCode, message: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            data: Some(data),
        }
    }
}

/// A JSON-RPC request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Request id.
    pub id: Id,
    /// Method name.
    pub method: Method,
    /// Params (any JSON).
    pub params: serde_json::Value,
}

/// A JSON-RPC response: exactly one of `result`/`error` is `Some`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Id of the answered request.
    pub id: Id,
    /// Success payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Error payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    /// A success response.
    #[must_use]
    pub fn ok(id: Id, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// An error response.
    #[must_use]
    pub fn err(id: Id, error: RpcError) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// A JSON-RPC notification (no id; no response expected).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Method name.
    pub method: Method,
    /// Params.
    pub params: serde_json::Value,
}

/// Any inbound frame. Serialised untagged: the wire shape is the inner
/// struct itself (flat JSON-RPC), never a variant wrapper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Envelope {
    /// A request wanting a response.
    Request(Request),
    /// A response to a previous request.
    Response(Response),
    /// A one-way notification.
    Notification(Notification),
}

/// Classifies one newline-stripped JSON line (hand-written, per F1.2).
///
/// # Errors
/// Returns [`ErrorCode::ParseError`] for undecodable JSON,
/// [`ErrorCode::InvalidRequest`] for a bad `jsonrpc` field or an
/// unclassifiable frame, and [`ErrorCode::MethodNotFound`] for an unknown
/// method string.
pub fn parse_frame(line: &str) -> Result<Envelope, RpcError> {
    let v: serde_json::Value = serde_json::from_str(line)
        .map_err(|e| RpcError::new(ErrorCode::ParseError, format!("parse: {e}")))?;
    let jsonrpc = v
        .get("jsonrpc")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if jsonrpc != "2.0" {
        return Err(RpcError::new(
            ErrorCode::InvalidRequest,
            format!("jsonrpc must be \"2.0\", got \"{jsonrpc}\""),
        ));
    }
    let id = v.get("id").cloned().filter(|x| !x.is_null());
    let method = v.get("method").and_then(serde_json::Value::as_str);
    let params = v.get("params").cloned().unwrap_or(serde_json::Value::Null);
    match (method, id) {
        (Some(m), Some(id)) => {
            let method: Method = m.parse().map_err(|name: String| {
                RpcError::with_data(
                    ErrorCode::MethodNotFound,
                    format!("unknown method '{name}'"),
                    serde_json::json!({"method": name}),
                )
            })?;
            let id: Id = serde_json::from_value(id)
                .map_err(|_| RpcError::new(ErrorCode::InvalidRequest, "bad id"))?;
            Ok(Envelope::Request(Request {
                jsonrpc: "2.0".into(),
                id,
                method,
                params,
            }))
        }
        (Some(m), None) => {
            let method: Method = m.parse().map_err(|name: String| {
                RpcError::new(
                    ErrorCode::MethodNotFound,
                    format!("unknown method '{name}'"),
                )
            })?;
            Ok(Envelope::Notification(Notification {
                jsonrpc: "2.0".into(),
                method,
                params,
            }))
        }
        (None, Some(id)) => {
            let result = v.get("result").cloned().filter(|x| !x.is_null());
            let error = v
                .get("error")
                .and_then(|e| serde_json::from_value(e.clone()).ok());
            if result.is_none() && error.is_none() {
                return Err(RpcError::new(
                    ErrorCode::InvalidRequest,
                    "response needs result or error",
                ));
            }
            Ok(Envelope::Response(Response {
                jsonrpc: "2.0".into(),
                id: serde_json::from_value(id)
                    .map_err(|_| RpcError::new(ErrorCode::InvalidRequest, "bad id"))?,
                result,
                error,
            }))
        }
        (None, None) => Err(RpcError::new(
            ErrorCode::InvalidRequest,
            "frame needs method or id",
        )),
    }
}

/// Serialises a frame to one ndjson line (no trailing newline).
///
/// # Errors
/// Returns [`ErrorCode::Internal`] when the value cannot serialise.
pub fn write_frame(frame: &Envelope) -> Result<String, RpcError> {
    serde_json::to_string(frame)
        .map_err(|e| RpcError::new(ErrorCode::Internal, format!("ser: {e}")))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Envelope, ErrorCode, Id, Method, parse_frame};

    #[test]
    fn parse_frame_classifies_request() {
        let e = parse_frame(r#"{"jsonrpc":"2.0","id":1,"method":"sys.hello","params":{}}"#)
            .expect("request");
        match e {
            Envelope::Request(r) => {
                assert_eq!(r.id, Id::Num(1));
                assert_eq!(r.method, Method::SysHello);
            }
            other => panic!("want request, got {other:?}"),
        }
    }

    #[test]
    fn parse_frame_classifies_notification() {
        let e = parse_frame(r#"{"jsonrpc":"2.0","method":"sys.shutdown","params":{}}"#)
            .expect("notification");
        assert!(matches!(e, Envelope::Notification(_)));
    }

    #[test]
    fn parse_frame_classifies_response() {
        let e = parse_frame(r#"{"jsonrpc":"2.0","id":7,"result":{"ok":true}}"#).expect("response");
        match e {
            Envelope::Response(r) => assert_eq!(r.id, Id::Num(7)),
            other => panic!("want response, got {other:?}"),
        }
    }

    #[test]
    fn unknown_method_is_method_not_found() {
        let err = parse_frame(r#"{"jsonrpc":"2.0","id":1,"method":"sys.nope"}"#)
            .expect_err("must be rejected");
        assert_eq!(err.code, i32::from(ErrorCode::MethodNotFound));
        assert!(err.message.contains("sys.nope"), "{}", err.message);
    }

    #[test]
    fn bad_jsonrpc_version_rejected() {
        let err = parse_frame(r#"{"jsonrpc":"1.0","id":1,"method":"sys.hello"}"#)
            .expect_err("must be rejected");
        assert_eq!(err.code, i32::from(ErrorCode::InvalidRequest));
    }
}
