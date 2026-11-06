//! The deliberately tiny expression language (spec F4.3, widened by spec 383).
//!
//! Context object is `{ "input": <run input>, "payload": <join payload>,
//! "nodes": { "<node id>": <value>, .. } }`, plus whatever `let` bound before it.
//!
//! The language is small ON PURPOSE, and it is closed:
//!
//! ```text
//! expr    := cmp
//! cmp     := add (("==" | "!=" | "<" | ">") add)?
//! add     := atom ("+" atom)*
//! atom    := "$." path | integer | "string" | name | "(" expr ")"
//!          | ("length" | "lower" | "trim") "(" expr ")"
//!          | ("starts_with" | "ends_with" | "contains") "(" expr "," expr ")"
//! ```
//!
//! Integers and strings only. A float is refused, not rounded: a graph that silently computes 1.5 as 1
//! would be lying about money, counts or limits. A literal too large for `i64` is refused at LOAD; an
//! addition that would overflow is refused when it happens.
//!
//! Before 383 a transform could only read a member of its input, so any arithmetic became a shell string
//! in the graph file: invisible in the graph, untestable, and outside the pin. That is what this replaces.

use pii_abi::PiiError;
use serde_json::Value;

pub use crate::template::{placeholders, render};

/// Spec 386: the largest prompt template a node may carry, in bytes.
pub const PROMPT_CAP_BYTES: usize = 64 * 1024;

/// A parsed expression. Parsing is the LOAD-TIME check: an expression that cannot parse never reaches a
/// run, and a name that is not bound is refused while the graph is being read, not when it executes.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// `$.a.b` — a member path into the context.
    Path(String),
    /// An integer literal. A float literal never parses.
    Int(i64),
    /// A string literal, double quoted.
    Str(String),
    /// A name bound by an earlier `let`.
    Var(String),
    /// `length(x)` — characters in a string.
    Len(Box<Self>),
    /// `lower(x)`, `trim(x)` — a string computed from a string (spec 392).
    Text(TextFn, Box<Self>),
    /// `starts_with(s, x)`, `ends_with(s, x)`, `contains(s, x)` — produces a boolean (spec 392).
    Has(HasFn, Box<Self>, Box<Self>),
    /// `a + b` — integers add, strings concatenate, nothing else mixes.
    Add(Box<Self>, Box<Self>),
    /// `a == b`, `a != b`, `a < b`, `a > b` — produces a boolean.
    Cmp(CmpOp, Box<Self>, Box<Self>),
}

/// The functions from a string to a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFn {
    /// `lower(x)`
    Lower,
    /// `trim(x)`
    Trim,
}

/// The tests of a string against a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HasFn {
    /// `starts_with(s, x)`
    Prefix,
    /// `ends_with(s, x)`
    Suffix,
    /// `contains(s, x)`
    Within,
}

/// The four comparisons the language has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `>`
    Gt,
}

impl CmpOp {
    fn as_str(self) -> &'static str {
        match self {
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Gt => ">",
        }
    }
}

impl Expr {
    /// Parses one expression.
    ///
    /// # Errors
    /// Returns [`PiiError::Invalid`] naming the expression when it does not parse, carries a float
    /// literal, or carries an integer literal too large for `i64`.
    pub fn parse(src: &str) -> Result<Self, PiiError> {
        let mut p = Parser {
            chars: src.chars().collect(),
            at: 0,
            src,
        };
        let e = p.expr()?;
        p.ws();
        if p.at < p.chars.len() {
            return Err(p.err(&format!("trailing '{}'", p.rest())));
        }
        Ok(e)
    }

    /// Every name this expression reads that is not a `$.` path.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.collect_names(&mut out);
        out
    }

    fn collect_names<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Self::Var(n) => out.push(n),
            Self::Len(e) | Self::Text(_, e) => e.collect_names(out),
            Self::Add(a, b) | Self::Cmp(_, a, b) | Self::Has(_, a, b) => {
                a.collect_names(out);
                b.collect_names(out);
            }
            Self::Path(_) | Self::Int(_) | Self::Str(_) => {}
        }
    }

    /// Evaluates against a context.
    ///
    /// # Errors
    /// Returns [`PiiError::Invalid`] on an unbound name, a float anywhere in the computation, a type
    /// mismatch, or an integer overflow.
    pub fn eval(&self, ctx: &Value) -> Result<Value, PiiError> {
        match self {
            Self::Path(path) => {
                let mut cur = ctx;
                for key in path.split('.') {
                    cur = match cur.get(key) {
                        Some(v) => v,
                        None => return Ok(Value::Null),
                    };
                }
                refuse_float(cur)
            }
            Self::Int(i) => Ok(Value::from(*i)),
            Self::Str(s) => Ok(Value::from(s.clone())),
            Self::Var(name) => {
                let v = ctx.get(name).ok_or_else(|| {
                    PiiError::Invalid(format!("name '{name}' is not bound in this transform"))
                })?;
                refuse_float(v)
            }
            Self::Len(inner) => match inner.eval(ctx)? {
                Value::String(s) => {
                    let n = i64::try_from(s.chars().count()).map_err(|_| {
                        PiiError::Invalid("length() does not fit in an integer".into())
                    })?;
                    Ok(Value::from(n))
                }
                other => Err(PiiError::Invalid(format!(
                    "length() takes a string, got {}",
                    kind_of(&other)
                ))),
            },
            Self::Text(func, inner) => eval_text(*func, &inner.eval(ctx)?),
            Self::Has(func, hay, needle) => eval_has(*func, &hay.eval(ctx)?, &needle.eval(ctx)?),
            Self::Add(a, b) => match (a.eval(ctx)?, b.eval(ctx)?) {
                (Value::Number(x), Value::Number(y)) => {
                    let (Some(x), Some(y)) = (x.as_i64(), y.as_i64()) else {
                        return Err(PiiError::Invalid("'+' on integers only".into()));
                    };
                    x.checked_add(y)
                        .map(Value::from)
                        .ok_or_else(|| PiiError::Invalid(format!("integer overflow: {x} + {y}")))
                }
                (Value::String(x), Value::String(y)) => Ok(Value::from(format!("{x}{y}"))),
                (x, y) => Err(PiiError::Invalid(format!(
                    "'+' needs two integers or two strings, got {} and {}",
                    kind_of(&x),
                    kind_of(&y)
                ))),
            },
            Self::Cmp(op, a, b) => {
                let (x, y) = (a.eval(ctx)?, b.eval(ctx)?);
                let truth = match (*op, &x, &y) {
                    (CmpOp::Eq, _, _) => x == y,
                    (CmpOp::Ne, _, _) => x != y,
                    (CmpOp::Lt, Value::Number(a), Value::Number(b)) => {
                        let (Some(a), Some(b)) = (a.as_i64(), b.as_i64()) else {
                            return Err(PiiError::Invalid("'<' on integers only".into()));
                        };
                        a < b
                    }
                    (CmpOp::Gt, Value::Number(a), Value::Number(b)) => {
                        let (Some(a), Some(b)) = (a.as_i64(), b.as_i64()) else {
                            return Err(PiiError::Invalid("'>' on integers only".into()));
                        };
                        a > b
                    }
                    (CmpOp::Lt, Value::String(a), Value::String(b)) => a < b,
                    (CmpOp::Gt, Value::String(a), Value::String(b)) => a > b,
                    (op, x, y) => {
                        return Err(PiiError::Invalid(format!(
                            "'{}' cannot compare {} with {}",
                            op.as_str(),
                            kind_of(x),
                            kind_of(y)
                        )));
                    }
                };
                Ok(Value::Bool(truth))
            }
        }
    }
}

/// A float is refused wherever a value is read: rounding is a silent decision, and a graph must not make
/// one on the operator's behalf.
fn refuse_float(v: &Value) -> Result<Value, PiiError> {
    if let Value::Number(n) = v
        && n.as_i64().is_none()
        && n.as_f64().is_some()
    {
        return Err(PiiError::Invalid(format!(
            "float values are not supported here: {n} — use an integer"
        )));
    }
    Ok(v.clone())
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "nothing",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "an integer",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

struct Parser<'a> {
    chars: Vec<char>,
    at: usize,
    src: &'a str,
}

impl Parser<'_> {
    fn err(&self, why: &str) -> PiiError {
        PiiError::Invalid(format!("bad expression '{}': {why}", self.src))
    }

    fn ws(&mut self) {
        while matches!(self.chars.get(self.at), Some(c) if c.is_whitespace()) {
            self.at += 1;
        }
    }

    /// The characters from `at` on. Never slices directly: the repo denies `slicing_slicing`, and a
    /// parser that can panic on its own cursor is a parser that turns a typo into a crash.
    fn from(&self, at: usize) -> &[char] {
        self.chars.get(at..).unwrap_or(&[])
    }

    fn rest(&self) -> String {
        self.from(self.at).iter().collect()
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn take(&mut self, s: &str) -> bool {
        self.ws();
        let want: Vec<char> = s.chars().collect();
        if self.from(self.at).starts_with(&want) {
            self.at += want.len();
            true
        } else {
            false
        }
    }

    fn expr(&mut self) -> Result<Expr, PiiError> {
        let left = self.add()?;
        self.ws();
        for (op, text) in [
            (CmpOp::Eq, "=="),
            (CmpOp::Ne, "!="),
            (CmpOp::Lt, "<"),
            (CmpOp::Gt, ">"),
        ] {
            if self.take(text) {
                let right = self.add()?;
                return Ok(Expr::Cmp(op, Box::new(left), Box::new(right)));
            }
        }
        Ok(left)
    }

    fn add(&mut self) -> Result<Expr, PiiError> {
        let mut left = self.atom()?;
        while self.take("+") {
            let right = self.atom()?;
            left = Expr::Add(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn atom(&mut self) -> Result<Expr, PiiError> {
        self.ws();
        match self.peek() {
            None => Err(self.err("it ends where a value is expected")),
            Some('$') => self.path(),
            Some('"') => self.string(),
            Some('(') => self.paren(),
            Some(c) if c.is_ascii_digit() => {
                let start = self.at;
                while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                    self.at += 1;
                }
                if matches!(self.peek(), Some('.')) {
                    return Err(self.err("float literals are not supported — use an integer"));
                }
                let text: String = self
                    .from(start)
                    .iter()
                    .take(self.at.saturating_sub(start))
                    .collect();
                text.parse::<i64>().map(Expr::Int).map_err(|_| {
                    self.err(&format!("integer overflow: {text} does not fit in an i64"))
                })
            }
            Some(c) if c.is_alphanumeric() || c == '_' => {
                let start = self.at;
                while matches!(self.peek(), Some(c) if c.is_alphanumeric() || c == '_') {
                    self.at += 1;
                }
                let name: String = self
                    .from(start)
                    .iter()
                    .take(self.at.saturating_sub(start))
                    .collect();
                if name == "length" && self.take("(") {
                    let inner = self.expr()?;
                    if !self.take(")") {
                        return Err(self.err("length( is never closed"));
                    }
                    return Ok(Expr::Len(Box::new(inner)));
                }
                if let Some(call) = self.string_call(&name)? {
                    return Ok(call);
                }
                if matches!(self.peek(), Some('.')) {
                    return Err(self.err(&format!(
                        "'{name}.' is not a path — a path starts with '$.'"
                    )));
                }
                Ok(Expr::Var(name))
            }
            Some(c) => Err(self.err(&format!("'{c}' cannot start a value"))),
        }
    }
    /// `lower(x)`, `trim(x)`, `starts_with(s, x)`, `ends_with(s, x)`, `contains(s, x)`: `None` when `name` is not one of them.
    fn string_call(&mut self, name: &str) -> Result<Option<Expr>, PiiError> {
        let text_fn = match name {
            "lower" => Some(TextFn::Lower),
            "trim" => Some(TextFn::Trim),
            _ => None,
        };
        let has_fn = match name {
            "starts_with" => Some(HasFn::Prefix),
            "ends_with" => Some(HasFn::Suffix),
            "contains" => Some(HasFn::Within),
            _ => None,
        };
        if (text_fn.is_none() && has_fn.is_none()) || !self.take("(") {
            return Ok(None);
        }
        let first = Box::new(self.expr()?);
        let call = if let Some(func) = text_fn {
            Expr::Text(func, first)
        } else if let Some(func) = has_fn {
            if !self.take(",") {
                return Err(self.err(&format!("{name}( takes two values")));
            }
            Expr::Has(func, first, Box::new(self.expr()?))
        } else {
            return Ok(None);
        };
        if !self.take(")") {
            return Err(self.err(&format!("{name}( is never closed")));
        }
        Ok(Some(call))
    }

    /// `$.a.b` — a path into the evaluation context. '-' is part of a key: graph ids carry it.
    fn path(&mut self) -> Result<Expr, PiiError> {
        if !self.take("$.") {
            return Err(self.err("'$' must be followed by a path"));
        }
        let start = self.at;
        // '-' too: node ids carry it (`method-judge`, `self-improve`), and a path grammar that
        // stops at the hyphen sends `$.nodes.implement__method-judge__in__agent` to the error arm —
        // which `require` reads as a MISSING KEY and refuses the whole callee.
        while matches!(self.peek(), Some(c) if c.is_alphanumeric() || c == '_' || c == '.' || c == '-')
        {
            self.at += 1;
        }
        if self.at == start {
            return Err(self.err("'$.' must name at least one key"));
        }
        Ok(Expr::Path(
            self.from(start)
                .iter()
                .take(self.at.saturating_sub(start))
                .collect(),
        ))
    }

    /// A double-quoted string literal, with `\"`, `\\` and `\n` escapes.
    fn string(&mut self) -> Result<Expr, PiiError> {
        self.at += 1;
        let mut s = String::new();
        loop {
            match self.peek() {
                None => return Err(self.err("unterminated string")),
                Some('"') => {
                    self.at += 1;
                    break;
                }
                Some('\\') => {
                    self.at += 1;
                    match self.peek() {
                        Some('"') => s.push('"'),
                        Some('\\') => s.push('\\'),
                        Some('n') => s.push('\n'),
                        Some(other) => {
                            s.push('\\');
                            s.push(other);
                        }
                        None => return Err(self.err("unterminated escape")),
                    }
                    self.at += 1;
                }
                Some(c) => {
                    s.push(c);
                    self.at += 1;
                }
            }
        }
        Ok(Expr::Str(s))
    }
    /// `( expr )` — grouping, so a comparison can be an operand.
    fn paren(&mut self) -> Result<Expr, PiiError> {
        self.at += 1;
        let e = self.expr()?;
        if !self.take(")") {
            return Err(self.err("'(' is never closed"));
        }
        Ok(e)
    }
}

fn eval_text(func: TextFn, value: &Value) -> Result<Value, PiiError> {
    let Value::String(text) = value else {
        return Err(PiiError::Invalid(format!(
            "{func:?} takes a string, got {}",
            kind_of(value)
        )));
    };
    Ok(Value::from(match func {
        TextFn::Lower => text.to_lowercase(),
        TextFn::Trim => text.trim().to_owned(),
    }))
}

fn eval_has(func: HasFn, hay: &Value, needle: &Value) -> Result<Value, PiiError> {
    let (Value::String(hay_str), Value::String(needle_str)) = (hay, needle) else {
        return Err(PiiError::Invalid(format!(
            "{func:?} takes two strings, got {} and {}",
            kind_of(hay),
            kind_of(needle)
        )));
    };
    Ok(Value::from(match func {
        HasFn::Prefix => hay_str.starts_with(needle_str.as_str()),
        HasFn::Suffix => hay_str.ends_with(needle_str.as_str()),
        HasFn::Within => hay_str.contains(needle_str.as_str()),
    }))
}

/// Evaluates `$.path.into.context` (the pre-383 form, kept for every caller that names a member).
///
/// # Errors
/// Returns [`PiiError::Invalid`] if `expr` does not start with `$.`.
pub fn eval(ctx: &Value, expr: &str) -> Result<Value, PiiError> {
    Expr::parse(expr)?.eval(ctx)
}

/// If `v` is a string starting with `$.`, evaluate it; otherwise return it unchanged. Used uniformly for
/// node config values.
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

    use super::{Expr, eval, eval_or_literal};
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

    #[test]
    fn parentheses_and_precedence() {
        let c = json!({});
        let e = Expr::parse("(1 + 2) + 3").unwrap();
        assert_eq!(e.eval(&c).unwrap(), json!(6));
        let e = Expr::parse("1 + 2 + 3").unwrap();
        assert_eq!(e.eval(&c).unwrap(), json!(6));
    }

    #[test]
    fn names_are_reported_for_load_time_binding() {
        let e = Expr::parse("a + length(b) + $.input.n").unwrap();
        assert_eq!(e.names(), vec!["a", "b"]);
    }

    #[test]
    fn a_float_literal_never_parses() {
        let err = Expr::parse("1.5 + 1").unwrap_err().to_string();
        assert!(err.contains("float"), "{err}");
    }
}

#[cfg(test)]
mod string_function_tests {
    use super::*;
    use serde_json::json;

    fn run(src: &str) -> Result<Value, PiiError> {
        Expr::parse(src)?.eval(&json!({"text": "  Is It DOWN?  ", "n": 3}))
    }

    #[test]
    fn the_string_functions_compute_what_a_classifier_needs() -> Result<(), PiiError> {
        assert_eq!(run("lower(trim($.text))")?, json!("is it down?"));
        assert_eq!(
            run(r#"starts_with(lower(trim($.text)), "is")"#)?,
            json!(true)
        );
        assert_eq!(run(r#"ends_with(trim($.text), "?")"#)?, json!(true));
        assert_eq!(run(r#"contains($.text, "DOWN")"#)?, json!(true));
        assert_eq!(run(r#"contains($.text, "down")"#)?, json!(false));
        Ok(())
    }

    #[test]
    fn a_string_function_refuses_what_is_not_a_string() {
        assert!(run("lower($.n)").is_err());
        assert!(run(r#"contains($.n, "3")"#).is_err());
        assert!(Expr::parse(r"contains($.text)").is_err());
        assert!(Expr::parse("lower($.text").is_err());
    }
}
