//! Free helpers the coder node shares: argument readers, the verdict and refusal outcomes, the journal line,
//! and the digests a spawn records.

use pii_abi::{NodeInput, Outcome, PortName};
use pii_core::{Ctx, ObsEvent};
use sha2::{Digest, Sha256};

use super::super::coder_skills::Rendered;
use pii_observe::{Event, Level, names};

pub(super) fn text(cfg: &serde_json::Value, key: &str) -> Option<String> {
    cfg.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

pub(super) fn texts(cfg: &serde_json::Value, key: &str) -> Vec<String> {
    cfg.get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The verdict of a node with no criteria of its own (spec 208): a clean exit hands on to a downstream `judge`,
/// and a non-zero exit goes to `err`, so the caller records it rather than the run failing outright.
pub(super) fn deferred_verdict(code: i32, pid: u32, agent: &str, model: &str) -> Outcome {
    if code == 0 {
        Outcome::Next {
            port: PortName::from("ok"),
            value: serde_json::json!({ "pid": pid, "rc": code, "criteria": [], "deferred": true }),
        }
    } else {
        refused_port(agent, model, &format!("worker exited {code}"))
    }
}

/// The `err` port a refusal routes to: the scheduler never retries a refusal (spec 207 R6).
pub(super) fn refused_port(agent: &str, model: &str, reason: &str) -> Outcome {
    Outcome::Next {
        port: PortName::from("err"),
        value: serde_json::json!({ "agent": agent, "model": model, "reason": reason }),
    }
}

/// A refusal `plan` already logged to the contract log: journalled for the run, then routed to `err` (spec 217 R5).
pub(super) fn refuse_planned(ctx: &Ctx, agent: &str, model: &str, reason: &str) -> Outcome {
    journal(
        ctx,
        &Event::new("coder", Level::Warn, names::WORKER_START_REFUSED)
            .field("agent", agent.to_owned())
            .field("model", model.to_owned())
            .field("reason", reason.to_owned()),
    );
    refused_port(agent, model, reason)
}

/// Reports an agent-call event to the run's context, where the engine records it in the journal (spec 217).
pub(super) fn journal(ctx: &Ctx, event: &Event) {
    ctx.observer().event(&ObsEvent::NodeObserved {
        kind: event.kind.to_owned(),
        fields: serde_json::to_value(&event.fields).unwrap_or_default(),
    });
}

/// The stop reason of a finished child (spec 217 R4).
///
/// A signal death is `128 + signal`, so a code above 128 is `signalled`; the node's own timeout kill is `timeout`;
/// any other code is the agent ending on its own.
pub fn stop_reason(code: i32) -> &'static str {
    if code == super::super::coder_runtime::TIMEOUT_EXIT_CODE {
        "timeout"
    } else if code > 128 {
        "signalled"
    } else {
        "end_turn"
    }
}

/// The sha256 of an argv as lowercase hex (spec 217 R3): identifies a call without journalling its arguments.
pub(super) fn argv_digest(argv: &[String]) -> String {
    let digest = Sha256::digest(argv.join("\0").as_bytes());
    format!("{digest:x}")
}

/// The pinned skills' names, comma-joined (spec 225 R7).
pub(super) fn skill_names(skills: &[Rendered]) -> String {
    skills
        .iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

/// The pinned skills' sha256s, comma-joined, in the order of [`skill_names`] (spec 225 R7).
pub(super) fn skill_hashes(skills: &[Rendered]) -> String {
    skills
        .iter()
        .map(|s| s.sha256.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

/// The session an agent was launched with, read from the argv it was spawned with.
///
/// The value after `--session` (pi), or after `--session-id` or `--resume` (claude), whichever flag comes first. Empty
/// when the argv names none (spec 244).
pub fn session_of(argv: &[String]) -> String {
    argv.windows(2)
        .find_map(|w| match w {
            [flag, value] if matches!(flag.as_str(), "--session" | "--session-id" | "--resume") => {
                Some(value.clone())
            }
            _ => None,
        })
        .unwrap_or_default()
}
pub(super) fn with_selected_spec(cfg: &serde_json::Value, input: &NodeInput) -> serde_json::Value {
    let mut cfg = cfg.clone();
    let selected = input
        .nodes
        .get("select")
        .and_then(|v| v.get("item"))
        .and_then(|i| i.get("spec"))
        .cloned();
    if let (Some(map), Some(spec)) = (cfg.as_object_mut(), selected) {
        map.entry("spec").or_insert(spec);
    }
    cfg
}
