//! The graph catalogue (spec 388): one skill document per graph, derived from the graph's file by `pii graph index`,
//! searched through the same lanes as every other memory document.
//!
//! `pii graph find` and the `graph_find` tool both
//! answer through [`find`], so they say the same thing. Nothing here grants anything: an entry holds only what the
//! graph file already publishes.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use pii_abi::PiiError;
use pii_core::Ctx;
use pii_store::MemStore;
use serde::{Deserialize, Serialize};

use crate::doc::{Doc, DocKind, Layer, decode_text, id_of, rows_for};
use crate::search::MemorySearcher;
use crate::skill::{is_skill, live, sha256_hex, slug_of};
use crate::tokenize::tokenize;

/// A catalogue document's slug is this prefix and the graph's name, so it can never take a hand-written skill's slug.
const SLUG_PREFIX: &str = "graph:";
/// How many candidates one answer names.
const MAX_CANDIDATES: usize = 5;
/// How deep the lanes are read: the store also holds facts, and they must not crowd the graphs out of the answer.
const SEARCH_DEPTH: usize = 256;

/// What one graph file publishes about itself (R1).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Entry {
    /// The graph's `name`.
    pub name: String,
    /// The file it was read from.
    pub file: String,
    /// The sha256 of that file when it was read.
    pub revision: String,
    /// One line of purpose.
    pub purpose: String,
    /// The rest of what the file says about itself, searched but not printed.
    pub about: String,
    /// The entry node.
    pub entry: String,
    /// The exit node.
    pub exit: String,
    /// The node kinds it uses.
    pub kinds: Vec<String>,
    /// The graphs it `call`s.
    pub calls: Vec<String>,
    /// The ports its nodes and edges name.
    pub ports: Vec<String>,
    /// Its capabilities.
    pub caps: Vec<String>,
}

impl Entry {
    /// The document's id, stable across revisions so a changed graph replaces its own entry.
    fn id(&self) -> String {
        id_of(&format!("graph-catalogue:{}", self.name))
    }

    fn text(&self) -> String {
        format!(
            "PII-SKILL\n{SLUG_PREFIX}{}\npurpose: {}\nkinds: {}\ncalls: {}\nentry: {}\nexit: {}\nports: {}\ncaps: {}\nabout: {}\nrevision: {}\nfile: {}\n",
            self.name,
            self.purpose,
            self.kinds.join(", "),
            self.calls.join(", "),
            self.entry,
            self.exit,
            self.ports.join(", "),
            self.caps.join(", "),
            self.about,
            self.revision,
            self.file,
        )
    }

    /// Reads a document back; `None` when it is not a catalogue document.
    fn parse(text: &str) -> Option<Self> {
        if !is_skill(text) {
            return None;
        }
        let name = slug_of(text).strip_prefix(SLUG_PREFIX)?.to_owned();
        let field = |key: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
                .map_or_else(String::new, |v| v.trim().to_owned())
        };
        let list = |key: &str| -> Vec<String> {
            field(key)
                .split(", ")
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .collect()
        };
        Some(Self {
            name,
            file: field("file"),
            revision: field("revision"),
            purpose: field("purpose"),
            about: field("about"),
            entry: field("entry"),
            exit: field("exit"),
            kinds: list("kinds"),
            calls: list("calls"),
            ports: list("ports"),
            caps: list("caps"),
        })
    }

    /// The graph lane's nodes (R2): its kinds, ports and call targets. Each is an edge to this document.
    fn concepts(&self) -> Vec<String> {
        let all = self.kinds.iter().chain(&self.ports).chain(&self.calls);
        let set: BTreeSet<String> = all.map(|c| c.to_lowercase()).collect();
        set.into_iter().collect()
    }

    /// Whether the entry itself says one of `wanted`. The vector lane ranks every document, so a place in the fused
    /// list is no evidence of an answer; the file path and revision are not what a graph does, and do not count.
    fn says_any(&self, wanted: &BTreeSet<String>) -> bool {
        let said = [
            self.name.as_str(),
            self.purpose.as_str(),
            self.about.as_str(),
            &self.kinds.join(" "),
            &self.calls.join(" "),
            &self.ports.join(" "),
        ]
        .join(" ");
        tokenize(&said).iter().any(|t| wanted.contains(t))
    }

    fn named_by(&self, wanted: &BTreeSet<String>) -> bool {
        tokenize(&self.name).iter().any(|t| wanted.contains(t))
    }

    /// The hit in incident form (R3): the name and purpose, with the kinds and outgoing calls on the hit's own lines.
    fn incident(&self) -> String {
        let calls = if self.calls.is_empty() {
            String::from("none")
        } else {
            self.calls.join(", ")
        };
        format!(
            "{}: {}\n  kinds: {}\n  calls: {calls}\n",
            self.name,
            self.purpose,
            self.kinds.join(", ")
        )
    }

    /// `true` when the file no longer holds the bytes this entry was built from.
    fn is_behind(&self) -> bool {
        std::fs::read(&self.file).map_or(true, |bytes| sha256_hex(&bytes) != self.revision)
    }

    fn file_name(&self) -> &str {
        Path::new(&self.file)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or(&self.file)
    }
}

/// What one [`sync`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Synced {
    /// Entries written, new or replaced.
    pub written: u64,
    /// Entries the store already held, byte for byte.
    pub unchanged: u64,
    /// Entries archived because their file is gone.
    pub removed: u64,
}

/// The entries the store holds, in id order.
///
/// # Errors
/// [`PiiError::Internal`] on a redb failure.
pub fn entries(store: &MemStore) -> Result<Vec<(String, Entry)>, PiiError> {
    Ok(live(store)?
        .into_iter()
        .filter_map(|(id, text)| Entry::parse(&text).map(|e| (id, e)))
        .collect())
}

/// Brings the store to `wanted` (R1): an entry the store already holds is not written at all, so a second run changes
/// nothing.
///
/// An entry whose file is gone is archived; one outside `wanted` whose file still exists is left alone,
/// because a run over another directory says nothing about it.
///
/// # Errors
/// [`PiiError::Internal`] on a redb failure.
pub fn sync(store: &MemStore, wanted: &[Entry], now_ms: u64) -> Result<Synced, PiiError> {
    let mut synced = Synced::default();
    for entry in wanted {
        let id = entry.id();
        let text = entry.text();
        let held = store.doc(&id)?;
        if held.is_some() && decode_text(held) == text {
            synced.unchanged += 1;
            continue;
        }
        let mut doc = Doc::new(
            Layer::L1,
            DocKind::Skill,
            text,
            entry.concepts(),
            1.0,
            now_ms,
        );
        doc.id = id;
        store.put(&rows_for(&doc))?;
        synced.written += 1;
    }
    for (id, held) in entries(store)? {
        if !wanted.iter().any(|e| e.name == held.name) && !Path::new(&held.file).exists() {
            store.archive(&id)?;
            synced.removed += 1;
        }
    }
    Ok(synced)
}

/// One answer: the candidates, the one line that says the catalogue is behind (R6), and the text both surfaces print.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Answer {
    /// The candidates' names, best first.
    pub hits: Vec<String>,
    /// The stale line, when a graph file has changed since the catalogue was built.
    pub stale: Option<String>,
    /// The whole answer as text.
    pub text: String,
}

/// Answers `question` from the catalogue through the memory lanes (R2, R3, R6).
///
/// # Errors
/// [`PiiError::Internal`] on a store failure.
pub fn find(
    searcher: &MemorySearcher,
    store: &MemStore,
    ctx: &Ctx,
    question: &str,
) -> Result<Answer, PiiError> {
    let wanted: BTreeSet<String> = tokenize(question).into_iter().collect();
    let mut hits: Vec<Entry> = searcher
        .search(ctx, question, SEARCH_DEPTH)?
        .hits
        .iter()
        .filter_map(|h| Entry::parse(&h.text))
        .filter(|e| e.says_any(&wanted))
        .collect();
    // Stable: a graph the question names comes first, and the lanes' order holds otherwise.
    hits.sort_by_key(|e| !e.named_by(&wanted));
    hits.truncate(MAX_CANDIDATES);

    let stale = stale_line(store)?;
    let mut text = String::new();
    if let Some(line) = &stale {
        let _ = writeln!(text, "{line}");
    }
    if hits.is_empty() {
        let _ = writeln!(text, "no candidate: no graph in the catalogue does this");
    }
    for hit in &hits {
        text.push_str(&hit.incident());
    }
    Ok(Answer {
        hits: hits.into_iter().map(|e| e.name).collect(),
        stale,
        text,
    })
}

/// The one line naming every file that moved past its entry, with the revision the entry describes.
fn stale_line(store: &MemStore) -> Result<Option<String>, PiiError> {
    let behind: Vec<String> = entries(store)?
        .iter()
        .filter(|(_, e)| e.is_behind())
        .map(|(_, e)| {
            let revision = e.revision.get(..12).unwrap_or(&e.revision);
            format!("{} (catalogue has {revision})", e.file_name())
        })
        .collect();
    Ok((!behind.is_empty()).then(|| {
        format!(
            "stale: changed since the last index: {}; run pii graph index",
            behind.join(", ")
        )
    }))
}
