//! Rendering (F5.4): pure functions, no I/O, no engine.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::state::{AppState, ConnStatus, LogLine};

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
pub(crate) fn render_log_line<'a>(line: &LogLine) -> Line<'a> {
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
// Kept as one function (matches the file's move from lib.rs verbatim, per
// spec-50 G1): the match arms are only reviewable together, and splitting
// would scatter styling that must stay visually consistent across event
// kinds. Slightly over clippy's 80-line default after the crate-level doc
// comment on the file grew during the split.
#[allow(clippy::too_many_lines)]
fn render_event_line<'a>(ev: &pii_proto::event::Event) -> Line<'a> {
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
        pii_proto::event::Event::RunStarted { graph, .. } => {
            progress(format!("run started: {graph}"))
        }
        pii_proto::event::Event::RouteDecision { step, model, .. } => {
            progress(format!("  route: {step} -> {model}"))
        }
        pii_proto::event::Event::RunSpawned { child_run, .. } => {
            progress(format!("  spawned: {child_run}"))
        }
        pii_proto::event::Event::NodeStarted { node, .. } => progress(format!("[{node}] started")),
        pii_proto::event::Event::NodeFinished { node, port, .. } => {
            progress(format!("  [{node}] -> {port}"))
        }
        pii_proto::event::Event::NodeFailed { node, error: e, .. } => {
            error(format!("[{node}] failed: {e}"))
        }
        pii_proto::event::Event::NodeDead { node, .. } => progress(format!("  [{node}] dead")),
        pii_proto::event::Event::AssistantDelta { text, .. } => answer(text.clone()),
        pii_proto::event::Event::ToolCall { tool, .. } => progress(format!("  tool: {tool}")),
        pii_proto::event::Event::ToolResult { tool, ok, .. } => progress(format!(
            "  tool: {tool} {}",
            if *ok { "ok" } else { "error" }
        )),
        pii_proto::event::Event::RunFinished { terminal, .. } => match terminal {
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
        pii_proto::event::Event::RunCancelled { .. } => progress(String::from("run cancelled")),
        pii_proto::event::Event::RunSuspended { key, reason, .. } => {
            progress(format!("(suspended: {reason}) key={key}"))
        }
        pii_proto::event::Event::RunResumed { key, .. } => progress(format!("resumed key={key}")),
        pii_proto::event::Event::ApprovalRequested { prompt, .. } => {
            progress(format!("approval requested: {prompt}"))
        }
        pii_proto::event::Event::ApprovalResolved { allow, .. } => progress(format!(
            "  approval {}",
            if *allow { "allowed" } else { "denied" }
        )),
        pii_proto::event::Event::WorkerDied { worker, pid, .. } => {
            error(format!("worker {worker} pid {pid} died"))
        }
        pii_proto::event::Event::WorkerRestarted { worker, pid, .. } => {
            progress(format!("worker {worker} pid {pid} restarted"))
        }
        pii_proto::event::Event::ChannelMessage { channel, chat, .. } => {
            progress(format!("channel message: {channel}/{chat}"))
        }
        pii_proto::event::Event::MemoryDegraded { lanes, .. } => {
            progress(format!("memory degraded: {}", lanes.join(",")))
        }
        pii_proto::event::Event::Lagged { dropped, .. } => {
            error(format!("warning: lagged, {dropped} dropped"))
        }
    }
}
