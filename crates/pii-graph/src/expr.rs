//! The deliberately tiny expression language (spec F4.3).
//!
//! Context object is `{ "input": <run input>, "payload": <join payload>,
//! "nodes": { "<node id>": <value>, .. } }`.
//!
//! Only object member access is supported: `$.<key>(.<key>)*`. No arrays, no
//! functions. An unresolvable path returns `Ok(Value::Null)`; a malformed
//! expression (not starting with `$.`) returns [`PiiError::Invalid`].

use pii_abi::PiiError;
use serde_json::Value;

/// Evaluates `$.path.into.context`.
///
/// # Errors
/// Returns [`PiiError::Invalid`] if `expr` does not start with `$.`.
pub fn eval(ctx: &Value, expr: &str) -> Result<Value, PiiError> {
    let path = expr
        .strip_prefix("$.")
        .ok_or_else(|| PiiError::Invalid(format!("expression must start with '$.' : '{expr}'")))?;
    if path.is_empty() {
        return Ok(Value::Null);
    }
    let mut cur = ctx;
    for key in path.split('.') {
        cur = match cur.get(key) {
            Some(v) => v,
            None => return Ok(Value::Null),
        };
    }
    Ok(cur.clone())
}

/// If `v` is a string starting with `$.`, evaluate it; otherwise return it
/// unchanged. Used uniformly for node config values.
///
/// # Errors
/// Returns [`PiiError::Invalid`] if a `$.`-string is malformed.
pub fn eval_or_literal(ctx: &Value, v: &Value) -> Result<Value, PiiError> {
    match v {
        Value::String(s) if s.starts_with("$.") => eval(ctx, s),
        other => Ok(other.clone()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{eval, eval_or_literal};
    use serde_json::{Value, json};

    fn ctx() -> Value {
        json!({"input": {"branch": "a", "n": 3}, "payload": {"x": 1}, "nodes": {"a": {"v": true}}})
    }

    #[test]
    fn eval_resolves_object_paths() {
        let c = ctx();
        assert_eq!(eval(&c, "$.input.branch").unwrap(), json!("a"));
        assert_eq!(eval(&c, "$.payload.x").unwrap(), json!(1));
        assert_eq!(eval(&c, "$.nodes.a.v").unwrap(), json!(true));
    }

    #[test]
    fn eval_unresolvable_returns_null() {
        let c = ctx();
        assert_eq!(eval(&c, "$.input.missing").unwrap(), Value::Null);
        assert_eq!(eval(&c, "$.nope.deep").unwrap(), Value::Null);
    }

    #[test]
    fn eval_malformed_is_invalid() {
        assert!(eval(&ctx(), "input.branch").is_err());
        assert!(eval(&ctx(), "no-dollar").is_err());
    }

    #[test]
    fn eval_or_literal_passes_non_exprs_through() {
        let c = ctx();
        assert_eq!(eval_or_literal(&c, &json!(42)).unwrap(), json!(42));
        assert_eq!(
            eval_or_literal(&c, &json!("literal")).unwrap(),
            json!("literal")
        );
        assert_eq!(eval_or_literal(&c, &json!("$.input.n")).unwrap(), json!(3));
    }
}
