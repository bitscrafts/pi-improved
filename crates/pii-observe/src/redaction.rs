//! Redaction: a policy applied at the trajectory sink, never at call sites.

/// What to do with one field value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redaction {
    /// Keep the value as is.
    Keep,
    /// Remove the field entirely.
    Drop,
    /// Replace the value with this text.
    Replace(String),
}

/// A redaction policy.
///
/// `field` is the dotted path of the field (nested
/// object keys joined with `.`; array elements share their parent's path),
/// `value` is its text (non-string leaves are passed in their JSON form).
pub trait RedactionPolicy: Send + Sync {
    /// Decides what happens to `field`.
    fn redact(&self, field: &str, value: &str) -> Redaction;
}

/// The default policy: keeps everything.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoRedaction;

impl RedactionPolicy for NoRedaction {
    fn redact(&self, _field: &str, _value: &str) -> Redaction {
        Redaction::Keep
    }
}

/// Drops every field whose dotted path, or last path segment, is listed.
/// The env-configurable policy (`PII_TRAJECTORY_REDACT=a,b`).
#[derive(Debug, Clone, Default)]
pub struct DropFields(Vec<String>);

impl DropFields {
    /// A policy dropping the named fields.
    #[must_use]
    pub fn new(names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self(names.into_iter().map(Into::into).collect())
    }

    /// Parses a comma-separated list; `None` when it names nothing.
    #[must_use]
    pub fn parse(list: &str) -> Option<Self> {
        let v: Vec<String> = list
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        (!v.is_empty()).then_some(Self(v))
    }
}

impl RedactionPolicy for DropFields {
    fn redact(&self, field: &str, _value: &str) -> Redaction {
        let last = field.rsplit('.').next().unwrap_or(field);
        if self.0.iter().any(|n| n == field || n == last) {
            Redaction::Drop
        } else {
            Redaction::Keep
        }
    }
}

/// Wraps a policy and additionally replaces any known secret value found in a
/// kept field (the inner policy's `Drop`/`Replace` decisions win).
pub struct ScrubbingPolicy {
    inner: std::sync::Arc<dyn RedactionPolicy>,
    scrubber: crate::SecretScrubber,
}

impl ScrubbingPolicy {
    /// Applies `inner`, then `scrubber` to what `inner` keeps.
    #[must_use]
    pub fn new(
        inner: std::sync::Arc<dyn RedactionPolicy>,
        scrubber: crate::SecretScrubber,
    ) -> Self {
        Self { inner, scrubber }
    }
}

impl RedactionPolicy for ScrubbingPolicy {
    fn redact(&self, field: &str, value: &str) -> Redaction {
        match self.inner.redact(field, value) {
            Redaction::Keep => match self.scrubber.scrub(value) {
                std::borrow::Cow::Borrowed(_) => Redaction::Keep,
                std::borrow::Cow::Owned(s) => Redaction::Replace(s),
            },
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn drop_fields_matches_path_or_last_segment_only() {
        let p = DropFields::parse(" token , a.b ").unwrap();
        assert_eq!(p.redact("token", "x"), Redaction::Drop);
        assert_eq!(p.redact("n.token", "x"), Redaction::Drop);
        assert_eq!(p.redact("a.b", "x"), Redaction::Drop);
        assert_eq!(p.redact("tokens", "x"), Redaction::Keep);
        assert!(DropFields::parse(" , ").is_none());
    }
}
