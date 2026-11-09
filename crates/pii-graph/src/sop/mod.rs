//! SOP engine (spec 113): a declarative procedure that compiles into the graph.
//!
//! The SOP says WHAT happens, in order; the graph machinery (journal, resume,
//! `Suspend`, capability-gated `Ctx`, the approval store) does HOW. There is no
//! second execution engine here: [`compile`] emits graph TOML and nothing else.
//!
//! Format (TOML): `id`, `title`, `caps`, and ordered `[[step]]` entries with
//! `id`, `kind` (`act` | `check` | `approve` | `notify`), optional `requires`
//! (ids of steps that must have finished) and the per-kind input
//! (`prompt` / `argv` / `prompt` / `message`).

mod compile;
mod lint;

use std::collections::BTreeSet;

use serde::Deserialize;

pub use compile::compile;

/// What a step does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    /// An agent turn driven by `prompt`.
    Act,
    /// A verifier: the program in `argv` must exit 0 or the run fails naming the step.
    Check,
    /// A human gate: the run suspends until `pii approve allow|deny`.
    Approve,
    /// A message out: `message` appended to `path`.
    Notify,
}

impl StepKind {
    /// The name used in the SOP file and in event fields.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Act => "act",
            Self::Check => "check",
            Self::Approve => "approve",
            Self::Notify => "notify",
        }
    }
}

/// One validated step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Unique step id (also the main graph node's id).
    pub id: String,
    /// What the step does.
    pub kind: StepKind,
    /// Ids of steps that must have finished before this one starts.
    pub requires: Vec<String>,
    /// `act` / `approve`: the prompt.
    pub prompt: Option<String>,
    /// `check`: the program and its arguments; the first entry is the program. Never a shell string (spec 399).
    pub argv: Vec<String>,
    /// `notify`: the message.
    pub message: Option<String>,
    /// `notify`: the file the message is appended to.
    pub path: Option<String>,
    /// `act`: the tools the agent may use.
    pub tools: Vec<String>,
    /// `act`: the agent's turn bound.
    pub max_turns: Option<u32>,
    /// `approve`: how long the run may wait for a decision, in milliseconds (a suspension needs an end).
    pub deadline_ms: Option<u64>,
}

/// A validated SOP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sop {
    /// The SOP id.
    pub id: String,
    /// A human title.
    pub title: String,
    /// Capability names, sorted and de-duplicated.
    pub caps: Vec<String>,
    /// Steps in declaration order.
    pub steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSop {
    id: Option<String>,
    title: Option<String>,
    #[serde(default)]
    caps: Vec<String>,
    #[serde(default)]
    step: Vec<RawStep>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStep {
    id: Option<String>,
    kind: Option<String>,
    #[serde(default)]
    requires: Vec<String>,
    prompt: Option<String>,
    /// Kept only to refuse it by name: a `command` string used to be a shell string (spec 399).
    command: Option<String>,
    argv: Option<Vec<String>>,
    message: Option<String>,
    path: Option<String>,
    #[serde(default)]
    tools: Vec<String>,
    max_turns: Option<u32>,
    deadline_ms: Option<u64>,
}

/// Parses and lints an SOP document.
///
/// # Errors
/// Every problem found, one message each (a parse error is a single message).
pub fn load(src: &str) -> Result<Sop, Vec<String>> {
    let raw: RawSop = toml::from_str(src).map_err(|e| vec![format!("sop toml parse: {e}")])?;
    let errors = lint::check(&raw);
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(lint::build(raw))
}

/// True for a node/step id made only of `[A-Za-z0-9_-]` (no `.`: the compiler
/// uses `.` to derive its helper node ids).
fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn sorted_unique(items: &[String]) -> Vec<String> {
    items
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
