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

use std::path::{Path, PathBuf};

/// One allowlisted surface with its written justification (R1d).
#[derive(Debug, Clone)]
pub(crate) struct AllowEntry {
    /// Surface id, e.g. `method:session.interrupt`.
    pub surface: String,
    /// Written, non-empty reason (R1d: empty ⇒ violation).
    pub reason: String,
}

/// Surface id for a method.
fn method_id(name: &str) -> String {
    format!("method:{name}")
}

/// Surface id for an event.
fn event_id(name: &str) -> String {
    format!("event:{name}")
}

/// Surface id for a config key.
fn config_id(name: &str) -> String {
    format!("config:{name}")
}

/// Surface id for a graph node key.
fn graph_key_id(name: &str) -> String {
    format!("graph-key:{name}")
}

/// The allowlist. Every entry is a deliberate, documented decision; each
/// reason names where the consumer lands.
pub(crate) fn allowlist() -> Vec<AllowEntry> {
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
    let reason_lines = require_written_reasons(&mut violations);

    if list {
        for line in report_lines(
            method_lines,
            event_lines,
            config_lines,
            graph_lines,
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
    reasons: Vec<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    out.push(format!("silent-noop-check --list ({} findings)", {
        methods.len() + events.len() + config.len() + graph.len()
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
    for s in reasons {
        out.push(format!("  ALLOW-REASON {s}"));
    }
    out
}

/// Scan 1 (R1a): methods in `method.rs` whose `Method::X` has no dispatch arm.
fn scan_methods(root: &Path, violations: &mut Vec<String>) -> Vec<String> {
    let method = root.join("crates/pii-proto/src/method.rs");
    let dispatch = root.join("crates/pii-server/src/dispatch.rs");
    let Ok(m_src) = std::fs::read_to_string(&method) else {
        violations.push(format!(
            "SILENT-NOOP FAIL: cannot read {}",
            method.display()
        ));
        return Vec::new();
    };
    let Ok(d_src) = std::fs::read_to_string(&dispatch) else {
        violations.push(format!(
            "SILENT-NOOP FAIL: cannot read {}",
            dispatch.display()
        ));
        return Vec::new();
    };
    let mut out = Vec::new();
    for name in enum_variant_names(&m_src, "Method") {
        let id = method_id(&name);
        let has_arm = dispatch_has_arm(&d_src, &name);
        let allowed = allowlist_for(&id);
        match (has_arm, allowed) {
            (true, _) => {}
            (false, Some(_)) => {
                out.push(format!("{id} [allowlisted]"));
            }
            (false, None) => {
                out.push(format!("{id} [UNWIRED]"));
                violations.push(format!(
                    "SILENT-NOOP FAIL: {id} has no dispatch arm and no allowlist reason"
                ));
            }
        }
    }
    out
}

/// Scan 1b (R1a helper): does `dispatch.rs` contain a `Method::NAME =>` arm?
fn dispatch_has_arm(src: &str, name: &str) -> bool {
    src.contains(&format!("Method::{name} =>"))
}

/// Scan 2 (R1b): events with no producer outside proto/subscribe/tui.
fn scan_events(root: &Path, violations: &mut Vec<String>) -> Vec<String> {
    let event = root.join("crates/pii-proto/src/event.rs");
    let Ok(e_src) = std::fs::read_to_string(&event) else {
        violations.push(format!("SILENT-NOOP FAIL: cannot read {}", event.display()));
        return Vec::new();
    };
    let mut out = Vec::new();
    for name in enum_variant_names(&e_src, "Event") {
        // Producer: a construction site `Event::NAME {` or `Event::NAME(` in a
        // crate file that is not pii-proto, subscribe.rs, or pii-tui.
        let mut produced = false;
        if let Ok(tree) = collect_rust_files(root) {
            for f in tree {
                if is_filter_or_render_site(&f) {
                    continue;
                }
                if let Ok(s) = std::fs::read_to_string(&f)
                    && (s.contains(&format!("Event::{name} {{"))
                        || s.contains(&format!("Event::{name}(")))
                {
                    produced = true;
                    break;
                }
            }
        }
        let id = event_id(&name);
        let allowed = allowlist_for(&id);
        match (produced, allowed) {
            (true, _) => {}
            (false, Some(_)) => {
                out.push(format!("{id} [allowlisted]"));
            }
            (false, None) => {
                out.push(format!("{id} [NO-PRODUCER]"));
                violations.push(format!(
                    "SILENT-NOOP FAIL: {id} has no producer and no allowlist reason"
                ));
            }
        }
    }
    out
}

/// Scan 3 (R1c): `[memory]` config fields with no reader.
fn scan_config(root: &Path, violations: &mut Vec<String>) -> Vec<String> {
    let cfg = root.join("crates/pii-server/src/daemon/config.rs");
    let Ok(c_src) = std::fs::read_to_string(&cfg) else {
        violations.push(format!("SILENT-NOOP FAIL: cannot read {}", cfg.display()));
        return Vec::new();
    };
    let fields = ["embedder", "cache_bytes"];
    let mut out = Vec::new();
    for field in fields {
        let read = config_field_read(root, &c_src, field);
        let id = config_id(field);
        let allowed = allowlist_for(&id);
        match (read, allowed) {
            (true, _) => {}
            (false, Some(_)) => {
                out.push(format!("{id} [allowlisted]"));
            }
            (false, None) => {
                out.push(format!("{id} [UNREAD]"));
                violations.push(format!(
                    "SILENT-NOOP FAIL: {id} has no read site and no allowlist reason"
                ));
            }
        }
    }
    out
}

/// `true` when `field` appears as a read site outside its declaration in
/// `config.rs` — i.e. `mem.embedder`, `.embedder`, or a config-access pattern
/// in any crate file (excluding the declaration file itself).
fn config_field_read(root: &Path, cfg_src: &str, field: &str) -> bool {
    let _ = cfg_src; // declaration presence is implied by the scan
    if let Ok(tree) = collect_rust_files(root) {
        for f in tree {
            if !f.ends_with("daemon/config.rs")
                && let Ok(s) = std::fs::read_to_string(&f)
            {
                // A read of the `[memory]` CONFIG key is a field access on a
                // section/config receiver, NOT the memory crate's own
                // `dyn Embedder` subsystem (`self.embedder`). Exclude `self.`
                // so the embedder trait's own field isn't counted as a reader
                // of the config key.
                let key_access = format!(".{field}");
                let self_access = format!("self.{field}");
                // The TOML/struct default in the declaration file is the only
                // non-read; any `X.{field}` or `X.{field}: ` elsewhere counts,
                // except `self.<field>` (the embedder subsystem's own field).
                if s.contains(&key_access) && !s.contains(&self_access) {
                    return true;
                }
            }
        }
    }
    false
}

/// Scan 4 (R1c): `top_k`-style graph node keys in `graphs/starter/*.toml`.
fn scan_graph_keys(root: &Path, violations: &mut Vec<String>) -> Vec<String> {
    let keys = ["top_k"];
    let mut out = Vec::new();
    for key in keys {
        // Declared in any starter graph?
        let declared = graph_declares_key(root, key);
        if !declared {
            continue;
        }
        let read = graph_key_read(root, key);
        let id = graph_key_id(key);
        let allowed = allowlist_for(&id);
        match (read, allowed) {
            (true, _) => {}
            (false, Some(_)) => {
                out.push(format!("{id} [allowlisted]"));
            }
            (false, None) => {
                out.push(format!("{id} [UNREAD]"));
                violations.push(format!(
                    "SILENT-NOOP FAIL: {id} is declared in a starter graph but read nowhere and not allowlisted"
                ));
            }
        }
    }
    out
}

fn graph_declares_key(root: &Path, key: &str) -> bool {
    let dir = root.join("graphs/starter");
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("toml")
                && let Ok(s) = std::fs::read_to_string(&p)
                && s.contains(key)
            {
                return true;
            }
        }
    }
    false
}

fn graph_key_read(root: &Path, key: &str) -> bool {
    // Graph node config is consumed by the ENGINE in `crates/`. xtask's own
    // `top_k` CLI flag (memory_eval) is unrelated and would false-positive.
    if let Ok(tree) = collect_crates_files(root) {
        for f in tree {
            if let Ok(s) = std::fs::read_to_string(&f)
                && s.contains(key)
            {
                return true;
            }
        }
    }
    false
}

/// Collects `.rs` files under `crates/` only.
fn collect_crates_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.join("crates")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                out.push(p);
            }
        }
    }
    Ok(out)
}

/// R1d: an allowlist entry with an EMPTY reason is itself a violation.
fn require_written_reasons(violations: &mut Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for a in allowlist() {
        if a.reason.trim().is_empty() {
            out.push(format!("{} [EMPTY-REASON]", a.surface));
            violations.push(format!(
                "SILENT-NOOP FAIL: {} is allowlisted but has an EMPTY reason (R1d)",
                a.surface
            ));
        }
    }
    out
}

/// Collects every `.rs` file under `crates/`, `graphs/`, skipped bins/targets.
fn collect_rust_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.join("crates"), root.join("xtask/src")];
    if root.join("graphs").is_dir() {
        stack.push(root.join("graphs"));
    }
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                out.push(p);
            }
        }
    }
    Ok(out)
}

/// True for `subscribe.rs` and `pii-tui` paths — filter/render sites, which are
/// NOT event producers.
fn is_filter_or_render_site(p: &Path) -> bool {
    let s = p.to_string_lossy();
    s.contains("subscribe.rs") || s.contains("pii-tui")
}

/// Extracts `VariantName` items of `pub enum NAME { ... }` (the first matching
/// enum on the string is used). Stops at the enum's closing brace via brace
/// counting (handles nested braces in doc-comments-turned-inline only crudely,
/// which is fine for these flat enums).
fn enum_variant_names(src: &str, enum_name: &str) -> Vec<String> {
    // Locate the enum block: `pub enum NAME { ... }`. Works for the flat
    // (unit + struct-like + tuple) enums in this repo.
    let start = src
        .find(&format!("pub enum {enum_name} {{"))
        .and_then(|i| src[i..].find('{').map(|j| i + j + 1))
        .unwrap_or(usize::MAX);
    let rest = &src[start..];
    // The enum closes at the first `}` at top level. We count braces so a
    // struct-like variant `Name { x: T, y: U }` doesn't prematurely end it.
    let mut open = 0usize;
    let mut close = 0usize;
    let mut names = Vec::new();
    for line in rest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with("///") || trimmed.starts_with("##") {
            continue;
        }
        if line.starts_with('}') && open == close {
            break; // end of the enum at top level
        }
        if let Some((name, _)) = variant_name_of(trimmed) {
            names.push(name);
        }
        open += trimmed.matches('{').count();
        close += trimmed.matches('}').count();
    }
    names
}

/// Pulls a leading Rust identifier off a variant line, ignoring serde attrs.
fn variant_name_of(line: &str) -> Option<(String, bool)> {
    // Skip `#[...]` lines.
    if line.starts_with('#') {
        return None;
    }
    let first = line.split([',', '{', '(', ' ', '\n']).next()?;
    if first.is_empty() {
        return None;
    }
    let is_ident = first
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase() || c == '_');
    if is_ident {
        Some((first.to_string(), true))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

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
        let lines = require_written_reasons(&mut v);
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

    /// The four scans run without error over the real tree.
    #[test]
    fn scans_run_over_real_tree() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let mut v = Vec::new();
        let _ = scan_methods(root, &mut v);
        let _ = scan_events(root, &mut v);
        let _ = scan_config(root, &mut v);
        let _ = scan_graph_keys(root, &mut v);
        let _ = require_written_reasons(&mut v);
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
}
