//! History seeding and MEMORIZE-marker extraction for the agent node.
//!
//! Moved verbatim (2026-09-17) from `agent.rs` — a move, not a rewrite.

use pii_model::{ChatMessage, Role};

/// Splits a trailing `MEMORIZE: <fact>` marker off the assistant's reply
/// (spec 09 v2 G2.2/G2.3). Returns `(user_visible_text, Option<fact>)`. The
/// marker is a side-channel — stripped from what the user sees, written to
/// global memory. A marker with empty/whitespace fact is ignored.
pub(super) fn split_memorize(text: &str) -> (String, Option<String>) {
    let trimmed = text.trim_end();
    for line in trimmed.rsplit('\n') {
        let l = line.trim();
        if let Some(fact) = l.strip_prefix("MEMORIZE:") {
            let fact = fact.trim();
            if !fact.is_empty() {
                let visible = trimmed[..trimmed.len() - l.len()].trim_end().to_owned();
                return (visible, Some(fact.to_owned()));
            }
        }
    }
    (text.to_owned(), None)
}

/// Seeds prior-turn [`ChatMessage`]s from the run input's `history`
/// (2026-09-05, session memory): an array of `{role, content}`,
/// populated by [`crate::Engine::prompt`] from the session's own tracked
/// transcript. Absent/malformed `history` (e.g. a direct `run.start`
/// call, which doesn't go through a session) degrades to an empty
/// iterator, not an error — a graph run with no session context simply
/// has nothing to seed from. Extracted from `AgentNode::run` to keep it
/// under the workspace's line-count lint.
pub(super) fn history_messages(payload: &serde_json::Value) -> Vec<ChatMessage> {
    payload
        .get("history")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let role = m.get("role").and_then(serde_json::Value::as_str)?;
            let content = m.get("content").and_then(serde_json::Value::as_str)?;
            let role = match role {
                "assistant" => Role::Assistant,
                "system" => Role::System,
                "tool" => Role::Tool,
                _ => Role::User,
            };
            Some(ChatMessage::new(role, content))
        })
        .collect()
}
