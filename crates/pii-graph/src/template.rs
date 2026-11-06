//! Prompt templates.
//!
//! Spec 386: the `{{path}}` placeholders of a template, and rendering one against an eval context. Split out of `expr.rs` so that file stays under the line rule; `expr::{placeholders, render}` are re-exported.

use crate::expr::eval;
use pii_abi::PiiError;
use serde_json::Value;

/// Spec 386: the `{{path}}` placeholders of a prompt template, in order of appearance.
///
/// # Errors
/// Returns [`PiiError::Invalid`] if a `{{` is never closed by `}}`.
pub fn placeholders(template: &str) -> Result<Vec<&str>, PiiError> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        let after = &rest[open + 2..];
        let close = after
            .find("}}")
            .ok_or_else(|| PiiError::Invalid(format!("unclosed placeholder in '{template}'")))?;
        out.push(after[..close].trim());
        rest = &after[close + 2..];
    }
    Ok(out)
}

/// Spec 386: `template` with each `{{path}}` replaced by the value `path` resolves to in `ctx`.
///
/// A string value is inserted as is; any other value as its JSON text. A path that resolves to nothing is an error,
/// never a blank.
///
/// # Errors
/// Returns [`PiiError::Invalid`] on an unclosed placeholder or a path that resolves to nothing.
pub fn render(ctx: &Value, template: &str) -> Result<String, PiiError> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let close = after
            .find("}}")
            .ok_or_else(|| PiiError::Invalid(format!("unclosed placeholder in '{template}'")))?;
        let path = after[..close].trim();
        let value = eval(ctx, &format!("$.{path}"))?;
        match value {
            Value::Null => {
                return Err(PiiError::Invalid(format!(
                    "placeholder '{{{{{path}}}}}' resolved to nothing"
                )));
            }
            Value::String(s) => out.push_str(&s),
            other => out.push_str(&other.to_string()),
        }
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    Ok(out)
}
