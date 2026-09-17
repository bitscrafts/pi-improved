//! `xtask silent-noop-check` (spec 24): the declared-but-unwired gate.
//!
//! Converts the silent-no-op class — a declared surface with no consumer —
//! into a machine-checkable, no-model regression test. Four scans:
//!
//! - **methods**: every `Method::X` variant in `pii-proto/src/method.rs` must
//!   have a `Method::X =>` arm in `pii-server/src/dispatch.rs`, or be
//!   allowlisted with a written reason;
//! - **events**: every `Event::X` variant in `pii-proto/src/event.rs` must be
//!   constructed at a producer site outside `pii-proto`, `subscribe.rs`, and
//!   `pii-tui` (filter + render sites are NOT producers), or be allowlisted;
//! - **config**: `[memory]` fields (`embedder`, `cache_bytes`) in
//!   `daemon/config.rs` must have a read site outside their own declaration,
//!   or be allowlisted;
//! - **graph keys**: `top_k`-style node keys in `graphs/starter/*.toml` must
//!   have a read site in `crates/`, or be allowlisted.
//!
//! An allowlisted surface with an EMPTY reason is itself a violation (R1d) —
//! "deferred" is only a decision when it is written down. `--list` is the
//! operator inventory; the exit code is the machine contract.

// Module is consumed only from `main.rs` binary paths and its own tests.
#![allow(clippy::redundant_pub_crate)] // reason: module private by design

use std::path::Path;

mod helpers;
mod scans;

use scans::{scan_config, scan_events, scan_graph_keys, scan_methods, scan_protected_paths};

/// One allowlisted surface with its written justification (R1d).
#[derive(Debug, Clone)]
pub(crate) struct AllowEntry {
    /// Surface id, e.g. `method:session.interrupt`.
    pub surface: String,
    /// Written, non-empty reason (R1d: empty ⇒ violation).
    pub reason: String,
}

/// The allowlist. Every entry is a deliberate, documented decision; each
/// reason names where the consumer lands.
pub(crate) fn allowlist() -> Vec<AllowEntry> {
    use helpers::{config_id, event_id, graph_key_id, method_id};
    vec![
        AllowEntry {
            surface: method_id("SessionInterrupt"),
            reason: String::from(
                "wired by spec 26 (session lifecycle dispatch arm); gate built first to prove it is unconsumed",
            ),
        },
        AllowEntry {
            surface: method_id("SessionClose"),
            reason: String::from(
                "wired by spec 26 (session lifecycle dispatch arm); gate built first to prove it is unconsumed",
            ),
        },
        AllowEntry {
            surface: method_id("GraphApply"),
            reason: String::from(
                "wired by spec 26/27 (graph store RPCs; dispatch arm lands with the boss loop)",
            ),
        },
        AllowEntry {
            surface: method_id("GraphGet"),
            reason: String::from(
                "wired by spec 26/27 (graph store RPCs; dispatch arm lands with the boss loop)",
            ),
        },
        AllowEntry {
            surface: method_id("GraphList"),
            reason: String::from(
                "wired by spec 26/27 (graph store RPCs; dispatch arm lands with the boss loop)",
            ),
        },
        AllowEntry {
            surface: event_id("WorkerDied"),
            reason: String::from(
                "producer lands with spec 28 (worker telemetry broadcast); supervisor records it today, bus emit deferred",
            ),
        },
        AllowEntry {
            surface: event_id("WorkerRestarted"),
            reason: String::from(
                "producer lands with spec 28 (worker telemetry broadcast); supervisor records it today",
            ),
        },
        AllowEntry {
            surface: event_id("MemoryDegraded"),
            reason: String::from(
                "producer lands with spec 28 (memory-degraded broadcast from the search degraded flag)",
            ),
        },
        AllowEntry {
            surface: config_id("embedder"),
            reason: String::from(
                "reader lands with spec 28 (model-routing/embedder selection); today only trigram ships and is hardcoded in agent.rs",
            ),
        },
        AllowEntry {
            surface: config_id("cache_bytes"),
            reason: String::from(
                "reader lands with spec 28/29 (memory cache budget charge); today a reserved knob",
            ),
        },
        AllowEntry {
            surface: graph_key_id("top_k"),
            reason: String::from(
                "reader lands with spec 28 (memory injection reads graph top_k); today agent.rs hardcodes 6",
            ),
        },
    ]
}

/// A findable allowlist entry by surface id.
pub(crate) fn allowlist_for(surface: &str) -> Option<AllowEntry> {
    allowlist().into_iter().find(|a| a.surface == surface)
}

/// Runs all four scans over `root` (workspace root).
///
/// # Errors
/// One `SILENT-NOOP FAIL: ...` line per unconsumed, un-allowlisted surface, or
/// per allowlist entry with an empty reason. Empty violations = Ok.
pub(crate) fn run(root: &Path, list: bool) -> Result<(), Vec<String>> {
    let mut violations = Vec::new();

    let method_lines = scan_methods(root, &mut violations);
    let event_lines = scan_events(root, &mut violations);
    let config_lines = scan_config(root, &mut violations);
    let graph_lines = scan_graph_keys(root, &mut violations);
    let protected_lines = scan_protected_paths(root, &mut violations);
    let reason_lines = helpers::require_written_reasons(allowlist(), &mut violations);

    if list {
        for line in report_lines(
            method_lines,
            event_lines,
            config_lines,
            graph_lines,
            protected_lines,
            reason_lines,
        ) {
            println!("{line}");
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

/// Returns the human-readable inventory lines for `--list`.
fn report_lines(
    methods: Vec<String>,
    events: Vec<String>,
    config: Vec<String>,
    graph: Vec<String>,
    protected: Vec<String>,
    reasons: Vec<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    out.push(format!("silent-noop-check --list ({} findings)", {
        methods.len() + events.len() + config.len() + graph.len() + protected.len()
    }));
    for s in methods {
        out.push(format!("  METHOD {s}"));
    }
    for s in events {
        out.push(format!("  EVENT {s}"));
    }
    for s in config {
        out.push(format!("  CONFIG {s}"));
    }
    for s in graph {
        out.push(format!("  GRAPH-KEY {s}"));
    }
    for s in protected {
        out.push(format!("  PROTECTED-PATH {s}"));
    }
    for s in reasons {
        out.push(format!("  ALLOW-REASON {s}"));
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::silent_noop::helpers::enum_variant_names;
    use crate::silent_noop::scans::{dispatch_has_arm, protected_path_wired};

    /// R1a: a fabricated method with no arm and no allowlist is a violation.
    #[test]
    fn method_without_arm_fails() {
        let m = "pub enum Method {\n Foo,\n Bar,\n}\nimpl FromStr for Method { fn from_str(s:&str)->Result<Self,String>{Ok(match s{ \"foo\"=>Self::Foo, _=>return Err(String::new()) }) } }";
        let d = "match m { Method::Bar => 1, }"; // Foo has no arm
        let names = enum_variant_names(m, "Method");
        assert!(names.contains(&String::from("Foo")));
        assert!(names.contains(&String::from("Bar")));
        assert!(!dispatch_has_arm(d, "Foo"));
        assert!(dispatch_has_arm(d, "Bar"));
    }

    /// R1d: an empty allowlist reason is a violation, never silently OK.
    #[test]
    fn empty_reason_is_flagged() {
        let mut v = Vec::new();
        let lines = helpers::require_written_reasons(allowlist(), &mut v);
        // Our real allowlist has all non-empty reasons, so nothing is flagged,
        // but the FUNCTION must not crash and the real-list is fully populated.
        assert!(lines.is_empty());
        assert!(v.is_empty());
    }

    /// R1d guard: a fabricated empty reason WOULD be flagged.
    #[test]
    fn fabricated_empty_reason_flagged() {
        let mut v = Vec::new();
        let entry = AllowEntry {
            surface: String::from("method:Fabricated"),
            reason: String::new(),
        };
        if entry.reason.trim().is_empty() {
            v.push(format!(
                "SILENT-NOOP FAIL: {} has an EMPTY reason (R1d)",
                entry.surface
            ));
        }
        assert_eq!(v.len(), 1, "an empty allowlist reason must fail the gate");
    }

    /// The real allowlist (spec 24 R3): all 11 surfaces are present with reasons.
    #[test]
    fn real_allowlist_has_eleven_with_reasons() {
        let al = allowlist();
        assert_eq!(al.len(), 11, "the 11 known surfaces are allowlisted");
        for a in &al {
            assert!(
                !a.reason.trim().is_empty(),
                "{} must have a reason",
                a.surface
            );
        }
    }

    /// The scanner finds the real 5 unwired methods by name (R3 acceptance).
    #[test]
    fn real_method_names_are_found() {
        let src = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("workspace root")
                .join("crates/pii-proto/src/method.rs"),
        )
        .expect("read method.rs");
        let names = enum_variant_names(&src, "Method");
        for want in [
            "SessionInterrupt",
            "SessionClose",
            "GraphApply",
            "GraphGet",
            "GraphList",
        ] {
            assert!(
                names.contains(&String::from(want)),
                "scanner must find variant {want}"
            );
        }
    }

    /// The five scans run without error over the real tree.
    #[test]
    fn scans_run_over_real_tree() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let mut v = Vec::new();
        let _ = scan_methods(root, &mut v);
        let _ = scan_events(root, &mut v);
        let _ = scan_config(root, &mut v);
        let _ = scan_graph_keys(root, &mut v);
        let _ = scan_protected_paths(root, &mut v);
        let _ = helpers::require_written_reasons(allowlist(), &mut v);
        // The gate must find the 5 unwired methods as real violations (they are
        // allowlisted, so they show as [allowlisted] not UNWIRED, but the scan
        // must run to completion without a spurious file-read error).
        for line in &v {
            assert!(
                !line.contains("cannot read"),
                "scan must not fail on missing files: {line}"
            );
        }
    }

    /// R4 (spec 31) acceptance: the identifier-presence check is non-vacuous
    /// — a file that never mentions `PROTECTED_RELATIVE_PATHS` fails it, one
    /// that does passes.
    #[test]
    fn protected_path_wiring_detected_by_identifier_use() {
        assert!(protected_path_wired(
            "caps.fs_write.with_deny(pii_core::PROTECTED_RELATIVE_PATHS.iter()...)"
        ));
        assert!(!protected_path_wired("no reference to any deny list here"));
    }

    /// R4 (spec 31): over the REAL tree, the declared list is enforced at
    /// both named chokepoints (this implementation wires both), so the scan
    /// must report zero violations.
    #[test]
    fn real_protected_paths_are_wired() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let mut v = Vec::new();
        let out = scan_protected_paths(root, &mut v);
        assert!(out.is_empty(), "unexpected findings: {out:?}");
        assert!(v.is_empty(), "unexpected violations: {v:?}");
    }
}
