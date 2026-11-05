//! Built-in node kinds (spec F4.5).

use crate::expr::{eval, eval_or_literal};
use pii_abi::{NodeInput, Outcome, PiiError, PortName, ResumeKey};
use pii_core::Ctx;

/// Build the evaluation context object for a node:
/// `{ "input": ..., "payload": ..., "nodes": ... }`.
pub(crate) fn eval_ctx(input: &NodeInput) -> serde_json::Value {
    serde_json::json!({
        "input": input.input,
        "payload": input.payload,
        "nodes": input.nodes,
    })
}

/// `transform`: takes `expr` (string) or `value` (any).
///
/// A malformed expr returns `Next{err, {"error": ..}}`; otherwise
/// `Next{ok, eval_or_literal(...)}`.
pub struct TransformNode {
    expr: Option<String>,
    value: Option<serde_json::Value>,
    /// spec 383: `let` binds names, `out` computes one value from them. Before this, a transform could
    /// only read a member of its input, so anything arithmetic had to become a shell string in the graph
    /// file -- invisible, untestable, and unpinned. The bindings are evaluated in order, each seeing the
    /// ones before it, and `out` sees all of them plus the input.
    lets: Vec<(String, crate::expr::Expr)>,
    out: Option<crate::expr::Expr>,
    /// the node's own id, so a load-time refusal can name it ("node 'calc': bad expression ...")
    node_id: String,
}

impl TransformNode {
    /// Builds from cfg.
    ///
    /// # Errors
    /// Returns [`PiiError::Invalid`] if neither `expr` nor `value` is present.
    pub fn new(cfg: &serde_json::Value) -> Result<Self, PiiError> {
        let expr = cfg.get("expr").and_then(|v| v.as_str()).map(str::to_string);
        let value = cfg.get("value").cloned();
        let out = cfg.get("out").and_then(|v| v.as_str()).map(str::to_string);
        let mut lets: Vec<(String, String)> = Vec::new();
        if let Some(tbl) = cfg.get("let") {
            // The shipped form is an ARRAY of "name = expression", in the order they are evaluated
            // (`let` is a sequence, not a map: a later binding may read an earlier one, and TOML tables
            // carry no order). The object form is accepted for readability of single bindings.
            match tbl {
                serde_json::Value::Array(items) => {
                    for (i, item) in items.iter().enumerate() {
                        let Some(line) = item.as_str() else {
                            return Err(PiiError::Invalid(format!(
                                "transform 'let[{i}]' must be a string \"name = expression\""
                            )));
                        };
                        let Some((name, e)) = line.split_once('=') else {
                            return Err(PiiError::Invalid(format!(
                                "transform 'let[{i}]' has no '=': {line}"
                            )));
                        };
                        let name = name.trim();
                        if name.is_empty() {
                            return Err(PiiError::Invalid(format!(
                                "transform 'let[{i}]' names nothing: {line}"
                            )));
                        }
                        lets.push((name.to_string(), e.trim().to_string()));
                    }
                }
                serde_json::Value::Object(map) => {
                    for (k, v) in map {
                        let Some(e) = v.as_str() else {
                            return Err(PiiError::Invalid(format!(
                                "transform 'let.{k}' must be an expression string"
                            )));
                        };
                        lets.push((k.clone(), e.to_string()));
                    }
                }
                other => {
                    return Err(PiiError::Invalid(format!(
                        "transform 'let' must be an array of \"name = expression\" or a table; got {other}"
                    )));
                }
            }
        }
        if out.is_some() && (expr.is_some() || value.is_some()) {
            return Err(PiiError::Invalid(
                "transform sets 'out' or 'expr'/'value', not both".into(),
            ));
        }
        if out.is_none() && !lets.is_empty() {
            return Err(PiiError::Invalid(
                "transform has 'let' but no 'out': the bindings would compute nothing".into(),
            ));
        }
        if expr.is_none() && value.is_none() && out.is_none() {
            return Err(PiiError::Invalid(
                "transform requires 'expr' (string), 'value' (any), or 'let' plus 'out'".into(),
            ));
        }
        let node_id = cfg
            .get("node-id")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        // LOAD TIME: every binding and `out` is parsed now, and a name is refused unless an earlier
        // binding introduced it. An expression that only fails when a run reaches it is a graph that
        // shipped broken — the same trap as a `$.` path that names nothing.
        let mut parsed: Vec<(String, crate::expr::Expr)> = Vec::new();
        let mut bound: Vec<String> = Vec::new();
        for (name, e) in &lets {
            let ast = crate::expr::Expr::parse(e)
                .map_err(|err| PiiError::Invalid(format!("node '{node_id}': {err}")))?;
            for used in ast.names() {
                if !bound.iter().any(|b| b == used) {
                    return Err(PiiError::Invalid(format!(
                        "node '{node_id}': bad expression '{e}': name '{used}' is not bound by an earlier let"
                    )));
                }
            }
            bound.push(name.clone());
            parsed.push((name.clone(), ast));
        }
        let out_ast = match &out {
            Some(o) => {
                let ast = crate::expr::Expr::parse(o)
                    .map_err(|err| PiiError::Invalid(format!("node '{node_id}': {err}")))?;
                for used in ast.names() {
                    if !bound.iter().any(|b| b == used) {
                        return Err(PiiError::Invalid(format!(
                            "node '{node_id}': bad expression '{o}': name '{used}' is not bound by an earlier let"
                        )));
                    }
                }
                Some(ast)
            }
            None => None,
        };
        Ok(Self {
            expr,
            value,
            lets: parsed,
            out: out_ast,
            node_id,
        })
    }
}

impl super::GraphNode for TransformNode {
    fn run(&self, _ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let ec = eval_ctx(input);
        let result = if let Some(out) = &self.out {
            // bindings are evaluated in declaration order, each against a context that has the input plus
            // every binding so far -- so `let m = n + 1` can read `n`, and `out = m` reads both
            let mut ctx = ec.clone();
            let mut failed = None;
            for (name, ast) in &self.lets {
                match ast.eval(&ctx) {
                    Ok(v) => {
                        if let Some(obj) = ctx.as_object_mut() {
                            obj.insert(name.clone(), v);
                        }
                    }
                    Err(err) => {
                        failed = Some(err);
                        break;
                    }
                }
            }
            match failed {
                Some(e) => Err(e),
                None => out.eval(&ctx),
            }
        } else {
            match (&self.expr, &self.value) {
                (Some(e), _) => eval(&ec, e),
                (None, Some(v)) => eval_or_literal(&ec, v),
                (None, None) => Ok(serde_json::Value::Null),
            }
        };
        match result {
            Ok(v) => Ok(Outcome::Next {
                port: PortName::from("ok"),
                value: v,
            }),
            Err(e) => Ok(Outcome::Next {
                port: PortName::from("err"),
                value: serde_json::json!({ "error": e.to_string() }),
            }),
        }
    }
}

/// `route`: takes `on` (expr) + `rules` (array of `{eq?: any, port: string}`).
pub struct RouteNode {
    on: String,
    rules: Vec<(Option<serde_json::Value>, PortName)>,
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
            let eq = r.get("eq").cloned();
            let port = PortName::from(
                r.get("port")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| PiiError::Invalid("route rule missing 'port' string".into()))?,
            );
            parsed.push((eq, port));
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
fn same_value(expected: &serde_json::Value, actual: &serde_json::Value) -> bool {
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
        let port = self.rules.iter().find_map(|(eq, port)| match eq {
            // A routing value is compared TRIMMED when both sides are strings. A command's stdout
            // carries its trailing newline, and `say!` writes with `writeln!`, so an exact
            // comparison of "nothing-stale" against "nothing-stale\n" never matched: the books
            // graph's `nothing-to-do` port was unreachable and its coder ran on every invocation
            // (found by the 045 review, 2026-11-04). Trimming here fixes every graph that routes
            // on a command's output, rather than teaching each graph about newlines.
            Some(expected) if same_value(expected, &on_val) => Some(port),
            Some(_) => None,
            // fallback rule (no eq)
            None => Some(port),
        });
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

/// `emit`: takes `path` (expr-or-literal), `mode` (write|append), `value`.
pub struct EmitNode {
    path: serde_json::Value,
    mode: EmitMode,
    value: serde_json::Value,
}

enum EmitMode {
    Write,
    Append,
}

impl EmitNode {
    /// Builds from cfg.
    ///
    /// # Errors
    /// Returns [`PiiError::Invalid`] on a malformed config.
    pub fn new(cfg: &serde_json::Value) -> Result<Self, PiiError> {
        let path = cfg
            .get("path")
            .cloned()
            .ok_or_else(|| PiiError::Invalid("emit requires 'path'".into()))?;
        let value = cfg
            .get("value")
            .cloned()
            .ok_or_else(|| PiiError::Invalid("emit requires 'value'".into()))?;
        let mode = match cfg.get("mode").and_then(|v| v.as_str()).unwrap_or("write") {
            "write" => EmitMode::Write,
            "append" => EmitMode::Append,
            other => {
                return Err(PiiError::Invalid(format!(
                    "emit 'mode' must be write|append, got '{other}'"
                )));
            }
        };
        Ok(Self { path, mode, value })
    }
}

impl super::GraphNode for EmitNode {
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let ec = eval_ctx(input);
        let path_v = eval_or_literal(&ec, &self.path)?;
        let path_s = path_v
            .as_str()
            .ok_or_else(|| PiiError::Invalid("emit 'path' did not resolve to a string".into()))?;
        let value_v = eval_or_literal(&ec, &self.value)?;
        // JSON strings are written raw (without quotes).
        let bytes = match &value_v {
            serde_json::Value::String(s) => s.clone().into_bytes(),
            other => other.to_string().into_bytes(),
        };
        let fs = ctx.fs()?;
        match self.mode {
            EmitMode::Write => fs.write(std::path::Path::new(path_s), &bytes)?,
            EmitMode::Append => fs.append(std::path::Path::new(path_s), &bytes)?,
        }
        Ok(Outcome::Next {
            port: PortName::from("ok"),
            value: value_v,
        })
    }
}

/// `wait`: takes `timer_ms` (u64). Sleeps in 5 ms slices, checking cancellation.
pub struct WaitNode {
    timer_ms: u64,
}

impl WaitNode {
    /// Builds from cfg.
    ///
    /// # Errors
    /// Returns [`PiiError::Invalid`] if `timer_ms` is missing.
    pub fn new(cfg: &serde_json::Value) -> Result<Self, PiiError> {
        let timer_ms = cfg
            .get("timer_ms")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| PiiError::Invalid("wait requires 'timer_ms' (u64)".into()))?;
        Ok(Self { timer_ms })
    }
}

impl super::GraphNode for WaitNode {
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let mut elapsed = 0u64;
        while elapsed < self.timer_ms {
            if ctx.cancelled() {
                return Err(PiiError::Cancelled);
            }
            let slice = 5u64.min(self.timer_ms - elapsed);
            std::thread::sleep(std::time::Duration::from_millis(slice));
            elapsed += slice;
        }
        if ctx.cancelled() {
            return Err(PiiError::Cancelled);
        }
        Ok(Outcome::Next {
            port: PortName::from("fired"),
            value: input.payload.clone(),
        })
    }
}

/// `refuse` (spec 212): fails the run by name.
///
/// `hook` is stamped by the weave from the attach id; `reason` is declared by the
/// fragment. `returns` is set by the load when a host wires this refusal (spec 364):
/// the verdict goes back to the host on `ok`, under that port's name, instead of
/// failing the run.
pub struct RefuseNode {
    hook: String,
    reason: String,
    returns: Option<String>,
}

impl RefuseNode {
    /// Builds from cfg; both fields default to a placeholder rather than failing.
    ///
    /// # Errors
    /// Never fails today; the `Result` keeps the factory signature uniform.
    pub fn new(cfg: &serde_json::Value) -> Result<Self, PiiError> {
        let field = |key: &str, default: &str| {
            cfg.get(key)
                .and_then(|v| v.as_str())
                .unwrap_or(default)
                .to_owned()
        };
        Ok(Self {
            hook: field("hook", "unnamed"),
            reason: field("reason", "refused"),
            returns: cfg
                .get("returns")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
        })
    }
}

impl super::GraphNode for RefuseNode {
    fn run(&self, _ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let Some(port) = &self.returns else {
            return Ok(Outcome::Fail {
                error: format!("run.refused{{hook={}, reason={}}}", self.hook, self.reason),
            });
        };
        // A join's payload is keyed by source, and the arm that arrived says why.
        let arm = input
            .payload
            .as_object()
            .and_then(|arms| arms.values().find(|v| v.get("reason").is_some()))
            .unwrap_or(&input.payload);
        let why = arm.get("reason").and_then(|v| v.as_str());
        let reason = why.map_or_else(
            || format!("{port}: {}", self.reason),
            |why| format!("{port}: {}: {why}", self.reason),
        );
        let criterion = arm.pointer("/failing/0").cloned();
        Ok(Outcome::Next {
            port: PortName::from("ok"),
            value: serde_json::json!({
                "verdict": "refused",
                "port": port,
                "reason": reason,
                "criterion": criterion,
            }),
        })
    }
}

/// `timer` (F5.6): cfg `{ in_ms: u64 }` or `{ cron: "<5 fields>" }`.
///
/// Returns
/// `Outcome::Suspend { reason: Timer { in_ms }, key: "timer:<node>:<fire>" }`
/// — the scheduler wheel owns the actual waiting. The blocking `wait` node
/// is unchanged; this kind never blocks.
#[derive(Debug)]
pub struct TimerNode {
    /// Delay for the plain form (0 = cron-driven).
    in_ms: u64,
    /// The parsed cron expression (`None` for the plain `in_ms` form).
    cron: Option<crate::cron::Cron>,
    /// Raw cron spec (for the key).
    #[allow(dead_code)] // reason: cron-sourced timers land with the
    // scheduler pass (F5.x); the parse keeps TOML load total.
    cron_spec: Option<String>,
}

impl TimerNode {
    /// Builds from cfg.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] when neither `in_ms` nor a valid `cron` is given.
    pub fn new(cfg: &serde_json::Value) -> Result<Self, PiiError> {
        let in_ms = cfg.get("in_ms").and_then(serde_json::Value::as_u64);
        let cron_spec = cfg
            .get("cron")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        if in_ms.is_none() && cron_spec.is_none() {
            return Err(PiiError::Invalid(
                "timer requires 'in_ms' (u64) or 'cron' (5-field string)".into(),
            ));
        }
        let parsed = match &cron_spec {
            Some(spec) => Some(crate::cron::Cron::parse(spec)?),
            None => None,
        };
        Ok(Self {
            in_ms: in_ms.unwrap_or(0),
            cron: parsed,
            cron_spec,
        })
    }

    /// The next fire delay in ms: `in_ms`, or the cron's next fire from `now`.
    #[must_use]
    pub fn next_in_ms(&self, now_ms: u64) -> u64 {
        self.cron
            .as_ref()
            .map_or(self.in_ms, |c| c.next_after(now_ms).saturating_sub(now_ms))
    }
}

impl super::GraphNode for TimerNode {
    fn run(&self, ctx: &Ctx, _input: &NodeInput) -> Result<Outcome, PiiError> {
        let now_ms = ctx.now_ms();
        let in_ms = self.next_in_ms(now_ms);
        let fire_at_ms = now_ms.saturating_add(in_ms);
        // The key must be unique per (run, node, fire time): two runs whose
        // timers happen to compute the same delay must not collide in the
        // resume index (F5.6).
        let key = match ctx.principal() {
            pii_core::Principal::GraphNode { run, node } => {
                format!("timer:{run}:{node}:{fire_at_ms}")
            }
            _ => format!("timer:{fire_at_ms}"),
        };
        Ok(Outcome::Suspend {
            reason: pii_abi::SuspendReason::Timer { in_ms },
            key: ResumeKey(key),
        })
    }

    /// Resumes to `fired` once the key's `fire_at_ms` (its trailing
    /// `:`-separated field, per the format above) has passed; otherwise
    /// `None` re-suspends via a fresh [`Self::run`] dispatch (F5.6/spec 30
    /// R1).
    fn on_resume(&self, ctx: &Ctx, input: &NodeInput, key: &ResumeKey) -> Option<Outcome> {
        let fire_at_ms: u64 = key.0.rsplit(':').next()?.parse().ok()?;
        if ctx.now_ms() < fire_at_ms {
            return None;
        }
        Some(Outcome::Next {
            port: PortName::from("fired"),
            value: input.payload.clone(),
        })
    }
}

#[cfg(test)]
mod trim_tests {
    use super::same_value;
    use serde_json::json;

    #[test]
    fn a_command_stdout_with_its_newline_still_matches() {
        assert!(same_value(
            &json!("nothing-stale"),
            &json!("nothing-stale\n")
        ));
    }

    #[test]
    fn near_miss_a_different_word_does_not_match() {
        assert!(!same_value(
            &json!("nothing-stale"),
            &json!("two-chapters\n")
        ));
    }

    #[test]
    fn non_strings_still_compare_exactly() {
        assert!(same_value(&json!(0), &json!(0)));
        assert!(!same_value(&json!(0), &json!("0")));
    }
}
