//! `pii-tui`: a pure protocol client (F5.1–F5.4).
//!
//! Holds **no domain state** (F5.2): no scheduler, no graph, no store, no
//! model. `AppState` is populated only from protocol messages. `render` is
//! pure and testable against a `TestBackend`.

use std::collections::VecDeque;

use futures_util::StreamExt;
use pii_proto::event::{Event, EventFilter};
use pii_proto::method::Method;
use pii_proto::session::SessionCreateParams;
use pii_proto::sys::PeerInfo;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

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

/// Renders the current state (F5.4). Pure: no I/O, no engine.
pub fn render(state: &AppState, frame: &mut Frame<'_>) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(frame.area());
    let [header_area, body_area, footer_area] = chunks.as_ref() else {
        return;
    };

    // Header: connection status + session id.
    let status = match state.status {
        ConnStatus::Disconnected => "disconnected",
        ConnStatus::Connected => "connected",
        ConnStatus::Reconnecting => "reconnecting",
    };
    let session = state
        .snapshot
        .as_ref()
        .map_or_else(|| "no session".to_owned(), |s| s.session_id.clone());
    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            "pii",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::raw(status),
        Span::raw("  session "),
        Span::raw(session),
    ]))
    .block(Block::default().borders(Borders::ALL).title("pii"));
    frame.render_widget(header, *header_area);

    // Body: the delta log (events, prompt echoes, and local system lines).
    // 2026-09-05: styled per role (user input / model output / thinking-
    // acting progress / system), matching the "user command in, model
    // command out, thinking/acting" convention the user asked for.
    let mut lines: Vec<Line<'_>> = Vec::new();
    for line in &state.deltas {
        lines.push(render_log_line(line));
    }
    if lines.is_empty() {
        lines.push(Line::from("(no events yet)"));
    }
    let body = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("events"))
        .wrap(Wrap { trim: true });
    frame.render_widget(body, *body_area);

    // Footer: input buffer.
    let footer = Paragraph::new(Line::from(vec![
        Span::raw("> "),
        Span::raw(state.input.clone()),
    ]))
    .block(Block::default().borders(Borders::ALL))
    .alignment(Alignment::Left);
    frame.render_widget(footer, *footer_area);
}

/// One rendered [`Line`] for any [`LogLine`], styled by role (2026-09-05):
/// user input, model output, thinking/acting progress, or a local system
/// message — matching the "user command in / model command out /
/// thinking-acting" convention. Colors carry the role so scanning a long
/// log is fast without reading every word.
fn render_log_line<'a>(line: &LogLine) -> Line<'a> {
    match line {
        LogLine::Prompt(text) => Line::from(vec![
            Span::styled(
                "you> ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(text.clone(), Style::default().fg(Color::Green)),
        ]),
        LogLine::System(text) => Line::from(Span::styled(
            format!("* {text}"),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )),
        // 2026-09-05: the live progress indicator — the question itself
        // with the run's latest journal seq in hex, e.g.
        // "why is the sky blue? [seq 0x1a]" — cross-referenceable
        // against `pii graph trace <run_id>`'s own seq column. seq=0
        // means no node-lifecycle event has arrived yet for this run.
        LogLine::Thinking { prompt, seq } => Line::from(Span::styled(
            format!("{prompt} [seq {seq:#x}]"),
            Style::default().fg(Color::DarkGray),
        )),
        LogLine::Event(ev) => render_event_line(ev),
    }
}

/// Splits an [`Event`] into its role (thinking/acting progress, the
/// model's final answer, or an error) and styles it accordingly
/// (2026-09-05): dim gray for progress (node/tool lifecycle — "thinking/
/// acting"), cyan+bold for the model's actual output ("pii>"), red for
/// any failure.
fn render_event_line<'a>(ev: &Event) -> Line<'a> {
    let progress =
        |text: String| Line::from(Span::styled(text, Style::default().fg(Color::DarkGray)));
    let answer = |text: String| {
        Line::from(vec![
            Span::styled(
                "pii> ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(text),
        ])
    };
    let error = |text: String| {
        Line::from(Span::styled(
            text,
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ))
    };
    match ev {
        Event::RunStarted { graph, .. } => progress(format!("run started: {graph}")),
        Event::NodeStarted { node, .. } => progress(format!("[{node}] started")),
        Event::NodeFinished { node, port, .. } => progress(format!("  [{node}] -> {port}")),
        Event::NodeFailed { node, error: e, .. } => error(format!("[{node}] failed: {e}")),
        Event::NodeDead { node, .. } => progress(format!("  [{node}] dead")),
        Event::AssistantDelta { text, .. } => answer(text.clone()),
        Event::ToolCall { tool, .. } => progress(format!("  tool: {tool}")),
        Event::ToolResult { tool, ok, .. } => progress(format!(
            "  tool: {tool} {}",
            if *ok { "ok" } else { "error" }
        )),
        Event::RunFinished { terminal, .. } => match terminal {
            pii_proto::event::WireTerminal::Success { output } => answer(
                output
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .map_or_else(|| String::from("done"), str::to_owned),
            ),
            pii_proto::event::WireTerminal::Failure { error: e } => error(format!("error: {e}")),
            pii_proto::event::WireTerminal::Suspended { .. } => {
                progress(String::from("(suspended)"))
            }
        },
        Event::RunCancelled { .. } => progress(String::from("run cancelled")),
        Event::RunSuspended { key, reason, .. } => {
            progress(format!("(suspended: {reason}) key={key}"))
        }
        Event::RunResumed { key, .. } => progress(format!("resumed key={key}")),
        Event::ApprovalRequested { prompt, .. } => {
            progress(format!("approval requested: {prompt}"))
        }
        Event::ApprovalResolved { allow, .. } => progress(format!(
            "  approval {}",
            if *allow { "allowed" } else { "denied" }
        )),
        Event::WorkerDied { worker, pid, .. } => error(format!("worker {worker} pid {pid} died")),
        Event::WorkerRestarted { worker, pid, .. } => {
            progress(format!("worker {worker} pid {pid} restarted"))
        }
        Event::ChannelMessage { channel, chat, .. } => {
            progress(format!("channel message: {channel}/{chat}"))
        }
        Event::MemoryDegraded { lanes, .. } => {
            progress(format!("memory degraded: {}", lanes.join(",")))
        }
        Event::Lagged { dropped, .. } => error(format!("warning: lagged, {dropped} dropped")),
    }
}

/// Sends one prompt on `session_id` and subscribes to its run's events
/// (2026-09-05, extracted from `run_tui` to keep it under the line-count
/// lint — this is the same two-call sequence the original one-shot
/// version did once at startup, now callable per Enter keypress).
///
/// # Errors
/// Returns [`TuiError`] on protocol or transport failure.
async fn submit_prompt(
    client: &pii_proto::client::Client,
    session_id: &str,
    text: String,
) -> Result<(), TuiError> {
    let prompt = client
        .call(
            Method::SessionPrompt,
            serde_json::to_value(pii_proto::session::SessionPromptParams {
                session_id: session_id.to_owned(),
                content: vec![pii_proto::session::ContentBlock::Text { text }],
                input: None,
            })
            .map_err(|e| TuiError::Client(e.to_string()))?,
        )
        .await
        .map_err(|e| TuiError::Client(e.message))?;
    let prompt: pii_proto::session::SessionPromptResult =
        serde_json::from_value(prompt).map_err(|e| TuiError::Client(e.to_string()))?;
    client
        .call(
            Method::EventSubscribe,
            serde_json::to_value(EventFilter {
                run: Some(prompt.run_id),
                session: None,
                kinds: Vec::new(),
                since_seq: Some(0),
            })
            .map_err(|e| TuiError::Client(e.to_string()))?,
        )
        .await
        .map_err(|e| TuiError::Client(e.message))?;
    Ok(())
}

/// Result of handling one keyboard event (2026-09-05, extracted from
/// `run_tui` to keep it under the line-count lint).
enum KeyOutcome {
    /// Keep looping.
    Continue,
    /// A prompt was submitted; the caller should mark a run active.
    Submitted,
    /// The user asked to quit (Esc / Ctrl+C / Ctrl+D / `/exit` / `/quit`).
    Quit,
}

/// Slash commands recognized in the input box (2026-09-05). Checked
/// before the text is treated as a prompt, matching the convention of
/// other terminal chat UIs (a bare `/word` is a local command, not
/// something sent to the model).
///
/// # Errors
/// Returns [`TuiError`] if submitting a prompt fails (only reachable via
/// the `None` branch, i.e. non-command input passed through unchanged).
fn handle_slash_command(text: &str, state: &mut AppState) -> Option<KeyOutcome> {
    match text.trim() {
        "/exit" | "/quit" => Some(KeyOutcome::Quit),
        "/help" => {
            state.push_system(String::from(
                "commands: /exit /quit (leave the TUI), /help (this message), \
                 /clear (clear the log)",
            ));
            Some(KeyOutcome::Continue)
        }
        "/clear" => {
            state.deltas.clear();
            Some(KeyOutcome::Continue)
        }
        other if other.starts_with('/') => {
            state.push_system(format!("unknown command: {other} (try /help)"));
            Some(KeyOutcome::Continue)
        }
        _ => None,
    }
}

/// Handles one crossterm key press against `state`, submitting a prompt on
/// Enter (via [`submit_prompt`]) when the input buffer is non-empty and is
/// not a recognized slash command (see [`handle_slash_command`]).
///
/// # Errors
/// Returns [`TuiError`] if submitting a prompt fails.
async fn handle_key(
    k: crossterm::event::KeyEvent,
    client: &pii_proto::client::Client,
    session_id: &str,
    state: &mut AppState,
) -> Result<KeyOutcome, TuiError> {
    if k.kind != crossterm::event::KeyEventKind::Press {
        return Ok(KeyOutcome::Continue);
    }
    match k.code {
        crossterm::event::KeyCode::Esc => Ok(KeyOutcome::Quit),
        crossterm::event::KeyCode::Char('c' | 'd')
            if k.modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            Ok(KeyOutcome::Quit)
        }
        crossterm::event::KeyCode::Enter if !state.input.trim().is_empty() => {
            let text = std::mem::take(&mut state.input);
            if let Some(outcome) = handle_slash_command(&text, state) {
                return Ok(outcome);
            }
            // 2026-09-05: echo the prompt into the main pane immediately —
            // previously it vanished from the input box the instant Enter
            // was pressed with no trace anywhere in the log until the
            // first `[agent] started` line arrived.
            state.push_prompt_echo(text.clone());
            submit_prompt(client, session_id, text).await?;
            Ok(KeyOutcome::Submitted)
        }
        crossterm::event::KeyCode::Char(c) => {
            state.input.push(c);
            Ok(KeyOutcome::Continue)
        }
        crossterm::event::KeyCode::Backspace => {
            state.input.pop();
            Ok(KeyOutcome::Continue)
        }
        _ => Ok(KeyOutcome::Continue),
    }
}

/// Creates a session and returns the initial [`AppState`] plus the
/// created session's id (2026-09-05, extracted from `run_tui` to keep it
/// under the line-count lint).
///
/// # Errors
/// Returns [`TuiError`] on protocol or transport failure.
async fn init_session(client: &pii_proto::client::Client) -> Result<(AppState, String), TuiError> {
    let cwd = std::env::current_dir().map_or_else(|_| "/".into(), |p| p.display().to_string());
    let created = client
        .call(
            Method::SessionCreate,
            serde_json::to_value(SessionCreateParams {
                cwd: cwd.clone(),
                model: None,
                tools: Vec::new(),
                graph: None,
            })
            .map_err(|e| TuiError::Client(e.to_string()))?,
        )
        .await
        .map_err(|e| TuiError::Client(e.message))?;
    let created: pii_proto::session::SessionCreateResult =
        serde_json::from_value(created).map_err(|e| TuiError::Client(e.to_string()))?;

    let mut state = AppState {
        status: ConnStatus::Connected,
        ..AppState::default()
    };
    state.snapshot = Some(pii_proto::session::SessionStateResult {
        session_id: created.session_id.clone(),
        cwd,
        status: pii_proto::session::SessionStatus::Idle,
        messages: Vec::new(),
        tokens_in: 0,
        tokens_out: 0,
        run_id: None,
    });
    Ok((state, created.session_id))
}

/// Runs the TUI over any async byte pair (F5.3, F11 opus review): hello →
/// session.create → an interactive input loop.
///
/// # Note (2026-09-05 fix)
/// The original implementation sent one hardcoded `"hello"` prompt and
/// exited the instant that single run finished — it had NO keyboard
/// input handling at all, which is why it visibly "opened for a second
/// and closed" for any real user. This rewrite creates the session once,
/// then loops: render → wait for EITHER a keyboard event or a protocol
/// event → on Enter with a non-empty input buffer, submit a NEW
/// `session.prompt` and subscribe to that run; on Esc/Ctrl+C/Ctrl+D,
/// exit cleanly. The event subscription is created once (per run, right
/// after each prompt) rather than once for the whole session, matching
/// the server's existing `run`-scoped `EventFilter`.
///
/// # Errors
/// Returns [`TuiError`] on protocol or transport failure.
pub async fn run_tui<R, W>(r: R, w: W) -> Result<(), TuiError>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let client_info = PeerInfo {
        name: "pii-tui".into(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        build: "debug".into(),
    };
    let (client, _hello) = pii_proto::client::Client::connect(r, w, client_info)
        .await
        .map_err(|e| TuiError::Client(e.to_string()))?;
    let mut events = client.events();

    let (mut state, session_id) = init_session(&client).await?;

    let mut terminal = ratatui::init();
    let mut keys = crossterm::event::EventStream::new();
    let mut run_active = false;

    let result: Result<(), TuiError> = loop {
        terminal
            .draw(|f| render(&state, f))
            .map_err(|e| TuiError::Render(e.to_string()))?;

        tokio::select! {
            key = keys.next() => {
                match key {
                    Some(Ok(crossterm::event::Event::Key(k))) => {
                        match handle_key(k, &client, &session_id, &mut state).await? {
                            KeyOutcome::Continue => {}
                            KeyOutcome::Submitted => run_active = true,
                            KeyOutcome::Quit => break Ok(()),
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => break Err(TuiError::Render(e.to_string())),
                    None => break Ok(()),
                }
            }
            push = events.recv(), if run_active => {
                match push {
                    Ok(p) => {
                        let is_terminal = matches!(
                            &p.event,
                            Event::RunFinished { .. } | Event::RunCancelled { .. }
                        );
                        state.push_event(p.seq, p.event);
                        if is_terminal {
                            run_active = false;
                        }
                    }
                    Err(_) => run_active = false,
                }
            }
        }
    };
    ratatui::restore();
    result
}

/// A minimal prompt helper kept for the reconnect path (F5.3): re-hello and
/// re-subscribe with `since_seq = last_seq`.
#[allow(dead_code)]
fn _reconnect_hint() -> Rect {
    Rect::new(0, 0, 0, 0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{AppState, KeyOutcome, handle_slash_command};

    #[test]
    fn exit_and_quit_signal_quit() {
        let mut state = AppState::default();
        assert!(matches!(
            handle_slash_command("/exit", &mut state),
            Some(KeyOutcome::Quit)
        ));
        assert!(matches!(
            handle_slash_command("/quit", &mut state),
            Some(KeyOutcome::Quit)
        ));
    }

    #[test]
    fn help_pushes_a_system_line_and_continues() {
        let mut state = AppState::default();
        assert!(matches!(
            handle_slash_command("/help", &mut state),
            Some(KeyOutcome::Continue)
        ));
        assert_eq!(state.deltas.len(), 1);
    }

    #[test]
    fn unknown_slash_command_pushes_a_help_hint() {
        let mut state = AppState::default();
        let outcome = handle_slash_command("/bogus", &mut state);
        assert!(matches!(outcome, Some(KeyOutcome::Continue)));
        assert_eq!(state.deltas.len(), 1);
    }

    #[test]
    fn clear_empties_the_log() {
        let mut state = AppState::default();
        state.push_system(String::from("x"));
        state.push_system(String::from("y"));
        assert_eq!(state.deltas.len(), 2);
        let outcome = handle_slash_command("/clear", &mut state);
        assert!(matches!(outcome, Some(KeyOutcome::Continue)));
        assert_eq!(state.deltas.len(), 0);
    }

    #[test]
    fn ordinary_text_is_not_a_command() {
        let mut state = AppState::default();
        assert!(handle_slash_command("hello there", &mut state).is_none());
        assert!(handle_slash_command("what is 3/2?", &mut state).is_none());
        assert_eq!(
            state.deltas.len(),
            0,
            "non-command text must not touch state"
        );
    }
}
