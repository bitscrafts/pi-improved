//! The `route` node kind.
//!
//! Spec F4.5, widened by spec 392: `eq` compares, `when` computes a predicate in the graph. Split out of `nodes.rs` so that file stays under the line rule; `nodes::RouteNode` is re-exported.

use crate::expr::eval;
use crate::nodes::eval_ctx;
use pii_abi::{NodeInput, Outcome, PiiError, PortName};
use pii_core::Ctx;

/// One routing rule's test: `eq` compares against `on`, `when` is a predicate computed in the graph (spec 392), and a rule
/// with neither is the fallback.
enum RouteTest {
    Eq(serde_json::Value),
    When(crate::expr::Expr),
    Fallback,
}

/// `route`: takes `on` (expr) + `rules` (array of `{eq?: any, when?: expr, port: string}`).
pub struct RouteNode {
    on: String,
    rules: Vec<(RouteTest, PortName)>,
}

impl RouteNode {
    /// Builds from cfg.
    ///
    /// # Errors
    /// Returns [`PiiError::Invalid`] on a malformed config.
    pub fn new(cfg: &serde_json::Value) -> Result<Self, PiiError> {
        let on = cfg
            .get("on")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PiiError::Invalid("route requires 'on' expr string".into()))?
            .to_string();
        let rules = cfg
            .get("rules")
            .and_then(|v| v.as_array())
            .ok_or_else(|| PiiError::Invalid("route requires a 'rules' array".into()))?;
        let mut parsed = Vec::new();
        for r in rules {
            let test = match (r.get("eq"), r.get("when")) {
                (Some(_), Some(_)) => {
                    return Err(PiiError::Invalid(
                        "route rule sets 'eq' or 'when', not both".into(),
                    ));
                }
                (Some(eq), None) => RouteTest::Eq(eq.clone()),
                (None, Some(when)) => RouteTest::When(crate::expr::Expr::parse(
                    when.as_str().ok_or_else(|| {
                        PiiError::Invalid("route rule 'when' must be an expression string".into())
                    })?,
                )?),
                (None, None) => RouteTest::Fallback,
            };
            let port = PortName::from(
                r.get("port")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| PiiError::Invalid("route rule missing 'port' string".into()))?,
            );
            parsed.push((test, port));
        }
        if parsed.is_empty() {
            return Err(PiiError::Invalid(
                "route 'rules' must define at least one".into(),
            ));
        }
        Ok(Self { on, rules: parsed })
    }
}

/// Equality for a routing rule: strings compare trimmed, everything else compares exactly.
///
/// The trim is deliberate and load-bearing — a route fed by a command's stdout compares against a
/// value that carries a trailing newline (see `RouteNode::run`).
pub(crate) fn same_value(expected: &serde_json::Value, actual: &serde_json::Value) -> bool {
    match (expected.as_str(), actual.as_str()) {
        (Some(e), Some(a)) => e.trim() == a.trim(),
        _ => expected == actual,
    }
}

impl super::GraphNode for RouteNode {
    fn run(&self, _ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let ec = eval_ctx(input);
        let on_val = eval(&ec, &self.on)?;
        // The routed payload travels on the chosen port (not just `on_val`):
        // downstream agents/gates need the WHOLE input (test stdout/stderr
        // next to exit_code) to localize and re-route — spec F6.2's gate
        // feeds <test output> back to the fix agent.
        let mut port = None;
        for (test, candidate) in &self.rules {
            let hit = match test {
                // A routing value is compared TRIMMED when both sides are strings. A command's stdout
                // carries its trailing newline, and `say!` writes with `writeln!`, so an exact
                // comparison of "nothing-stale" against "nothing-stale\n" never matched: the books
                // graph's `nothing-to-do` port was unreachable and its coder ran on every invocation
                // (found by the 045 review, 2026-11-04). Trimming here fixes every graph that routes
                // on a command's output, rather than teaching each graph about newlines.
                RouteTest::Eq(expected) => same_value(expected, &on_val),
                RouteTest::When(when) => match when.eval(&ec)? {
                    serde_json::Value::Bool(hit) => hit,
                    other => {
                        return Err(PiiError::Invalid(format!(
                            "route: a 'when' must compute a boolean, got {other}"
                        )));
                    }
                },
                RouteTest::Fallback => true,
            };
            if hit {
                port = Some(candidate);
                break;
            }
        }
        port.map_or_else(
            || {
                Err(PiiError::Invalid(format!(
                    "route: no rule matched value {on_val} and no fallback"
                )))
            },
            |port| {
                Ok(Outcome::Next {
                    port: port.clone(),
                    value: input.payload.clone(),
                })
            },
        )
    }
}
