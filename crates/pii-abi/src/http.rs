//! HTTP data types (F1.1): pure ABI data for egress, no sockets.
//!
//! These types are part of the stable ABI surface so a worker or WASM guest
//! can name them. `pii-net` implements the transport; `pii-core` defines the
//! [`pii_core::HttpTransport`] seam that consumes them.

use serde::{Deserialize, Serialize};

/// HTTP method vocabulary (F1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HttpMethod {
    /// GET.
    Get,
    /// POST.
    Post,
    /// PUT.
    Put,
    /// DELETE.
    Delete,
}

impl std::fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
        })
    }
}

/// An outgoing HTTP request (F1.1). Pure data — no sockets here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRequest {
    /// Method.
    pub method: HttpMethod,
    /// Absolute URL (http only; https is refused until Phase 3b).
    pub url: String,
    /// Extra headers (caller headers may not override `Host`/`Connection`).
    pub headers: Vec<(String, String)>,
    /// Request body (empty for GET).
    #[serde(default)]
    pub body: Vec<u8>,
    /// Wall-clock budget in milliseconds; one deadline spans all stages.
    pub timeout_ms: u64,
}

/// A completed HTTP response (F1.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpResponse {
    /// Status code.
    pub status: u16,
    /// Response headers in arrival order.
    pub headers: Vec<(String, String)>,
    /// Response body.
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// ASCII-case-insensitive header lookup (first match wins).
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// True for 200..=299.
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..=299).contains(&self.status)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{HttpMethod, HttpRequest, HttpResponse};

    #[test]
    fn method_serialises_screaming_snake() {
        assert_eq!(serde_json::to_string(&HttpMethod::Get).unwrap(), r#""GET""#);
        let m: HttpMethod = serde_json::from_str(r#""POST""#).unwrap();
        assert_eq!(m, HttpMethod::Post);
    }

    #[test]
    fn header_lookup_is_case_insensitive() {
        let r = HttpResponse {
            status: 200,
            headers: vec![(String::from("Content-Type"), String::from("text/plain"))],
            body: Vec::new(),
        };
        assert_eq!(r.header("content-type"), Some("text/plain"));
        assert_eq!(r.header("CONTENT-TYPE"), Some("text/plain"));
        assert_eq!(r.header("missing"), None);
        assert!(r.is_success());
    }

    #[test]
    fn request_roundtrip() {
        let req = HttpRequest {
            method: HttpMethod::Post,
            url: String::from("http://127.0.0.1:1/api/chat"),
            headers: Vec::new(),
            body: b"{}".to_vec(),
            timeout_ms: 5_000,
        };
        let line = serde_json::to_string(&req).unwrap();
        let back: HttpRequest = serde_json::from_str(&line).unwrap();
        assert_eq!(back, req);
    }
}
