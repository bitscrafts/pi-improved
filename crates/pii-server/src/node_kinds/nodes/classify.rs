//! `classify` node kind (spec 128): sorts a text into a class by ordered rules — markers and
//! keywords, no model. Pure and deterministic.
//!
//! Config: `on` (a `$.`-expression, default `$.input.text`), `default` (the class when no rule
//! matches), and `rules`: an ordered array of `{ class, prefix?, suffix?, any? }` where each
//! listed string is matched case-insensitively against the trimmed text (`prefix`: starts with,
//! `suffix`: ends with, `any`: contains). The first rule with a match wins. Port `ok` carries
//! `{ class, matched, text }` (`matched` is the string that fired, or null for the default).

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx};
use pii_graph::expr::eval;
use pii_graph::node::{GraphNode, NodeFactory};
use serde_json::{Value, json};

/// Most rules and most patterns per rule: a classifier is a short list, not a corpus.
const MAX_RULES: usize = 64;
const MAX_PATTERNS: usize = 64;

struct Rule {
    class: String,
    prefix: Vec<String>,
    suffix: Vec<String>,
    any: Vec<String>,
}

struct ClassifyNode {
    on: String,
    default: String,
    rules: Vec<Rule>,
}

impl Rule {
    fn matched(&self, text: &str) -> Option<&str> {
        let found = self
            .prefix
            .iter()
            .find(|p| text.starts_with(p.as_str()))
            .or_else(|| self.suffix.iter().find(|p| text.ends_with(p.as_str())))
            .or_else(|| self.any.iter().find(|p| text.contains(p.as_str())));
        found.map(String::as_str)
    }
}

impl GraphNode for ClassifyNode {
    fn run(&self, _ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let ec = json!({"input": input.input, "payload": input.payload, "nodes": input.nodes});
        let Value::String(raw) = eval(&ec, &self.on)? else {
            return Err(PiiError::Invalid(format!(
                "classify: '{}' did not resolve to a string",
                self.on
            )));
        };
        let text = raw.trim().to_lowercase();
        let (class, matched) = self
            .rules
            .iter()
            .find_map(|r| r.matched(&text).map(|m| (r.class.as_str(), Some(m))))
            .unwrap_or((self.default.as_str(), None));
        Ok(Outcome::Next {
            port: PortName::from("ok"),
            value: json!({"class": class, "matched": matched, "text": raw}),
        })
    }
}

/// Registered by [`crate::Engine::new`] and the lint registry.
pub struct ClassifyFactory;

fn patterns(r: &Value, key: &str) -> Result<Vec<String>, PiiError> {
    let Some(v) = r.get(key) else {
        return Ok(Vec::new());
    };
    let bad = || {
        PiiError::Invalid(format!(
            "classify: '{key}' must be an array of non-empty strings"
        ))
    };
    let list = v.as_array().ok_or_else(bad)?;
    if list.len() > MAX_PATTERNS {
        return Err(PiiError::Invalid(format!(
            "classify: more than {MAX_PATTERNS} '{key}' patterns"
        )));
    }
    list.iter()
        .map(|p| {
            p.as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_lowercase)
                .ok_or_else(bad)
        })
        .collect()
}

impl NodeFactory for ClassifyFactory {
    fn kind(&self) -> &'static str {
        "classify"
    }

    fn effects(&self) -> Effects {
        Effects::PURE
    }

    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }

    fn ports(&self, _cfg: &Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok")])
    }

    fn build(&self, cfg: &Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let default = cfg
            .get("default")
            .and_then(Value::as_str)
            .filter(|d| !d.is_empty())
            .ok_or_else(|| {
                PiiError::Invalid(String::from("classify requires 'default' (a class name)"))
            })?
            .to_owned();
        let on = cfg
            .get("on")
            .map_or(Some("$.input.text"), Value::as_str)
            .filter(|o| o.starts_with("$."))
            .ok_or_else(|| {
                PiiError::Invalid(String::from("classify: 'on' must be a '$.' expression"))
            })?
            .to_owned();
        let raw = cfg
            .get("rules")
            .and_then(Value::as_array)
            .filter(|r| !r.is_empty() && r.len() <= MAX_RULES)
            .ok_or_else(|| {
                PiiError::Invalid(format!(
                    "classify requires 'rules' (1..={MAX_RULES} entries)"
                ))
            })?;
        let mut rules = Vec::new();
        for r in raw {
            let class = r
                .get("class")
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())
                .ok_or_else(|| {
                    PiiError::Invalid(String::from("classify: every rule needs a 'class'"))
                })?
                .to_owned();
            let rule = Rule {
                class,
                prefix: patterns(r, "prefix")?,
                suffix: patterns(r, "suffix")?,
                any: patterns(r, "any")?,
            };
            if rule.prefix.is_empty() && rule.suffix.is_empty() && rule.any.is_empty() {
                return Err(PiiError::Invalid(format!(
                    "classify: rule '{}' has no prefix, suffix or any pattern",
                    rule.class
                )));
            }
            rules.push(rule);
        }
        Ok(Box::new(ClassifyNode { on, default, rules }))
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

    use pii_core::{Budget, CapSet};

    use super::*;

    fn cfg() -> Value {
        json!({"default": "chat", "rules": [
            {"class": "alert", "prefix": ["[alert]"], "any": ["outage"]},
            {"class": "question", "suffix": ["?"]},
        ]})
    }

    fn classify(text: &str) -> Value {
        let node = ClassifyFactory.build(&cfg()).unwrap();
        let input = NodeInput {
            input: json!({"text": text}),
            payload: Value::Null,
            nodes: json!({}),
        };
        let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
        match node.run(&ctx, &input).unwrap() {
            Outcome::Next { value, .. } => value,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn first_matching_rule_wins_and_names_what_fired() {
        let v = classify("  [ALERT] disk full?  ");
        assert_eq!(
            (v["class"].as_str(), v["matched"].as_str()),
            (Some("alert"), Some("[alert]"))
        );
        let v = classify("Is the daemon up?");
        assert_eq!(
            (v["class"].as_str(), v["matched"].as_str()),
            (Some("question"), Some("?"))
        );
        let v = classify("big OUTAGE in the lab");
        assert_eq!(v["class"], "alert");
    }

    #[test]
    fn no_rule_means_the_default_with_null_matched() {
        let v = classify("good morning");
        assert_eq!(v["class"], "chat");
        assert!(v["matched"].is_null());
        assert_eq!(v["text"], "good morning");
    }

    #[test]
    fn a_bad_config_is_refused_at_build() {
        for bad in [
            json!({"rules": [{"class": "a", "any": ["x"]}]}),
            json!({"default": "d"}),
            json!({"default": "d", "rules": []}),
            json!({"default": "d", "rules": [{"class": "a"}]}),
            json!({"default": "d", "rules": [{"any": ["x"]}]}),
            json!({"default": "d", "on": "text", "rules": [{"class": "a", "any": ["x"]}]}),
            json!({"default": "d", "rules": [{"class": "a", "any": [""]}]}),
        ] {
            assert!(ClassifyFactory.build(&bad).is_err(), "{bad}");
        }
    }
}
