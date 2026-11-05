//! Spec 386: a node may carry a `prompt` template.
//!
//! Its `{{path}}` placeholders name paths the graph can supply, and are checked at load; a template is resolved against
//! the same eval context a node already uses.

use pii_abi::PiiError;
use serde_json::Value;

use crate::expr::eval;
use crate::spec::GraphSpec;

/// The largest prompt a node may carry, in bytes. A graph cannot smuggle a file into an instruction past this.
pub const PROMPT_CAP_BYTES: usize = 64 * 1024;

/// The node kinds that may carry a prompt. A command's text is not a prompt, so `tool` is not among them.
const PROMPT_KINDS: [&str; 1] = ["coder"];

/// The `path` of each `{{path}}` in `template`, in order.
///
/// # Errors
/// [`PiiError::Invalid`] when a placeholder is never closed or names nothing.
pub fn placeholders(template: &str) -> Result<Vec<&str>, PiiError> {
    let mut paths = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let end = after
            .find("}}")
            .ok_or_else(|| PiiError::Invalid("unclosed placeholder '{{'".into()))?;
        let path = after[..end].trim();
        if path.is_empty() {
            return Err(PiiError::Invalid("empty placeholder '{{}}'".into()));
        }
        paths.push(path);
        rest = &after[end + 2..];
    }
    Ok(paths)
}

/// Checks every node's `prompt` at load.
///
/// The kind may carry one, `brief` is absent, the text is within the cap, and each placeholder names a path the graph
/// can supply. `needs` is the input the graph declares it reads.
///
/// # Errors
/// [`PiiError::Invalid`] naming the node, and the placeholder or cap at fault.
pub fn check(spec: &GraphSpec, needs: &[String]) -> Result<(), PiiError> {
    for node in &spec.nodes {
        let Some(prompt) = node.cfg.get("prompt") else {
            continue;
        };
        let id = node.id.as_str();
        if !PROMPT_KINDS.contains(&node.kind.as_str()) {
            return Err(PiiError::Invalid(format!(
                "node '{id}' (kind '{}') carries a prompt: prompts belong to agent nodes ({})",
                node.kind,
                PROMPT_KINDS.join(", ")
            )));
        }
        if node.cfg.get("brief").is_some() {
            return Err(PiiError::Invalid(format!(
                "node '{id}' declares both 'prompt' and 'brief': a node says where its instruction comes from once"
            )));
        }
        let template = prompt
            .as_str()
            .ok_or_else(|| PiiError::Invalid(format!("node '{id}': prompt must be a string")))?;
        if template.len() > PROMPT_CAP_BYTES {
            return Err(PiiError::Invalid(format!(
                "node '{id}': prompt is {} bytes, over the {PROMPT_CAP_BYTES}-byte cap",
                template.len()
            )));
        }
        for path in placeholders(template).map_err(|e| invalid_in(id, &e.to_string()))? {
            check_path(spec, needs, path)
                .map_err(|why| invalid_in(id, &format!("placeholder {{{{{path}}}}} {why}")))?;
        }
    }
    Ok(())
}

/// Whether `path` names something the graph supplies: a payload, a declared input key, or a node of the graph.
fn check_path(spec: &GraphSpec, needs: &[String], path: &str) -> Result<(), String> {
    let (root, rest) = path.split_once('.').unwrap_or((path, ""));
    let key = rest.split('.').next().unwrap_or("");
    match root {
        "payload" => Ok(()),
        "input" if needs.iter().any(|d| d == key) => Ok(()),
        "input" => Err(format!(
            "reads an input the graph does not declare in 'needs' (declares [{}])",
            needs.join(", ")
        )),
        "nodes" if spec.nodes.iter().any(|n| n.id.as_str() == key) => Ok(()),
        "nodes" => Err(format!("names node '{key}', which the graph does not have")),
        _ => Err("must start with input., payload. or nodes.".into()),
    }
}

fn invalid_in(node: &str, why: &str) -> PiiError {
    PiiError::Invalid(format!("node '{node}': {why}"))
}

/// The template with each `{{path}}` replaced by the value `ctx` holds at `path`.
///
/// A placeholder that resolves to nothing is a refusal, never a blank: an empty instruction is how an agent spends
/// money writing nothing. A string value is inserted as it is; any other value as its JSON text.
///
/// # Errors
/// [`PiiError::Invalid`] naming the placeholder that is unclosed or resolves to nothing.
pub fn render(template: &str, ctx: &Value) -> Result<String, PiiError> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find("}}")
            .ok_or_else(|| PiiError::Invalid("unclosed placeholder '{{'".into()))?;
        let path = after[..end].trim();
        match eval(ctx, &format!("$.{path}"))? {
            Value::Null => {
                return Err(PiiError::Invalid(format!(
                    "placeholder {{{{{path}}}}} resolved to nothing"
                )));
            }
            Value::String(s) => out.push_str(&s),
            other => out.push_str(&other.to_string()),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}
