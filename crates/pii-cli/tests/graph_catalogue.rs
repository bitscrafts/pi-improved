//! Spec 388: `pii graph index` derives a catalogue of graphs into the memory store and `pii graph find` answers from it,
//! in incident form. Every test runs the built `pii` binary over a scratch copy of the shipped graphs, with its own
//! `HOME` and state dir.
//!
//! The shape of an answer, as these tests read it: a hit starts on an unindented line whose first word is the graph's
//! name, and its kinds and calls follow on that line or on indented lines under it. Any other unindented line, such as
//! the stale line, belongs to no hit.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout and stderr.

#[path = "daemon_guard/mod.rs"]
mod daemon_guard;

const FLOW_QUESTION: &str = "write the books";

/// A graph the loader refuses although it is well-formed TOML: no such node kind.
const UNKNOWN_KIND: &str = "name = \"broken-kind\"\nentry = \"a\"\nexit = \"a\"\ncaps = []\n\n[[node]]\nid = \"a\"\nkind = \"no-such-kind\"\n";
/// A graph no TOML parser accepts.
const UNPARSABLE: &str = "name = \"broken-syntax\"\n[[node]\nid = ";
/// A valid graph in the user directory, so the walk is seen to cover both directories (R1).
const USER_GRAPH: &str = "name = \"user-flow\"\nentry = \"step\"\nexit = \"step\"\ncaps = []\n\n[[node]]\nid = \"step\"\nkind = \"transform\"\nvalue = \"ok\"\n";

#[path = "graph_catalogue/agent.rs"]
pub mod agent;
#[path = "graph_catalogue/fixture.rs"]
pub mod fixture;

use fixture::*;

use pii_memory::doc::{DocKind, decode_kind, decode_text};
use std::collections::BTreeSet;

/// The shipped directory has no broken file left to report (spec 380 made every top-level graph load), so the two the
/// contract names are planted beside the copy. The fragment is the one shipped file that is not a graph on its own: the
/// count may or may not include it, and every real graph must be there either way.
#[test]
fn index_covers_every_loadable_graph() {
    let fx = Fixture::new();
    let broken = [
        ("broken-kind.toml", UNKNOWN_KIND),
        ("broken-syntax.toml", UNPARSABLE),
    ];
    for (file, text) in broken {
        let path = fx.graphs().join(file);
        std::fs::write(&path, text).unwrap();
        let lint = fx.pii(&["graph", "lint", path.to_str().unwrap()]);
        assert_ne!(
            lint.status.code(),
            Some(0),
            "{file} must not load: {lint:?}"
        );
    }
    let user_graphs = fx.state().join("graphs");
    std::fs::create_dir_all(&user_graphs).unwrap();
    std::fs::write(user_graphs.join("user-flow.toml"), USER_GRAPH).unwrap();

    let out = fx.pii(&["graph", "index", "--dir", "graphs"]);
    let text = combined(&out);
    let count = indexed_count(&text).unwrap_or_else(|| panic!("no 'indexed N graphs': {text}"));

    let all = shipped();
    let graphs = all.iter().filter(|g| !g.is_fragment).count();
    let fragments = all.len() - graphs;
    assert!(
        count == graphs + 1 || count == graphs + 1 + fragments,
        "{graphs} shipped graphs and the user's one must each have an entry, got {count}: {text}"
    );
    for (file, _) in broken {
        assert!(
            text.lines().any(|l| l.contains(file)),
            "a file that does not load must be reported by name, not skipped: {file}: {text}"
        );
    }

    let store = fx.memory();
    assert_eq!(
        usize::try_from(store.n_docs().unwrap()).unwrap(),
        count,
        "one document per graph, in an empty store"
    );
    let entries: Vec<String> = store
        .doc_ids()
        .unwrap()
        .iter()
        .map(|id| {
            let bytes = store.doc(id).unwrap();
            assert_eq!(
                decode_kind(bytes.clone()).unwrap(),
                DocKind::Skill,
                "a catalogue entry is a skill document: {id}"
            );
            decode_text(bytes)
        })
        .collect();
    let names = all
        .iter()
        .filter(|g| !g.is_fragment)
        .map(|g| g.name.as_str())
        .chain(["user-flow"]);
    for name in names {
        assert!(
            entries
                .iter()
                .any(|e| e.lines().any(|l| names_word(l, name))),
            "no catalogue entry names {name}"
        );
    }
    for (file, _) in broken {
        let stem = file.trim_end_matches(".toml");
        assert!(
            !entries.iter().any(|e| e.contains(stem)),
            "a file that does not load has no entry: {stem}"
        );
    }
}

#[test]
fn index_is_idempotent() {
    let fx = Fixture::new();
    let first = fx.index();
    let before = store_counts(&fx.memory());
    assert!(first > 0, "the first run indexed nothing");
    assert!(before.0 > 0, "the first run wrote no document: {before:?}");
    assert!(
        before.3 > 0,
        "kinds, ports and call targets are relationships in the store: {before:?}"
    );

    let second = fx.index();
    let after = store_counts(&fx.memory());
    assert_eq!(first, second, "both runs cover the same graphs");
    assert_eq!(before, after, "the second run must change nothing");
}

#[test]
fn find_names_the_graph_that_does_the_flow() {
    let fx = Fixture::new();
    fx.index();
    let out = fx.find(FLOW_QUESTION);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));

    let (hits, _) = read_answer(&stdout);
    let at = hits
        .iter()
        .position(|h| h.name == "books")
        .unwrap_or_else(|| panic!("books is not a candidate: {stdout}"));
    assert!(at < 3, "books must be in the top three: {stdout}");

    let books = shipped_named("books");
    assert!(
        !books.calls.is_empty(),
        "the shipped books graph calls docs"
    );
    let own = hits[at].lines.join("\n");
    for call in &books.calls {
        assert!(
            names_word(&own, call),
            "books without its call {call}: {own}"
        );
    }
    for kind in &books.kinds {
        assert!(
            names_word(&own, kind),
            "books without its kind {kind}: {own}"
        );
    }
}

#[test]
fn find_reports_calls_colocated() {
    let fx = Fixture::new();
    fx.index();
    let out = fx.find(FLOW_QUESTION);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let (hits, outside) = read_answer(&stdout);
    assert!(!hits.is_empty(), "no candidate: {}", combined(&out));

    let mut seen = BTreeSet::new();
    let mut with_calls = 0;
    for hit in &hits {
        assert!(
            seen.insert(hit.name.clone()),
            "{} heads two lines, so its edges are a flat list: {stdout}",
            hit.name
        );
        let calls = shipped_named(&hit.name).calls;
        with_calls += usize::from(!calls.is_empty());
        let own = hit.lines.join("\n");
        for call in &calls {
            assert!(
                names_word(&own, call),
                "{} calls {call}, and it must say so on its own lines: {stdout}",
                hit.name
            );
        }
    }
    assert!(with_calls > 0, "no candidate with calls to read: {stdout}");
    for line in &outside {
        assert!(
            !line.to_lowercase().contains("call") && !line.contains("->"),
            "calls in a section of their own: {line}: {stdout}"
        );
    }
}

#[test]
fn a_stale_catalogue_says_so() {
    let fx = Fixture::new();
    fx.index();
    let says_stale = |text: &str, file: &str| {
        text.lines()
            .any(|l| l.to_lowercase().contains("stale") && l.contains(file))
    };

    let fresh = combined(&fx.find(FLOW_QUESTION));
    assert!(
        !fresh.to_lowercase().contains("stale"),
        "nothing has changed: {fresh}"
    );

    // The content changes as well as the mtime, so the test holds whichever one the revision is built from.
    let touched = fx.graphs().join("tdd.toml");
    let mut text = std::fs::read_to_string(&touched).unwrap();
    text.push_str("\n# changed since the catalogue was built\n");
    std::fs::write(&touched, text).unwrap();

    let out = fx.find(FLOW_QUESTION);
    let answer = combined(&out);
    assert!(says_stale(&answer, "tdd.toml"), "no stale line: {answer}");
    assert_eq!(
        answer
            .lines()
            .filter(|l| l.to_lowercase().contains("stale"))
            .count(),
        1,
        "stale is said in one line: {answer}"
    );
    let (hits, _) = read_answer(&String::from_utf8_lossy(&out.stdout));
    assert!(
        hits.iter().any(|h| h.name == "books"),
        "a stale catalogue still answers: {answer}"
    );
}

#[test]
fn control_a_question_with_no_flow_answers_none() {
    let fx = Fixture::new();
    fx.index();
    let out = fx.find("translate this to Japanese");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "none is an answer: {out:?}");

    let (hits, _) = read_answer(&stdout);
    let invented: Vec<&str> = hits.iter().map(|h| h.name.as_str()).collect();
    assert!(invented.is_empty(), "invented {invented:?}: {stdout}");
    let lower = stdout.to_lowercase();
    assert!(
        lower.contains("none") || lower.contains("no candidate") || lower.contains("no graph"),
        "the answer must say there is none: {stdout}"
    );
}
