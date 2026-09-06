//! TUI render tests (F5.4, part-3 EC4): `render` is pure and testable
//! against a `TestBackend`; `AppState` holds only protocol-derived fields.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_proto::event::{Event, WireTerminal};
use pii_proto::session::{SessionStateResult, SessionStatus, WireMessage};
use pii_tui::{AppState, ConnStatus, render};

#[test]
fn renders_snapshot_without_engine() {
    let mut state = AppState {
        status: ConnStatus::Connected,
        snapshot: Some(SessionStateResult {
            session_id: "s1".into(),
            cwd: "/".into(),
            status: SessionStatus::Running,
            messages: vec![WireMessage {
                role: "user".into(),
                content: "hello".into(),
            }],
            tokens_in: 0,
            tokens_out: 0,
            run_id: Some("r1".into()),
        }),
        ..AppState::default()
    };
    state.push_event(Event::NodeStarted {
        run_id: "r1".into(),
        node: "agent".into(),
    });
    state.push_event(Event::RunFinished {
        run_id: "r1".into(),
        terminal: WireTerminal::Success {
            output: serde_json::json!({"text": "done"}),
        },
    });

    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    terminal.draw(|f| render(&state, f)).expect("render");
    let buffer = terminal.backend().buffer().clone();
    let text = buffer_to_string(&buffer);

    // The assistant text and node names appear in the buffer.
    assert!(text.contains("agent"), "node name should render: {text}");
    assert!(text.contains("s1"), "session id should render: {text}");
    assert!(text.contains("connected"), "status should render: {text}");
}

#[test]
fn app_state_caps_deltas_at_500() {
    let mut state = AppState::default();
    for i in 0..600 {
        state.push_event(Event::NodeStarted {
            run_id: "r1".into(),
            node: format!("n{i}"),
        });
    }
    assert_eq!(state.deltas.len(), 500, "delta buffer must cap at 500");
}

/// 2026-09-05: a submitted prompt must be visible in the rendered main
/// pane (the exact gap the user reported: "no command echo on main
/// pane"), not silently vanish from the input box.
#[test]
fn prompt_echo_renders_in_the_body() {
    let mut state = AppState::default();
    state.push_prompt_echo(String::from("what is 3 times 2?"));

    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    terminal.draw(|f| render(&state, f)).expect("render");
    let buffer = terminal.backend().buffer().clone();
    let text = buffer_to_string(&buffer);

    assert!(
        text.contains("what is 3 times 2?"),
        "the submitted prompt must be echoed into the log: {text}"
    );
}

/// 2026-09-05: a local system message (e.g. slash-command feedback) must
/// also render in the body.
#[test]
fn system_message_renders_in_the_body() {
    let mut state = AppState::default();
    state.push_system(String::from("unknown command: /bogus (try /help)"));

    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    terminal.draw(|f| render(&state, f)).expect("render");
    let buffer = terminal.backend().buffer().clone();
    let text = buffer_to_string(&buffer);

    assert!(
        text.contains("unknown command: /bogus"),
        "system messages must render: {text}"
    );
}

fn buffer_to_string(buffer: &ratatui::buffer::Buffer) -> String {
    let mut out = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = buffer.cell((x, y)).expect("cell");
            out.push(cell.symbol().chars().next().unwrap_or(' '));
        }
        out.push('\n');
    }
    out
}
