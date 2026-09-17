//! Argument parsing and request shaping for the fetch tool (spec 38).
//!
//! The allowlist is the feature: GET, no headers, no body. Everything else
//! is refused with `PiiError::Invalid` BEFORE the chokepoint is touched.

use pii_abi::{HttpMethod, HttpRequest, PiiError};
use serde_json::Value;

/// Parsed, validated tool arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchArgs {
    /// Absolute http(s) URL.
    pub url: String,
    /// Millisecond budget for the whole request.
    pub timeout_ms: u64,
}

impl FetchArgs {
    /// Parses and validates tool arguments. Only `url` (and optionally
    /// `timeout_ms`) may be present; any headers/body/method key is a
    /// refusal — the tool API carries no credentials and permits no mutation.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] for a missing/empty/non-string `url`, any
    /// forbidden key (`headers`, `header`, `method`, `body`, `auth`,
    /// `cookies`), or a non-GET method.
    pub fn parse(args: &Value) -> Result<Self, PiiError> {
        let invalid = |m: &str| PiiError::Invalid(format!("http_fetch: {m}"));
        if let Some(Value::String(method)) = args.get("method")
            && !method.eq_ignore_ascii_case("GET")
        {
            return Err(invalid(
                "only GET is permitted (POST/PUT/DELETE and uploads are blocked)",
            ));
        }
        for forbidden in [
            "headers",
            "header",
            "auth",
            "authorization",
            "cookies",
            "body",
            "form",
            "files",
        ] {
            if args.get(forbidden).is_some() {
                return Err(invalid(&format!(
                    "custom '{forbidden}' is blocked: the fetch API is credential-free and GET-only"
                )));
            }
        }
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("url must be a string"))?;
        if url.is_empty() {
            return Err(invalid("url must not be empty"));
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(invalid("url must be absolute http(s)"));
        }
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(30_000);
        Ok(Self {
            url: String::from(url),
            timeout_ms,
        })
    }

    /// Shapes the chokepoint request: GET, zero caller headers, empty body.
    #[must_use]
    pub fn to_request(&self) -> HttpRequest {
        HttpRequest {
            method: HttpMethod::Get,
            url: self.url.clone(),
            headers: Vec::new(),
            body: Vec::new(),
            timeout_ms: self.timeout_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::FetchArgs;
    use pii_abi::{HttpMethod, PiiError};
    use serde_json::json;

    /// `fetch_post_blocked`: a POST (or any mutating method) is refused.
    #[test]
    fn fetch_post_blocked() {
        let err =
            FetchArgs::parse(&json!({"url": "http://example.com", "method": "POST"})).unwrap_err();
        assert!(matches!(err, PiiError::Invalid(_)), "got {err:?}");
        assert!(
            err.to_string().contains("only GET is permitted"),
            "error must name the GET-only policy: {err}"
        );
    }

    /// `fetch_with_headers`: caller headers are refused — the API is
    /// credential-free.
    #[test]
    fn fetch_with_headers_blocked() {
        for key in ["headers", "header", "auth", "cookies"] {
            let err = FetchArgs::parse(&json!({
                "url": "http://example.com",
                key: {"X-Secret": "s3cr3t"}
            }))
            .unwrap_err();
            assert!(matches!(err, PiiError::Invalid(_)), "{key}: {err:?}");
            assert!(
                err.to_string().contains("blocked"),
                "{key} refusal must say blocked: {err}"
            );
        }
    }

    /// A well-formed GET-only call shapes a bare `HttpRequest`.
    #[test]
    fn fetch_args_shape_bare_get() {
        let a = FetchArgs::parse(&json!({"url": "http://example.com/"})).unwrap();
        let req = a.to_request();
        assert_eq!(req.method, HttpMethod::Get);
        assert!(req.headers.is_empty());
        assert!(req.body.is_empty());
        assert_eq!(req.url, "http://example.com/");
    }

    /// A relative or non-http URL is refused before any network work.
    #[test]
    fn fetch_rejects_non_absolute_url() {
        for url in ["example.com", "ftp://example.com", ""] {
            assert!(
                FetchArgs::parse(&json!({"url": url})).is_err(),
                "must refuse '{url}'"
            );
        }
    }
}
