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

/// 2026-09-05: node-lifecycle events during an active run collapse into
/// a trailing "thinking" line (question + live journal seq in hex)
/// instead of one log line each (the user's explicit request:
/// "[agent]/[done] look unnecessary... add [an indicator] at the end of
/// the question" — then refined from dots to a hex seq number so it
/// doubles as a real debugging aid, cross-referenceable against `pii
/// graph trace <run_id>`'s own seq column). `push_prompt_echo` opens the
/// indicator; `push_event` on a terminal event (`RunFinished` here)
/// converts it back into a plain `you> <question>` line before logging
/// the final answer.
#[test]
fn thinking_indicator_collapses_into_the_question_line_then_settles() {
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
    state.push_prompt_echo(String::from("hello"));
    state.push_event(
        2,
        Event::NodeStarted {
            run_id: "r1".into(),
            node: "agent".into(),
        },
    );
    state.push_event(
        3,
        Event::NodeFinished {
            run_id: "r1".into(),
            node: "agent".into(),
            port: "ok".into(),
            value: serde_json::Value::Null,
        },
    );
    state.push_event(
        7,
        Event::RunFinished {
            run_id: "r1".into(),
            terminal: WireTerminal::Success {
                output: serde_json::json!({"text": "done"}),
            },
        },
    );

    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    terminal.draw(|f| render(&state, f)).expect("render");
    let buffer = terminal.backend().buffer().clone();
    let text = buffer_to_string(&buffer);

    // The question settles to a plain line (no per-node noise, no
    // leftover seq indicator) once the run finishes, and the final
    // answer shows.
    assert!(text.contains("hello"), "the question should render: {text}");
    assert!(
        text.contains("done"),
        "the final answer should render: {text}"
    );
    assert!(
        !text.contains("[agent]"),
        "per-node lifecycle lines should no longer render: {text}"
    );
    assert!(
        !text.contains("seq"),
        "a settled question must not show a stale seq indicator: {text}"
    );
    assert!(text.contains("s1"), "session id should render: {text}");
    assert!(text.contains("connected"), "status should render: {text}");
}

/// 2026-09-05: while a run is active (no terminal event yet), the
/// trailing line shows the question with the LATEST node-lifecycle
/// event's journal seq in hex — this is the actual "thinking" indicator
/// the user asked for, verified mid-run rather than only after it
/// settles, and checked against the exact hex value (not just "some
/// dots") since the whole point is a debuggable, real identifier.
#[test]
fn thinking_indicator_shows_the_latest_seq_while_a_run_is_active() {
    let mut state = AppState::default();
    state.push_prompt_echo(String::from("why is the sky blue?"));
    state.push_event(
        5,
        Event::NodeStarted {
            run_id: "r1".into(),
            node: "agent".into(),
        },
    );
    state.push_event(
        0x1a,
        Event::ToolCall {
            run_id: "r1".into(),
            node: "agent".into(),
            tool: "fs_read".into(),
            args: serde_json::Value::Null,
        },
    );

    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    terminal.draw(|f| render(&state, f)).expect("render");
    let buffer = terminal.backend().buffer().clone();
    let text = buffer_to_string(&buffer);

    assert!(
        text.contains("why is the sky blue? [seq 0x1a]"),
        "the trailing line must show the latest event's seq in hex: {text}"
    );
}

#[test]
fn app_state_caps_deltas_at_500() {
    let mut state = AppState::default();
    for i in 0..600 {
        // Each iteration opens a fresh Thinking line and immediately
        // settles it via a terminal event, so every iteration produces
        // exactly one permanent log line (matches how a real session
        // grows the log: one line per completed prompt, not one per
        // node-lifecycle event).
        state.push_prompt_echo(format!("q{i}"));
        state.push_event(
            u64::try_from(i).unwrap_or(0),
            Event::RunFinished {
                run_id: "r1".into(),
                terminal: WireTerminal::Success {
                    output: serde_json::json!({"text": format!("a{i}")}),
                },
            },
        );
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
