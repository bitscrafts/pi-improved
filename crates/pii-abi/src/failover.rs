//! Typed failover reasons and the pure classifier that produces them.
//!
//! `classify` maps `(status, message, origin)` to a [`FailoverReason`];
//! [`FailoverReason::recovery`] maps that reason to a [`Recovery`] strategy.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Typed reason a provider call failed over.
///
/// Each variant states its producer and recovery; see [`Recovery`] for the mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FailoverReason {
    /// Invalid or missing credentials (401, or 403 without billing text). `NextProvider`.
    Auth,
    /// Credit or quota exhaustion (402, or 403/429 with billing text). `NextProvider`.
    Billing,
    /// Our request rate exceeded the provider's limit (429 from the provider). `RetrySame`.
    RateLimit,
    /// The model behind an aggregator is throttled, not our key (429 relayed upstream). `NextProvider`.
    UpstreamRateLimit,
    /// The provider is at capacity (503/529, or 429 reused for capacity). `RetrySame`.
    Overloaded,
    /// A provider-side fault (500/502, or other 5xx). `RetrySame`.
    ServerError,
    /// The call did not complete before its deadline (408, or a timeout message). `RetrySame`.
    Timeout,
    /// The request exceeds the model's context window. `Compress` — never advances the chain.
    ContextOverflow,
    /// The request body exceeds a size limit (413). `Compress` — never advances the chain.
    PayloadTooLarge,
    /// The named model does not exist on this provider (404 naming a model). `NextProvider`.
    ModelNotFound,
    /// No rule matched; the failure mode is not identified. `NextProvider` — never guess retryable.
    Unknown,
}

/// Every [`FailoverReason`] variant, in declaration order.
pub const ALL: [FailoverReason; 11] = [
    FailoverReason::Auth,
    FailoverReason::Billing,
    FailoverReason::RateLimit,
    FailoverReason::UpstreamRateLimit,
    FailoverReason::Overloaded,
    FailoverReason::ServerError,
    FailoverReason::Timeout,
    FailoverReason::ContextOverflow,
    FailoverReason::PayloadTooLarge,
    FailoverReason::ModelNotFound,
    FailoverReason::Unknown,
];

impl FailoverReason {
    /// The wire name, identical to the `serde` `snake_case` form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::Billing => "billing",
            Self::RateLimit => "rate_limit",
            Self::UpstreamRateLimit => "upstream_rate_limit",
            Self::Overloaded => "overloaded",
            Self::ServerError => "server_error",
            Self::Timeout => "timeout",
            Self::ContextOverflow => "context_overflow",
            Self::PayloadTooLarge => "payload_too_large",
            Self::ModelNotFound => "model_not_found",
            Self::Unknown => "unknown",
        }
    }

    /// The recovery strategy for this reason.
    #[rustfmt::skip]
    #[must_use]
    pub const fn recovery(self) -> Recovery {
        match self {
            Self::Auth | Self::Billing | Self::UpstreamRateLimit
            | Self::ModelNotFound | Self::Unknown => Recovery::NextProvider,
            Self::RateLimit | Self::Overloaded | Self::ServerError | Self::Timeout => Recovery::RetrySame,
            Self::ContextOverflow | Self::PayloadTooLarge => Recovery::Compress,
        }
    }
}

impl fmt::Display for FailoverReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The recovery strategy selected by a [`FailoverReason`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Recovery {
    /// Retry the same provider (transient failure).
    RetrySame,
    /// Move to the next provider in the chain.
    NextProvider,
    /// Compress the request before retrying; never advance the chain.
    Compress,
}

/// Which side produced an error.
///
/// `Provider` is the endpoint we called; `Upstream` is an aggregator relaying an error from
/// the model behind it. `classify` does not detect origin — the caller states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The endpoint we called produced the error itself.
    Provider,
    /// An aggregator relayed an error from the model behind it.
    Upstream,
}

#[rustfmt::skip]
const BILLING: &[&str] = &[
    "insufficient credits", "insufficient_quota", "insufficient balance", "credit balance",
    "credits exhausted", "payment required", "exceeded your current quota", "out of funds",
    "key limit exceeded", "spending limit",
];

#[rustfmt::skip]
const RATE_LIMIT: &[&str] = &[
    "rate limit", "rate_limit", "too many requests", "throttl",
    "requests per minute", "tokens per minute", "resource_exhausted",
];

const OVERLOADED: &[&str] = &["overloaded", "at capacity", "over capacity"];
#[rustfmt::skip]
const CONTEXT: &[&str] = &[
    "context length", "context size", "maximum context", "context window",
    "too many tokens", "prompt too long", "prompt is too long", "input is too long",
    "max_model_len", "n_ctx", "maximum allowed input length",
    "exceeds the maximum number of input tokens",
];
const TIMEOUT: &[&str] = &["timed out", "timeout", "deadline exceeded"];
#[rustfmt::skip]
const MODEL_NOT_FOUND_HINTS: &[&str] =
    &["not found", "does not exist", "no such", "unknown", "invalid", "unsupported"];

fn any_match(haystack: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|p| haystack.contains(p))
}

/// Classifies a provider failure into a [`FailoverReason`].
///
/// Pure, total and deterministic: never panics for any `status`, any `message` (empty,
/// non-ASCII, or multi-KB), and either `origin`. Message matching is ASCII-case-insensitive
/// substring matching, evaluated as a first-match-wins rule list (spec 37).
#[must_use]
pub fn classify(status: Option<u16>, message: &str, origin: Origin) -> FailoverReason {
    let m = message.to_ascii_lowercase();
    match status {
        Some(401) => FailoverReason::Auth,
        Some(403) => {
            if any_match(&m, BILLING) {
                FailoverReason::Billing
            } else {
                FailoverReason::Auth
            }
        }
        Some(402) => FailoverReason::Billing,
        Some(404) => {
            if m.contains("model") && any_match(&m, MODEL_NOT_FOUND_HINTS) {
                FailoverReason::ModelNotFound
            } else {
                FailoverReason::Unknown
            }
        }
        Some(408) => FailoverReason::Timeout,
        Some(413) => FailoverReason::PayloadTooLarge,
        Some(429) => classify_429(&m, origin),
        Some(400) => {
            if any_match(&m, CONTEXT) {
                FailoverReason::ContextOverflow
            } else {
                FailoverReason::Unknown
            }
        }
        Some(500 | 502) => {
            if any_match(&m, CONTEXT) {
                FailoverReason::ContextOverflow
            } else {
                FailoverReason::ServerError
            }
        }
        Some(503 | 529) => {
            if any_match(&m, CONTEXT) {
                FailoverReason::ContextOverflow
            } else {
                FailoverReason::Overloaded
            }
        }
        Some(other) if (500..=599).contains(&other) => FailoverReason::ServerError,
        Some(_) => FailoverReason::Unknown,
        None => classify_message_only(&m, origin),
    }
}

fn classify_429(m: &str, origin: Origin) -> FailoverReason {
    if any_match(m, OVERLOADED) {
        return FailoverReason::Overloaded;
    }
    match origin {
        Origin::Upstream => FailoverReason::UpstreamRateLimit,
        Origin::Provider => {
            if any_match(m, BILLING) {
                FailoverReason::Billing
            } else {
                FailoverReason::RateLimit
            }
        }
    }
}

fn classify_message_only(m: &str, origin: Origin) -> FailoverReason {
    if any_match(m, TIMEOUT) {
        return FailoverReason::Timeout;
    }
    if any_match(m, RATE_LIMIT) {
        return match origin {
            Origin::Upstream => FailoverReason::UpstreamRateLimit,
            Origin::Provider => FailoverReason::RateLimit,
        };
    }
    if any_match(m, OVERLOADED) {
        return FailoverReason::Overloaded;
    }
    if any_match(m, CONTEXT) {
        return FailoverReason::ContextOverflow;
    }
    FailoverReason::Unknown
}

#[cfg(test)]
mod tests {
    use super::{ALL, FailoverReason, Origin, Recovery, classify};

    const P: Origin = Origin::Provider;
    const U: Origin = Origin::Upstream;

    fn check(status: Option<u16>, msg: &str, origin: Origin, want: FailoverReason) {
        assert_eq!(classify(status, msg, origin), want);
    }
    #[test]
    fn classify_401_is_auth() {
        check(Some(401), "invalid api key", P, FailoverReason::Auth);
    }
    #[test]
    fn classify_403_billing_text_is_billing() {
        check(Some(403), "Key limit exceeded", P, FailoverReason::Billing);
    }
    #[test]
    fn classify_403_plain_is_auth() {
        check(Some(403), "forbidden", P, FailoverReason::Auth);
    }
    #[test]
    fn classify_402_is_billing() {
        check(Some(402), "", P, FailoverReason::Billing);
    }
    #[test]
    fn classify_404_naming_model_is_model_not_found() {
        let msg = "model 'foo' not found, try pulling it first";
        check(Some(404), msg, P, FailoverReason::ModelNotFound);
    }
    #[test]
    fn classify_404_bare_is_unknown() {
        check(Some(404), "404 page not found", P, FailoverReason::Unknown);
    }
    #[test]
    fn classify_408_is_timeout() {
        check(Some(408), "", P, FailoverReason::Timeout);
    }
    #[test]
    fn classify_413_is_payload_too_large() {
        check(Some(413), "", P, FailoverReason::PayloadTooLarge);
    }
    #[test]
    fn classify_429_provider_is_rate_limit() {
        check(Some(429), "Too Many Requests", P, FailoverReason::RateLimit);
    }
    #[test]
    fn classify_429_upstream_is_upstream_rate_limit() {
        check(
            Some(429),
            "Provider returned error",
            U,
            FailoverReason::UpstreamRateLimit,
        );
    }
    #[test]
    fn classify_429_overloaded_text_is_overloaded() {
        check(
            Some(429),
            "server is overloaded",
            U,
            FailoverReason::Overloaded,
        );
    }
    #[test]
    fn classify_429_billing_text_is_billing() {
        check(
            Some(429),
            "insufficient credits",
            P,
            FailoverReason::Billing,
        );
    }
    #[test]
    fn classify_503_and_529_are_overloaded() {
        check(Some(503), "", P, FailoverReason::Overloaded);
        check(Some(529), "", P, FailoverReason::Overloaded);
    }
    #[test]
    fn classify_500_and_502_are_server_error() {
        check(Some(500), "", U, FailoverReason::ServerError);
        check(Some(502), "", P, FailoverReason::ServerError);
    }
    #[test]
    fn classify_400_context_text_is_context_overflow() {
        let msg = "This model's maximum context length is 8192 tokens";
        check(Some(400), msg, P, FailoverReason::ContextOverflow);
    }
    #[test]
    fn classify_500_context_text_is_context_overflow() {
        check(
            Some(500),
            "context length exceeded",
            P,
            FailoverReason::ContextOverflow,
        );
    }
    #[test]
    fn classify_message_only_timeout() {
        check(
            None,
            "deadline exceeded after 5000ms",
            P,
            FailoverReason::Timeout,
        );
    }
    #[test]
    fn classify_message_only_prompt_too_long() {
        check(
            None,
            "llamacpp: prompt too long",
            P,
            FailoverReason::ContextOverflow,
        );
    }
    #[test]
    fn every_recovery_is_reachable_and_compress_is_exact() {
        let mut compress = Vec::new();
        let mut retry_same = 0;
        let mut next_provider = 0;
        for r in ALL {
            match r.recovery() {
                Recovery::Compress => compress.push(r),
                Recovery::RetrySame => retry_same += 1,
                Recovery::NextProvider => next_provider += 1,
            }
        }
        assert_eq!(
            compress,
            vec![
                FailoverReason::ContextOverflow,
                FailoverReason::PayloadTooLarge
            ]
        );
        assert!(retry_same >= 1);
        assert!(next_provider >= 1);
    }
    #[test]
    fn compress_reasons_never_advance_chain() {
        for r in [
            FailoverReason::ContextOverflow,
            FailoverReason::PayloadTooLarge,
        ] {
            assert_eq!(r.recovery(), Recovery::Compress);
            assert_ne!(r.recovery(), Recovery::NextProvider);
        }
    }
    #[test]
    fn unknown_is_next_provider_not_retry() {
        assert_eq!(FailoverReason::Unknown.recovery(), Recovery::NextProvider);
        check(
            None,
            "net: no address reachable for host",
            P,
            FailoverReason::Unknown,
        );
        check(Some(418), "", P, FailoverReason::Unknown);
        check(Some(200), "", P, FailoverReason::Unknown);
    }
    #[test]
    fn throttle_text_beats_context_text() {
        let msg = "Throttling error: too many tokens, please wait";
        check(None, msg, P, FailoverReason::RateLimit);
    }
    #[test]
    fn wire_name_matches_serde_for_every_variant() {
        for r in ALL {
            let value = serde_json::to_value(r).ok();
            assert_eq!(value, Some(serde_json::json!(r.as_str())));
            assert_eq!(r.to_string(), r.as_str());
            let back: Result<FailoverReason, _> =
                serde_json::from_value(serde_json::json!(r.as_str()));
            assert_eq!(back.ok(), Some(r));
        }
    }
    #[test]
    fn all_lists_every_variant_once() {
        assert_eq!(ALL.len(), 11);
        let mut names: Vec<&str> = ALL.iter().map(|r| r.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 11);

        // Exhaustive match: fails to compile if a variant is added without updating ALL.
        match FailoverReason::Unknown {
            FailoverReason::Auth
            | FailoverReason::Billing
            | FailoverReason::RateLimit
            | FailoverReason::UpstreamRateLimit
            | FailoverReason::Overloaded
            | FailoverReason::ServerError
            | FailoverReason::Timeout
            | FailoverReason::ContextOverflow
            | FailoverReason::PayloadTooLarge
            | FailoverReason::ModelNotFound
            | FailoverReason::Unknown => {}
        }
    }
    #[test]
    fn classify_never_panics() {
        let messages = ["", "ünïcödé ☃", &"x".repeat(65_536)];
        for status in (0..=999u16).map(Some).chain([None]) {
            for message in messages {
                for origin in [P, U] {
                    let _ = classify(status, message, origin);
                }
            }
        }
    }
}
