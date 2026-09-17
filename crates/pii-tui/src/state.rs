//! TUI state types (F5.2): protocol-derived state only.

use std::collections::VecDeque;

use pii_proto::event::Event;

/// TUI failure modes.
#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    /// Protocol/transport failure.
    #[error("tui: {0}")]
    Client(String),
    /// Terminal render failure.
    #[error("tui: render: {0}")]
    Render(String),
}

/// Connection status (derived from protocol messages only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnStatus {
    /// Not yet connected.
    Disconnected,
    /// Connected and helloed.
    Connected,
    /// Reconnecting.
    Reconnecting,
}

/// One line in the TUI's event/echo log.
///
/// A real protocol [`Event`], the user's own submitted prompt (so it's
/// visible in the main pane instead of vanishing the instant Enter is
/// pressed), a local system message (e.g. an unknown slash command), or
/// the live "thinking" indicator described on [`LogLine::Thinking`].
/// Only the `Event` variant is server-derived; the rest are pure
/// client-side UI feedback, not "domain state" in the F5.2 sense (no
/// scheduler/graph/store/model data crosses into them). (2026-09-05)
pub enum LogLine {
    /// A protocol event from the daemon.
    Event(Event),
    /// Echo of a prompt the user just submitted.
    Prompt(String),
    /// A local message not tied to any protocol event (e.g. command help).
    System(String),
    /// The active run's live progress indicator (2026-09-05): renders as
    /// the original prompt text with the run's latest journal sequence
    /// number appended in hex (`"why is the sky blue? [seq 0x1a]"`) —
    /// the SAME `seq` [`pii graph trace <run_id>`] would show per line,
    /// so a stuck/runaway run is directly cross-referenceable against the
    /// trace log, not just "still thinking". Replaces the previous
    /// per-`[node] started`/`-> port` line-per-event noise — those
    /// events still advance the shown seq but no longer each get their
    /// own log line. This is the LAST line in the log while a run is
    /// active; it is replaced in place (not appended) by
    /// [`AppState::tick_thinking`], and converted back to a plain
    /// [`LogLine::Prompt`] once the run reaches a terminal state.
    Thinking {
        /// The prompt this progress indicator belongs to.
        prompt: String,
        /// The latest event's journal sequence number for this run.
        seq: u64,
    },
}

/// The TUI's entire state (F5.2): only protocol-derived fields, plus the
/// local UI-only fields documented on [`LogLine`].
pub struct AppState {
    /// Last `session.state` snapshot.
    pub snapshot: Option<pii_proto::session::SessionStateResult>,
    /// Received event deltas plus local echo/system lines, capped at 500,
    /// in display order (2026-09-05: replaces the old events-only log so
    /// a submitted prompt is visible in the main pane, not just implied
    /// by the next `[agent] started` line).
    pub deltas: VecDeque<LogLine>,
    /// Connection status.
    pub status: ConnStatus,
    /// Input buffer.
    pub input: String,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            snapshot: None,
            deltas: VecDeque::new(),
            status: ConnStatus::Disconnected,
            input: String::new(),
        }
    }
}

impl AppState {
    /// Records a received event (2026-09-05): node-lifecycle events
    /// (`NodeStarted`/`NodeFinished`/`NodeDead`/`ToolCall`/`ToolResult`)
    /// advance the trailing [`LogLine::Thinking`] indicator's shown `seq`
    /// instead of each getting their own log line — see
    /// [`Self::tick_thinking`]. Everything else (the final answer, any
    /// failure, approval requests, worker/channel/memory events) is
    /// still logged in full; nothing observable is silently dropped.
    /// `seq` is the event's journal sequence number (the SAME number
    /// `pii graph trace <run_id>` shows per line) — passed by the caller
    /// from `EventPush::seq`, not derived here.
    pub fn push_event(&mut self, seq: u64, ev: Event) {
        if matches!(
            ev,
            Event::NodeStarted { .. }
                | Event::NodeFinished { .. }
                | Event::NodeDead { .. }
                | Event::ToolCall { .. }
                | Event::ToolResult { .. }
        ) {
            self.tick_thinking(seq);
            return;
        }
        self.finish_thinking();
        self.push_line(LogLine::Event(ev));
    }

    /// Records the user's own submitted prompt (2026-09-05) as a live
    /// [`LogLine::Thinking`] indicator — NOT a separate echo line: the
    /// prompt text and the live seq share one line
    /// (`"you> why is the sky blue? [seq 0x1a]"`), per the user's
    /// explicit request (a real, cross-referenceable identifier — the
    /// same `seq` a trace log line would show — rather than an arbitrary
    /// dot count). [`Self::finish_thinking`] converts it back to a plain
    /// [`LogLine::Prompt`] once the run completes, so the permanent log
    /// record is just the question, not a stale seq number.
    pub fn push_prompt_echo(&mut self, text: String) {
        self.push_line(LogLine::Thinking {
            prompt: text,
            seq: 0,
        });
    }

    /// Records a local system message (2026-09-05), e.g. an unrecognized
    /// slash command's help text.
    pub fn push_system(&mut self, text: String) {
        self.push_line(LogLine::System(text));
    }

    /// Advances the trailing `Thinking` line's shown seq to `seq`
    /// (2026-09-05). A no-op if the last line isn't `Thinking` (e.g.
    /// between runs).
    fn tick_thinking(&mut self, seq: u64) {
        if let Some(LogLine::Thinking { seq: shown, .. }) = self.deltas.back_mut() {
            *shown = seq;
        }
    }

    /// Converts the trailing `Thinking` line back into a plain `Prompt`
    /// line (seq indicator removed) once a run reaches a terminal event
    /// (2026-09-05), so the permanent log record is just the question —
    /// not a stale seq number frozen from whenever the run finished.
    fn finish_thinking(&mut self) {
        if let Some(line @ LogLine::Thinking { .. }) = self.deltas.back_mut() {
            let LogLine::Thinking { prompt, .. } =
                std::mem::replace(line, LogLine::System(String::new()))
            else {
                unreachable!("matched Thinking above");
            };
            *line = LogLine::Prompt(prompt);
        }
    }

    fn push_line(&mut self, line: LogLine) {
        if self.deltas.len() >= 500 {
            self.deltas.pop_front();
        }
        self.deltas.push_back(line);
    }
}
