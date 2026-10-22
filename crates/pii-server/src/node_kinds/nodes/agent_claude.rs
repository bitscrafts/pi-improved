//! What a `claude -p --output-format stream-json` run says about itself, read back from its captured output.
//!
//! Observability only, like [`super::agent_worker::reported_routes`]: the session id is the resume handle, the cost and
//! turns say what the run spent, and `is_error` says whether claude itself called the run a failure. Every field is
//! optional, because a killed or timed-out run has an `init` line and no `result`, and plain text has neither.

/// The facts a claude run reported. A field is `None` when the output did not carry it, which is unknown, not zero.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunFacts {
    /// The session id from the `init` line, or the `result` line when only that was captured.
    pub session_id: Option<String>,
    /// The model claude reported on its `init` line.
    pub model: Option<String>,
    /// `num_turns` from the `result` line.
    pub turns: Option<u64>,
    /// `total_cost_usd` from the `result` line.
    pub cost_usd: Option<f64>,
    /// `is_error` from the `result` line.
    pub is_error: Option<bool>,
}

/// Reads the facts out of claude's stream-json output. Lines that are not JSON are skipped.
#[must_use]
pub fn run_facts(output: &str) -> RunFacts {
    let mut facts = RunFacts::default();
    for line in output.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let kind = v.get("type").and_then(serde_json::Value::as_str);
        if facts.session_id.is_none() {
            facts.session_id = v
                .get("session_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
        }
        match (kind, v.get("subtype").and_then(serde_json::Value::as_str)) {
            (Some("system"), Some("init")) => {
                facts.model = v
                    .get("model")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
            }
            (Some("result"), _) => {
                facts.turns = v.get("num_turns").and_then(serde_json::Value::as_u64);
                facts.cost_usd = v.get("total_cost_usd").and_then(serde_json::Value::as_f64);
                facts.is_error = v.get("is_error").and_then(serde_json::Value::as_bool);
            }
            _ => {}
        }
    }
    facts
}

/// The `claude` command line for one dispatch.
///
/// Headless `-p` cannot ask for permission, so without a mode it cannot write. stream-json (which `-p` only allows
/// with `--verbose`) is what makes the route, the session id and the cost readable. Partial messages are NOT
/// requested: one haiku run measured 3,779 `stream_event` lines, past the capture bound. No provider flag and no key
/// on the argv: claude reads its own credential.
#[must_use]
pub fn argv(sid: Option<&str>, model: &str, brief: &str) -> Vec<String> {
    let s = |x: &str| x.to_owned();
    let mut v = vec![s("claude")];
    if let Some(sid) = sid {
        v.extend([s("--resume"), s(sid)]);
    }
    v.extend([
        s("--model"),
        s(model),
        s("--permission-mode"),
        s("bypassPermissions"),
        s("--output-format"),
        s("stream-json"),
        s("--verbose"),
        s("-p"),
        s(brief),
    ]);
    v
}
