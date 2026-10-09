//! Events: a leveled, structured observation from a subsystem.

use std::borrow::Cow;

use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// Severity of an event. Ordered so thresholds work (`Trace < … < Error`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Finest-grained diagnostics.
    Trace,
    /// Developer diagnostics.
    Debug,
    /// Normal operation.
    Info,
    /// Something unexpected but recoverable.
    Warn,
    /// A failure.
    Error,
}

/// Identifies a span an event belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SpanId(pub u64);

/// Ordered key/value fields; cheap to build (a `Vec`, static keys by default)
/// and serialised as a JSON object.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Fields(Vec<(Cow<'static, str>, Value)>);

impl Fields {
    /// No fields.
    #[must_use]
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Adds a field (builder style).
    #[must_use]
    pub fn with(mut self, key: &'static str, value: impl Into<Value>) -> Self {
        self.0.push((Cow::Borrowed(key), value.into()));
        self
    }

    /// The value for `key`, if present (last write wins).
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Number of fields.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no fields.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Serialize for Fields {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            m.serialize_entry(k, v)?;
        }
        m.end()
    }
}

struct FieldsVisitor;

impl<'de> Visitor<'de> for FieldsVisitor {
    type Value = Fields;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Fields, A::Error> {
        let mut out = Vec::new();
        while let Some((k, v)) = a.next_entry::<String, Value>()? {
            out.push((Cow::Owned(k), v));
        }
        Ok(Fields(out))
    }
}

impl<'de> Deserialize<'de> for Fields {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_map(FieldsVisitor)
    }
}

/// A structured observation. Names are `&'static str` (see [`crate::names`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Event {
    /// The reporting subsystem (e.g. `"graph"`).
    pub subsystem: &'static str,
    /// Severity.
    pub level: Level,
    /// What happened (a [`crate::names`] constant).
    pub kind: &'static str,
    /// Structured detail.
    pub fields: Fields,
    /// The span this event belongs to, if any.
    pub span: Option<SpanId>,
}

impl Event {
    /// An event with no fields and no span.
    #[must_use]
    pub fn new(subsystem: &'static str, level: Level, kind: &'static str) -> Self {
        Self {
            subsystem,
            level,
            kind,
            fields: Fields::new(),
            span: None,
        }
    }

    /// Adds a field.
    #[must_use]
    pub fn field(mut self, key: &'static str, value: impl Into<Value>) -> Self {
        self.fields = self.fields.with(key, value);
        self
    }

    /// Attaches a span.
    #[must_use]
    pub fn in_span(mut self, span: SpanId) -> Self {
        self.span = Some(span);
        self
    }
}

/// The owned, deserialisable form of an [`Event`] (what a reader of the
/// serialised stream sees, since `&'static str` cannot be deserialised).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedEvent {
    /// The reporting subsystem.
    pub subsystem: String,
    /// Severity.
    pub level: Level,
    /// What happened.
    pub kind: String,
    /// Structured detail.
    pub fields: Fields,
    /// The span this event belongs to, if any.
    pub span: Option<SpanId>,
}

impl From<&Event> for OwnedEvent {
    fn from(e: &Event) -> Self {
        Self {
            subsystem: e.subsystem.to_owned(),
            level: e.level,
            kind: e.kind.to_owned(),
            fields: e.fields.clone(),
            span: e.span,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn level_orders_for_thresholds() {
        assert!(Level::Trace < Level::Debug);
        assert!(Level::Warn < Level::Error);
        assert!(Level::Info >= Level::Info);
    }

    #[test]
    fn event_serde_round_trip() {
        let e = Event::new("graph", Level::Warn, "graph.node.dead")
            .field("count", 3u64)
            .field("why", "upstream failed")
            .in_span(SpanId(7));
        let json = serde_json::to_string(&e).unwrap();
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v.pointer("/fields/count"), Some(&Value::from(3)));
        assert_eq!(v.pointer("/level"), Some(&Value::from("warn")));
        assert_eq!(v.pointer("/span"), Some(&Value::from(7)));
        let back: OwnedEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, OwnedEvent::from(&e));
        assert_eq!(back.fields.get("count"), Some(&Value::from(3u64)));
    }

    #[test]
    fn event_without_span_round_trips() {
        let e = Event::new("x", Level::Info, "k");
        let json = serde_json::to_string(&e).unwrap();
        let back: OwnedEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, OwnedEvent::from(&e));
        assert!(back.fields.is_empty());
        assert_eq!(back.span, None);
    }
}
