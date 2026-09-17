//! Conversation compaction (spec 12, R1).
//!
//! When the estimated token count of the message list crosses the
//! threshold, the *oldest* turns are collapsed into a single summary
//! system message rather than truncating the head — truncation would lose
//! the system prompt's authority and the earliest user intent. The
//! summary is a deterministic condensation (roles + trimmed content), not
//! a model call: no extra tokens, no extra latency.
//!
//! Token estimate uses the chars/4 heuristic (G8): the exact number is
//! not load-bearing, only the threshold crossing.

use pii_model::{ChatMessage, Role};

/// Compaction knobs, derived from the node config.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Compaction {
    /// Fire compaction when `estimate_tokens(messages)` exceeds this.
    pub threshold_tokens: usize,
    /// Recent turns (counted from the tail) never collapsed.
    pub keep_recent_turns: usize,
}

impl Default for Compaction {
    fn default() -> Self {
        Self {
            threshold_tokens: 24_000,
            keep_recent_turns: 6,
        }
    }
}

/// Chars/4 estimate over every message's content (G8 heuristic).
#[must_use]
pub(crate) fn estimate_tokens(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .map(|m| m.content.chars().count())
        .sum::<usize>()
        / 4
}

/// Returns `true` when `messages` is above the compaction threshold.
#[must_use]
pub(crate) fn needs_compaction(messages: &[ChatMessage], compaction: &Compaction) -> bool {
    estimate_tokens(messages) > compaction.threshold_tokens
}

/// Compacts `messages`: the oldest turns — everything except the system
/// prompt at the head and the last `keep_recent_turns` non-system turns —
/// collapse into one summary system message inserted right after the
/// leading system prompt. The current (trailing) user message is always
/// preserved, as are the most recent tool results.
#[must_use]
pub(crate) fn compact(messages: &[ChatMessage], compaction: &Compaction) -> Vec<ChatMessage> {
    // Split off the leading system prompt (never collapsed — the system
    // prompt's authority must survive compaction).
    let head_systems = messages
        .iter()
        .take_while(|m| m.role == Role::System)
        .count();
    let (head, rest) = messages.split_at(head_systems.max(1).min(messages.len()));

    // Classify the rest into turn boundaries: a turn starts at a User
    // message (or the first message when the transcript opens with tool
    // results from a seeded history).
    let mut turn_starts: Vec<usize> = Vec::new();
    for (i, m) in rest.iter().enumerate() {
        if m.role == Role::User || turn_starts.is_empty() {
            turn_starts.push(i);
        }
    }
    let n_turns = turn_starts.len();
    // Nothing to collapse: fewer turns than we would keep plus one.
    if n_turns <= compaction.keep_recent_turns {
        let mut out = head.to_vec();
        out.extend_from_slice(rest);
        return out;
    }
    let split_at = turn_starts
        .get(n_turns - compaction.keep_recent_turns)
        .copied()
        .unwrap_or(0);
    let (old, recent) = rest.split_at(split_at);

    let mut out = head.to_vec();
    out.push(ChatMessage::new(Role::System, summary_of(old)));
    out.extend_from_slice(recent);
    out
}

/// Deterministic condensation of the collapsed turns: per message,
/// `role: <first 160 chars>`. No model call (G8 spirit: cheap, bounded).
fn summary_of(turns: &[ChatMessage]) -> String {
    let mut s = String::from("Earlier context (compacted):\n");
    for m in turns {
        let role = match m.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        let mut content: String = m.content.chars().take(160).collect();
        if m.content.chars().count() > 160 {
            content.push('…');
        }
        s.push_str(role);
        s.push_str(": ");
        s.push_str(&content);
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Role, estimate_tokens};

    fn user(s: &str) -> pii_model::ChatMessage {
        pii_model::ChatMessage::new(Role::User, s)
    }

    #[allow(dead_code)] // reason: used below once more cases land; kept for the
    // table contract
    fn long_turn(i: usize) -> Vec<pii_model::ChatMessage> {
        // ~4.8k chars ≈ 1.2k estimated tokens per turn.
        let filler = "x".repeat(4_800);
        vec![
            user(&format!("turn {i} about {filler}")),
            pii_model::ChatMessage::new(Role::Assistant, format!("answer {i} {filler}")),
        ]
    }

    /// The threshold uses chars/4 (G8): 4 chars → 1 estimated token.
    #[test]
    fn estimate_is_chars_over_four() {
        let msgs = vec![user("abcd")];
        assert_eq!(estimate_tokens(&msgs), 1);
    }

    /// R1: a long conversation above the threshold compacts the oldest turns
    /// into a summary system message; the current user message survives.
    #[test]
    fn compaction_fires_above_threshold() {
        use super::{Compaction, compact, needs_compaction};
        let mut msgs = vec![pii_model::ChatMessage::new(Role::System, "sys")];
        // 20 long turns ≈ 20 * 1.2k = 24k estimated tokens > 24k threshold.
        for i in 0..20 {
            msgs.extend(long_turn(i));
        }
        let comp = Compaction {
            threshold_tokens: 24_000,
            keep_recent_turns: 6,
        };
        assert!(needs_compaction(&msgs, &comp), "should need compaction");
        let out = compact(&msgs, &comp);
        // A summary system message was inserted after the head system prompt.
        assert!(
            out.iter()
                .any(|m| m.role == Role::System && m.content.starts_with("Earlier context")),
            "summary system message missing"
        );
        // The last user message (the current turn) survives.
        assert!(
            out.iter()
                .any(|m| m.role == Role::User && m.content.starts_with("turn 19")),
            "current user message lost"
        );
        // The output is strictly smaller than the input.
        assert!(out.len() < msgs.len(), "compaction must shrink");
    }

    /// R1: a short conversation is byte-identical (no compaction).
    #[test]
    fn compaction_skips_short_conversations() {
        use super::{Compaction, compact, needs_compaction};
        let msgs = vec![
            pii_model::ChatMessage::new(Role::System, "sys"),
            user("hello"),
            pii_model::ChatMessage::new(Role::Assistant, "hi"),
        ];
        let comp = Compaction::default();
        assert!(
            !needs_compaction(&msgs, &comp),
            "short convo must not compact"
        );
        let out = compact(&msgs, &comp);
        assert_eq!(out, msgs, "short convo must be unchanged");
    }

    /// R1: recent tool results survive compaction; only the oldest turns collapse.
    #[test]
    fn compaction_preserves_recent_tool_results() {
        use super::{Compaction, compact};
        let mut msgs = vec![pii_model::ChatMessage::new(Role::System, "sys")];
        for i in 0..20 {
            msgs.extend(long_turn(i));
        }
        // Append a recent tool result (the last thing before the current turn).
        msgs.push(pii_model::ChatMessage::new(
            Role::Tool,
            "recent tool output",
        ));
        let comp = Compaction {
            threshold_tokens: 24_000,
            keep_recent_turns: 6,
        };
        let out = compact(&msgs, &comp);
        assert!(
            out.iter()
                .any(|m| m.role == Role::Tool && m.content == "recent tool output"),
            "recent tool result lost"
        );
    }
}
