//! The TUI driver loop (F5.3, F11 opus review).

use futures_util::StreamExt;
use pii_proto::event::Event;
use pii_proto::sys::PeerInfo;
use ratatui::layout::Rect;

use crate::input::{KeyOutcome, handle_key, init_session};
use crate::render::render;
use crate::state::TuiError;

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

    use crate::input::{KeyOutcome, handle_slash_command};
    use crate::state::AppState;

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
