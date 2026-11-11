//! Built-in node kinds (spec F4.5).

use crate::expr::{eval, eval_or_literal};
use pii_abi::{NodeInput, Outcome, PiiError, PortName, ResumeKey};
use pii_core::Ctx;

pub use crate::nodes_route::RouteNode;

mod refuse;
pub use refuse::RefuseNode;

/// Build the evaluation context object for a node:
/// `{ "input": ..., "payload": ..., "nodes": ... }`.
pub(crate) fn eval_ctx(input: &NodeInput) -> serde_json::Value {
    serde_json::json!({
        "input": input.input,
        "payload": input.payload,
        "nodes": input.nodes,
    })
}

/// Programs that read a command string or re-exec one, matched on the final path component. ONE list for the graph loader and the
/// SOP lint, so a shell cannot be refused in one and accepted in the other.
pub(crate) const SHELLS: &[&str] = &["sh", "bash", "dash", "zsh", "ksh", "fish", "env"];

/// Spec 400 R1/R2: a `run` node's `programs` is a literal, non-empty array of program names, and none of them is a shell.
/// An expression, a string or a table is refused, so run input can never supply the program; a shell is refused by its
/// final path component, so `/bin/bash` is one too.
fn check_declared_programs(node: &crate::spec::NodeSpec) -> Result<(), PiiError> {
    let refuse = |why: &str| {
        PiiError::Invalid(format!(
            "tool.run.programs: node '{}' {why}; `programs` is a literal array of bare program names in this file",
            node.id
        ))
    };
    let Some(programs) = node
        .cfg
        .get("programs")
        .and_then(serde_json::Value::as_array)
    else {
        return Err(refuse(
            "has `tool` = \"run\" without a literal array `programs`",
        ));
    };
    if programs.is_empty() {
        return Err(refuse("declares no program"));
    }
    for entry in programs {
        let Some(name) = entry
            .as_str()
            .filter(|n| !n.trim().is_empty() && !n.starts_with("$."))
        else {
            return Err(refuse(&format!(
                "has the `programs` entry {entry}, which is not a literal name"
            )));
        };
        let program = name.rsplit('/').next().unwrap_or(name);
        if SHELLS.contains(&program) {
            return Err(refuse(&format!(
                "declares the shell `{name}`; a `tool` node runs a program, not a shell"
            )));
        }
    }
    Ok(())
}

/// Spec 393 R3: a `tool` node's `run` is an argv array whose first entry is a literal program, never a shell. A shell
/// string, a `$.` argv0 and `["bash", "-c", ..]` are all refused, naming the node: authority comes from the committed
/// spec, not from the graph.
pub(crate) fn check_tool_surface(nodes: &[crate::spec::NodeSpec]) -> Result<(), PiiError> {
    for node in nodes.iter().filter(|n| n.kind == "tool") {
        // Spec 399 R3/R8: `bash` is the shell itself, so a graph `tool` node never names it; `run` must declare the
        // programs it may exec, because its argv can arrive as run input where this check cannot see it.
        match node.cfg.get("tool").and_then(serde_json::Value::as_str) {
            Some("bash") => {
                return Err(PiiError::Invalid(format!(
                    "tool.bash.shell: node '{}' has `tool` = \"bash\"; a graph `tool` node runs a program, not a \
                     shell: use `tool = \"run\"` with an argv array",
                    node.id
                )));
            }
            Some("run") => check_declared_programs(node)?,
            _ => {}
        }
        let Some(run) = node.cfg.get("run") else {
            continue;
        };
        let argv: Option<Vec<&str>> = run
            .as_array()
            .and_then(|items| items.iter().map(serde_json::Value::as_str).collect());
        let argv0 = argv.as_ref().and_then(|a| a.first()).copied();
        let literal = argv0.is_some_and(|p| {
            let program = p.rsplit('/').next().unwrap_or(p);
            !p.starts_with("$.")
                && !program.is_empty()
                && !p.contains(char::is_whitespace)
                && !SHELLS.contains(&program)
        });
        if !literal {
            return Err(PiiError::Invalid(format!(
                "tool.run.shell: node '{}' has `run` = {run}; `run` takes an argv array with a literal argv0 and \
                 no shell",
                node.id
            )));
        }
    }
    Ok(())
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
        let lets = Self::parse_lets(cfg)?;
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
        })
    }

    /// Parses `let`: an ARRAY of `"name = expression"` (a sequence, because a later binding may
    /// read an earlier one and TOML tables carry no order) or a table for a single binding.
    fn parse_lets(cfg: &serde_json::Value) -> Result<Vec<(String, String)>, PiiError> {
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
        Ok(lets)
    }
}

impl TransformNode {
    /// Evaluates `let` bindings in declaration order, each against a context holding the input plus every
    /// binding so far, and then evaluates `out` against all of them.
    ///
    /// The first failing binding is the run's failure: a later binding would otherwise read a value that
    /// was never computed, which is the kind of quiet wrong answer this language exists to refuse.
    fn bind_then_eval(
        &self,
        ec: &serde_json::Value,
        out: &crate::expr::Expr,
    ) -> Result<serde_json::Value, PiiError> {
        let mut ctx = ec.clone();
        for (name, ast) in &self.lets {
            let v = ast.eval(&ctx)?;
            if let Some(obj) = ctx.as_object_mut() {
                obj.insert(name.clone(), v);
            }
        }
        out.eval(&ctx)
    }
}

impl super::GraphNode for TransformNode {
    fn run(&self, _ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let ec = eval_ctx(input);
        let result = match (&self.out, &self.expr, &self.value) {
            (Some(out), _, _) => self.bind_then_eval(&ec, out),
            (None, Some(e), _) => eval(&ec, e),
            (None, None, Some(v)) => eval_or_literal(&ec, v),
            (None, None, None) => Ok(serde_json::Value::Null),
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
    use crate::nodes_route::same_value;
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
