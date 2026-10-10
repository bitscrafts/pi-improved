//! The observability seam of [`ToolSet`]: one event per reported tool call and
//! one per bash deny-list hit.

use std::sync::Arc;

use pii_abi::PiiError;
use pii_observe::{Event, Level, Observable, Observed, Observer, names};
use serde_json::Value;

use crate::ToolSet;

impl Observable for ToolSet {
    fn observer(&self) -> Option<&dyn Observer> {
        self.observer.as_deref()
    }
}

impl ToolSet {
    /// Attaches the observer that [`ToolSet::observe_call`] reports to.
    ///
    /// A builder rather than a constructor argument: `ToolSet::builtin` and
    /// `subset` are built at many call sites that have no observer yet.
    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn Observer>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Reports one finished invocation of tool `name`: a [`names::TOOLS_CALL`]
    /// event with the tool name and outcome (`ok` / `denied` / `error`), plus
    /// a [`names::TOOLS_DENY_LIST_HIT`] event when a bash command matched the
    /// deny list. A `Denied` result is also reported as a
    /// [`pii_core::ObsEvent::Denial`] on `ctx`'s observer. Never alters
    /// `result`; arguments and output are not logged.
    pub fn observe_call(
        &self,
        ctx: &pii_core::Ctx,
        name: &str,
        args: &Value,
        result: &Result<Value, PiiError>,
    ) {
        // A tool-level denial (deny list, cwd scope, http allowlist, ...) is
        // reported to the run's observer so it can reach the audit trail.
        if let Err(PiiError::Denied(why)) = result {
            ctx.observer().event(&pii_core::ObsEvent::Denial {
                principal: ctx.principal_str(),
                cap: format!("tool:{name}: {why}"),
            });
        }
        let outcome = match result {
            Ok(_) => "ok",
            Err(PiiError::Denied(_)) => "denied",
            Err(_) => "error",
        };
        let Some(tool) = self.get(name) else {
            return;
        };
        let deny_entry = (tool.name() == "bash")
            .then(|| {
                args.get("command")
                    .and_then(Value::as_str)
                    .and_then(crate::bash::deny_list_hit)
            })
            .flatten();
        // The run's observer carries the event to the daemon log (the
        // daemon's tool sets have no observer of their own).
        ctx.observer().event(&pii_core::ObsEvent::ToolCall {
            tool: tool.name().to_owned(),
            outcome: outcome.to_owned(),
            deny_entry: deny_entry.map(str::to_owned),
        });
        self.emit(
            &Event::new("tools", Level::Info, names::TOOLS_CALL)
                .field("tool", tool.name())
                .field("outcome", outcome),
        );
        if let Some(hit) = deny_entry {
            self.emit(
                &Event::new("tools", Level::Warn, names::TOOLS_DENY_LIST_HIT).field("entry", hit),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use pii_observe::Recorder;
    use serde_json::json;

    use super::*;

    #[test]
    fn call_and_deny_list_hit_reach_the_observer_by_name() {
        let rec = Arc::new(Recorder::new());
        let set = ToolSet::builtin(std::path::PathBuf::from(".")).with_observer(rec.clone());
        let args = json!({"command": "sudo ls"});
        let denied = Err(PiiError::Denied("deny list".into()));
        let ctx = pii_core::Ctx::with_observer(
            pii_core::CapSet::default(),
            pii_core::Budget::unlimited(),
            Arc::new(pii_core::NullObserver),
            pii_core::TraceCtx {
                run: pii_abi::RunId::from("r1"),
                span: 0,
            },
        );
        set.observe_call(&ctx, "bash", &args, &denied);
        let calls = rec.events_named(names::TOOLS_CALL);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].fields.get("tool"), Some(&json!("bash")));
        assert_eq!(calls[0].fields.get("outcome"), Some(&json!("denied")));
        let hits = rec.events_named(names::TOOLS_DENY_LIST_HIT);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].fields.get("entry"), Some(&json!("sudo")));
        set.observe_call(&ctx, "bash", &json!({"command": "ls"}), &Ok(json!({})));
        assert_eq!(rec.events_named(names::TOOLS_DENY_LIST_HIT).len(), 1);
        assert_eq!(rec.events_named(names::TOOLS_CALL).len(), 2);
    }
}
