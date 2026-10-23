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
/// on the argv: claude reads its own credential. A budget, when the node sets one, is the claude spend cap.
///
/// A resumed run (`sid`) carries `--resume <sid>`. A fresh run carries `--session-id <new_session>` when the caller
/// chose one, so the id is known before the spawn (spec 244). The two flags are never both emitted.
#[must_use]
pub fn argv(
    sid: Option<&str>,
    new_session: Option<&str>,
    model: &str,
    max_budget_usd: Option<f64>,
    brief: &str,
) -> Vec<String> {
    let s = |x: &str| x.to_owned();
    let mut v = vec![s("claude")];
    match (sid, new_session) {
        (Some(sid), _) => v.extend([s("--resume"), s(sid)]),
        (None, Some(id)) => v.extend([s("--session-id"), s(id)]),
        (None, None) => {}
    }
    v.extend([
        s("--model"),
        s(model),
        s("--permission-mode"),
        s("bypassPermissions"),
        s("--output-format"),
        s("stream-json"),
        s("--verbose"),
    ]);
    if let Some(usd) = max_budget_usd {
        v.extend([s("--max-budget-usd"), format!("{usd}")]);
    }
    v.extend([s("-p"), s(brief)]);
    v
}

/// The env names that carry a claude credential.
pub const CREDENTIAL_ENV: [&str; 2] = ["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN"];

/// Whether `env` gives claude a credential.
///
/// Either an env token, or the login file `<HOME>/.claude/.credentials.json` with an OAuth block. The host's login is
/// that file, and claude reads it itself through `HOME`, so nothing is copied or put on the argv. Only the block's
/// presence is checked; its values are never read into the journal.
#[must_use]
pub fn has_credential(env: &std::collections::BTreeMap<String, String>) -> bool {
    if CREDENTIAL_ENV
        .iter()
        .any(|k| env.get(*k).is_some_and(|v| !v.is_empty()))
    {
        return true;
    }
    let Some(home) = env.get("HOME").filter(|h| !h.is_empty()) else {
        return false;
    };
    std::fs::read_to_string(std::path::Path::new(home).join(".claude/.credentials.json"))
        .ok()
        .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
        .is_some_and(|v| {
            v.get("claudeAiOauth")
                .is_some_and(serde_json::Value::is_object)
        })
}

/// Whether `id` is a lowercase, hyphenated UUID v4: the only shape pii hands `claude` as a session id (spec 244 R5).
/// A graph's `sid` that fails this never reaches the argv.
#[must_use]
pub fn is_session_uuid(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            14 => *c == b'4',
            19 => matches!(c, b'8' | b'9' | b'a' | b'b'),
            _ => c.is_ascii_digit() || (b'a'..=b'f').contains(c),
        })
}
