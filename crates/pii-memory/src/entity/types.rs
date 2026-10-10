//! The record types, the content-derived ids and the validation rules.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Longest entity name, in bytes.
pub const MAX_NAME_BYTES: usize = 256;
/// Longest kind, in bytes.
pub const MAX_KIND_BYTES: usize = 64;
/// Longest attribute key, in bytes.
pub const MAX_ATTR_KEY_BYTES: usize = 64;
/// Longest attribute value, in bytes.
pub const MAX_ATTR_VALUE_BYTES: usize = 1024;
/// Longest provenance source, in bytes.
pub const MAX_SOURCE_BYTES: usize = 256;

/// A typed thing the agent knows about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    /// `e-` + 32 hex of blake3 over the kind and the name (never the attributes).
    pub id: String,
    /// What sort of thing it is (`device`, `room`, ...).
    pub kind: String,
    /// What it is called; with the kind, this is its identity.
    pub name: String,
    /// Data about it. On a merge the greater value of a clashing key wins, so
    /// the outcome does not depend on write order.
    pub attributes: BTreeMap<String, String>,
    /// Where it came from: `run:<id>`, `file:<path>` or `actor:<name>`; sorted, deduped.
    pub provenance: Vec<String>,
}

/// A typed, directed link between two entities that exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relationship {
    /// `r-` + 32 hex of blake3 over `from`, `to` and the kind.
    pub id: String,
    /// The source entity id.
    pub from: String,
    /// The target entity id.
    pub to: String,
    /// What the link means (`located_in`, ...).
    pub kind: String,
    /// Data about the link (merged like an entity's).
    pub attributes: BTreeMap<String, String>,
    /// Where it came from; sorted, deduped.
    pub provenance: Vec<String>,
}

/// What an entity write is asked to record.
#[derive(Debug, Clone, Default)]
pub struct EntityInput {
    /// The kind.
    pub kind: String,
    /// The name.
    pub name: String,
    /// Attributes (data).
    pub attributes: BTreeMap<String, String>,
    /// Provenance sources; at least one is required.
    pub provenance: Vec<String>,
}

impl EntityInput {
    /// An input with no attributes and no provenance yet.
    #[must_use]
    pub fn new(kind: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            name: name.into(),
            ..Self::default()
        }
    }

    /// Adds a provenance source.
    #[must_use]
    pub fn source(mut self, source: impl Into<String>) -> Self {
        self.provenance.push(source.into());
        self
    }

    /// Adds an attribute.
    #[must_use]
    pub fn attr(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }
}

/// What a relationship write is asked to record.
#[derive(Debug, Clone, Default)]
pub struct RelationshipInput {
    /// The source entity id.
    pub from: String,
    /// The target entity id.
    pub to: String,
    /// The kind.
    pub kind: String,
    /// Attributes (data).
    pub attributes: BTreeMap<String, String>,
    /// Provenance sources; at least one is required.
    pub provenance: Vec<String>,
}

impl RelationshipInput {
    /// An input with no attributes and no provenance yet.
    #[must_use]
    pub fn new(from: &str, to: &str, kind: impl Into<String>) -> Self {
        Self {
            from: from.to_owned(),
            to: to.to_owned(),
            kind: kind.into(),
            ..Self::default()
        }
    }

    /// Adds a provenance source.
    #[must_use]
    pub fn source(mut self, source: impl Into<String>) -> Self {
        self.provenance.push(source.into());
        self
    }

    /// Adds an attribute.
    #[must_use]
    pub fn attr(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }
}

fn hex_id(prefix: &str, domain: &str, parts: &[&str]) -> String {
    let mut h = blake3::Hasher::new();
    h.update(domain.as_bytes());
    for p in parts {
        h.update(&[0]);
        h.update(p.as_bytes());
    }
    format!(
        "{prefix}-{}",
        h.finalize().to_hex().chars().take(32).collect::<String>()
    )
}

/// The id of the entity of `kind` named `name`: what is hashed is the domain tag, the kind and the name.
#[must_use]
pub fn entity_id(kind: &str, name: &str) -> String {
    hex_id("e", "pii.entity.v1", &[kind, name])
}

/// The id of the relationship `kind` from `from` to `to`.
#[must_use]
pub fn relationship_id(from: &str, to: &str, kind: &str) -> String {
    hex_id("r", "pii.relationship.v1", &[from, to, kind])
}

/// True when `id` has the shape of an entity id (`e-` + 32 lowercase hex).
#[must_use]
pub fn is_entity_id(id: &str) -> bool {
    id.strip_prefix("e-").is_some_and(|h| {
        h.len() == 32
            && h.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// Why a write or a query was refused. Each has a stable word for events and counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    /// No provenance source at all.
    NoProvenance,
    /// A source that is not `run:`, `file:` or `actor:` plus something.
    BadProvenance,
    /// An empty or malformed field.
    Invalid,
    /// A field over its size bound.
    TooLarge,
    /// The entity cap is reached.
    EntityCap,
    /// The relationship cap is reached.
    RelationshipCap,
    /// A record would exceed the provenance bound.
    ProvenanceCap,
    /// A relationship or query names an entity that does not exist.
    UnknownEntity,
    /// An empty query.
    BadQuery,
    /// A depth of zero.
    BadDepth,
    /// A depth above the configured cap.
    DepthCap,
    /// More matches than the result cap allows.
    ResultCap,
    /// The write would take the graph past its byte budget (spec 127).
    BytesCap,
    /// An archive could not be written and verified, so nothing was removed (spec 127).
    ArchiveFailed,
}

impl Reason {
    /// The stable word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoProvenance => "no_provenance",
            Self::BadProvenance => "bad_provenance",
            Self::Invalid => "invalid",
            Self::TooLarge => "too_large",
            Self::EntityCap => "entity_cap",
            Self::RelationshipCap => "relationship_cap",
            Self::ProvenanceCap => "provenance_cap",
            Self::UnknownEntity => "unknown_entity",
            Self::BadQuery => "bad_query",
            Self::BadDepth => "bad_depth",
            Self::DepthCap => "depth_cap",
            Self::ResultCap => "result_cap",
            Self::BytesCap => "bytes_cap",
            Self::ArchiveFailed => "archive_failed",
        }
    }
}

/// A failure: a named refusal, or the store could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    /// A rule refused the call; it is counted and nothing was written.
    Refused(Reason, String),
    /// The file could not be read, parsed or written.
    Store(String),
}

impl GraphError {
    /// The refusal word, or `None` for a store failure.
    #[must_use]
    pub const fn reason(&self) -> Option<&'static str> {
        match self {
            Self::Refused(r, _) => Some(r.as_str()),
            Self::Store(_) => None,
        }
    }
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(r, why) => write!(f, "{}: {why}", r.as_str()),
            Self::Store(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for GraphError {}

pub(super) fn refuse<T>(r: Reason, why: impl Into<String>) -> Result<T, GraphError> {
    Err(GraphError::Refused(r, why.into()))
}

fn has_control(s: &str) -> bool {
    s.chars().any(char::is_control)
}

/// Checks a kind: lowercase ascii, digits, `_`, `-`, `.`; non-empty; bounded.
pub(super) fn check_kind(kind: &str) -> Result<(), GraphError> {
    if kind.is_empty() || kind.len() > MAX_KIND_BYTES {
        return refuse(Reason::Invalid, "kind must be 1..=64 bytes");
    }
    if !kind
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.'))
    {
        return refuse(Reason::Invalid, "kind may hold only a-z 0-9 _ - .");
    }
    Ok(())
}

pub(super) fn check_name(name: &str) -> Result<(), GraphError> {
    if name.trim().is_empty() || has_control(name) {
        return refuse(
            Reason::Invalid,
            "name must be non-empty text without control characters",
        );
    }
    if name.len() > MAX_NAME_BYTES {
        return refuse(
            Reason::TooLarge,
            format!("name over {MAX_NAME_BYTES} bytes"),
        );
    }
    Ok(())
}

pub(super) fn check_attributes(
    attrs: &BTreeMap<String, String>,
    max: usize,
) -> Result<(), GraphError> {
    if attrs.len() > max {
        return refuse(Reason::TooLarge, format!("more than {max} attributes"));
    }
    for (k, v) in attrs {
        if k.is_empty() || has_control(k) {
            return refuse(
                Reason::Invalid,
                "attribute keys must be non-empty without control characters",
            );
        }
        if k.len() > MAX_ATTR_KEY_BYTES || v.len() > MAX_ATTR_VALUE_BYTES {
            return refuse(
                Reason::TooLarge,
                format!(
                    "attribute '{}' over its size bound",
                    k.chars().take(32).collect::<String>()
                ),
            );
        }
    }
    Ok(())
}

/// Validates the sources and returns them sorted and deduped. Empty = refused.
pub(super) fn check_provenance(sources: &[String]) -> Result<Vec<String>, GraphError> {
    if sources.is_empty() {
        return refuse(
            Reason::NoProvenance,
            "a record needs a run id, a file path or a named actor",
        );
    }
    for s in sources {
        let rest = ["run:", "file:", "actor:"]
            .iter()
            .find_map(|p| s.strip_prefix(p));
        match rest {
            Some(r) if !r.trim().is_empty() && s.len() <= MAX_SOURCE_BYTES && !has_control(s) => {}
            _ => {
                return refuse(
                    Reason::BadProvenance,
                    "a source is run:<id>, file:<path> or actor:<name>, non-empty, up to 256 bytes",
                );
            }
        }
    }
    let mut out = sources.to_vec();
    out.sort();
    out.dedup();
    Ok(out)
}

/// Merges two sorted-deduped source lists.
pub(super) fn merge_sources(a: &[String], b: &[String]) -> Vec<String> {
    let mut out: Vec<String> = a.iter().chain(b).cloned().collect();
    out.sort();
    out.dedup();
    out
}

/// Merges attributes; for a clashing key the greater value wins (order-independent).
pub(super) fn merge_attrs(
    old: &BTreeMap<String, String>,
    new: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut out = old.clone();
    for (k, v) in new {
        out.entry(k.clone())
            .and_modify(|cur| {
                if v > cur {
                    cur.clone_from(v);
                }
            })
            .or_insert_with(|| v.clone());
    }
    out
}
