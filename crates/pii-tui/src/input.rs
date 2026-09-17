//! Input handling (2026-09-05, extracted from `run_tui` to keep it under
//! the line-count lint).
//!
//! `mod input` is private to the crate (see `lib.rs`), so its items only
//! need `pub(crate)` visibility to be reachable from sibling modules
//! (`driver.rs`) -- but clippy's `redundant_pub_crate` flags `pub(crate)`
//! on an item inside an already-private module as redundant (a private
//! module's items are already crate-visible via the module path). Keep
//! `pub(crate)` for readability/intent at the call sites rather than
//! bumping to full `pub`, which would make these implementation details
//! part of the crate's public API.
#![allow(clippy::redundant_pub_crate)]

use pii_proto::method::Method;
use pii_proto::session::SessionCreateParams;

use crate::state::{AppState, ConnStatus, TuiError};

/// Sends one prompt on `session_id` and subscribes to its run's events
/// (2026-09-05, extracted from `run_tui` to keep it under the line-count
/// lint — this is the same two-call sequence the original one-shot
/// version did once at startup, now callable per Enter keypress).
///
/// # Errors
/// Returns [`TuiError`] on protocol or transport failure.
pub(crate) async fn submit_prompt(
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
            serde_json::to_value(pii_proto::event::EventFilter {
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
pub(crate) enum KeyOutcome {
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
pub(crate) fn handle_slash_command(text: &str, state: &mut AppState) -> Option<KeyOutcome> {
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
pub(crate) async fn handle_key(
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
pub(crate) async fn init_session(
    client: &pii_proto::client::Client,
) -> Result<(AppState, String), TuiError> {
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
