//! The agent's captured output is journalled and stored, so a secret the agent prints must not reach either (the
//! `secret` stand-in printed `sk-live-...` and it was found in the daemon log AND in the store). Two sources are
//! scrubbed: the exact VALUES of the credentials pii passed the agent, and token-shaped strings the agent produced on
//! its own. Controls: ordinary text, a short value, and a non-credential env value (PATH) are left alone.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;

use pii_server::node_kinds::nodes::coder::redact::{credential_values, redact};

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

#[test]
fn a_passed_credential_value_is_scrubbed_from_the_output() {
    let e = env(&[
        ("OLLAMA_API_KEY", "abc123SECRETvalue999"),
        ("PATH", "/usr/bin"),
    ]);
    let out = redact("the key is abc123SECRETvalue999 ok", &credential_values(&e));
    assert!(!out.contains("abc123SECRETvalue999"), "value leaked: {out}");
    assert!(out.contains("[REDACTED]"));
}

#[test]
fn a_token_shaped_string_the_agent_printed_is_scrubbed() {
    let out = redact("token sk-live-FAKESECRET123456789 leaked", &[]);
    assert!(!out.contains("FAKESECRET"), "token leaked: {out}");
}

#[test]
fn a_bearer_header_is_scrubbed() {
    let out = redact(
        "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig",
        &[],
    );
    assert!(!out.contains("eyJhbGci"), "bearer leaked: {out}");
}

#[test]
fn ordinary_text_is_left_alone() {
    let text = "ran 4 tests, all passed; see /usr/bin and sk-learn is a library";
    assert_eq!(
        redact(text, &[]),
        text,
        "near-miss text must pass through unchanged"
    );
}

#[test]
fn a_non_credential_env_value_is_not_treated_as_a_secret() {
    let e = env(&[
        ("PATH", "/usr/local/bin:/usr/bin"),
        ("HOME", "/root"),
        ("TERM", "xterm-256color"),
    ]);
    assert!(
        credential_values(&e).is_empty(),
        "PATH/HOME/TERM are not credentials, scrubbing them would mangle every path"
    );
}

#[test]
fn a_short_credential_value_is_not_scrubbed_everywhere() {
    // A 3-character "key" would replace common substrings across the whole output.
    let e = env(&[("MY_API_KEY", "abc")]);
    assert!(credential_values(&e).is_empty());
}

#[test]
fn every_credential_named_variable_is_collected() {
    let e = env(&[
        ("OLLAMA_API_KEY", "value-one-long-enough"),
        ("GH_TOKEN", "value-two-long-enough"),
        ("DB_PASSWORD", "value-three-long-enough"),
        ("CLIENT_SECRET", "value-four-long-enough"),
    ]);
    assert_eq!(credential_values(&e).len(), 4);
}
