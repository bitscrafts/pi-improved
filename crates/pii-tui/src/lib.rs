//! `pii-tui`: a pure protocol client (F5.1–F5.4).
//!
//! Holds **no domain state** (F5.2): no scheduler, no graph, no store, no
//! model. `AppState` is populated only from protocol messages. `render` is
//! pure and testable against a `TestBackend`.

use std::collections::VecDeque;

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

/// The TUI's entire state (F5.2): only protocol-derived fields.
pub struct AppState {
    /// Last `session.state` snapshot.
    pub snapshot: Option<pii_proto::session::SessionStateResult>,
    /// Received event deltas, capped at 500.
    pub deltas: VecDeque<Event>,
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
    /// Records a received event, capping the delta buffer at 500.
    pub fn push_event(&mut self, ev: Event) {
        if self.deltas.len() >= 500 {
            self.deltas.pop_front();
        }
        self.deltas.push_back(ev);
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

    // Body: the delta log.
    let mut lines: Vec<Line<'_>> = Vec::new();
    for ev in &state.deltas {
        lines.push(Line::from(render_event(ev)));
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

/// One human line for an event (F5.7-style, reused by the TUI).
fn render_event(ev: &Event) -> String {
    match ev {
        Event::RunStarted { graph, .. } => format!("run started: {graph}"),
        Event::NodeStarted { node, .. } => format!("⏵ {node}"),
        Event::NodeFinished { node, port, .. } => format!("  {node} → {port}"),
        Event::NodeFailed { node, error, .. } => format!("✘ {node}: {error}"),
        Event::NodeDead { node, .. } => format!("  {node} dead"),
        Event::AssistantDelta { text, .. } => text.clone(),
        Event::ToolCall { tool, .. } => format!("  ↳ {tool}"),
        Event::ToolResult { tool, ok, .. } => {
            format!("  ↳ {tool} {}", if *ok { "ok" } else { "err" })
        }
        Event::RunFinished { terminal, .. } => match terminal {
            pii_proto::event::WireTerminal::Success { .. } => "✔ done".to_owned(),
            pii_proto::event::WireTerminal::Failure { error } => format!("✘ {error}"),
            pii_proto::event::WireTerminal::Suspended { .. } => "… suspended".to_owned(),
        },
        Event::RunCancelled { .. } => "run cancelled".to_owned(),
        Event::Lagged { dropped, .. } => format!("⚠ lagged: {dropped} dropped"),
    }
}

/// Runs the TUI over any async byte pair (F5.3): hello → session.create →
/// event.subscribe → session.state → render loop.
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

    let created = client
        .call(
            Method::SessionCreate,
            serde_json::to_value(SessionCreateParams {
                cwd: std::env::current_dir()
                    .map_or_else(|_| "/".into(), |p| p.display().to_string()),
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

    client
        .call(
            Method::EventSubscribe,
            serde_json::to_value(EventFilter {
                run: None,
                session: Some(created.session_id.clone()),
                kinds: Vec::new(),
                since_seq: Some(0),
            })
            .map_err(|e| TuiError::Client(e.to_string()))?,
        )
        .await
        .map_err(|e| TuiError::Client(e.message))?;

    let mut state = AppState {
        status: ConnStatus::Connected,
        ..AppState::default()
    };

    // Render loop: drain events into state, then render.
    let mut terminal = ratatui::init();
    let result = loop {
        match events.recv().await {
            Ok(push) => {
                state.push_event(push.event);
                if matches!(
                    state.deltas.back(),
                    Some(Event::RunFinished { .. } | Event::RunCancelled { .. })
                ) {
                    break Ok(());
                }
            }
            Err(_) => break Ok(()),
        }
        terminal
            .draw(|f| render(&state, f))
            .map_err(|e| TuiError::Render(e.to_string()))?;
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
