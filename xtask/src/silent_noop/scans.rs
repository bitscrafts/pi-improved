//! The five silent-no-op scans (R1a, R1b, R1c, R1c-graphs, R4).

use std::path::Path;

use super::allowlist_for;
use super::helpers::{
    collect_crates_files, collect_rust_files, config_id, enum_variant_names, event_id,
    graph_key_id, is_filter_or_render_site, method_id,
};

/// Scan 1 (R1a): methods in `method.rs` whose `Method::X` has no dispatch arm.
pub(crate) fn scan_methods(root: &Path, violations: &mut Vec<String>) -> Vec<String> {
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
pub(crate) fn dispatch_has_arm(src: &str, name: &str) -> bool {
    src.contains(&format!("Method::{name} =>"))
}

/// Scan 2 (R1b): events with no producer outside proto/subscribe/tui.
pub(crate) fn scan_events(root: &Path, violations: &mut Vec<String>) -> Vec<String> {
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
pub(crate) fn scan_config(root: &Path, violations: &mut Vec<String>) -> Vec<String> {
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
pub(crate) fn scan_graph_keys(root: &Path, violations: &mut Vec<String>) -> Vec<String> {
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

/// Scan 5 (R4, spec 31): `pii_core::PROTECTED_RELATIVE_PATHS` is declared as
/// the deny list a heal run's `fs_write` scope must enforce — but a declared
/// list is not the same as an ENFORCED one. Checks that the identifier is
/// actually referenced at both chokepoints the spec names: `run_caps`'s
/// `fs_write` deny wiring, and `xtask heal-collect`'s pre-write refusal
/// check. A protected-path list with no enforcement site is a silent no-op
/// exactly like an unwired method or an unread config key.
pub(crate) fn scan_protected_paths(root: &Path, violations: &mut Vec<String>) -> Vec<String> {
    let decl = root.join("crates/pii-core/src/cap.rs");
    let Ok(decl_src) = std::fs::read_to_string(&decl) else {
        violations.push(format!("SILENT-NOOP FAIL: cannot read {}", decl.display()));
        return Vec::new();
    };
    if !decl_src.contains("PROTECTED_RELATIVE_PATHS") {
        // Nothing declared yet — no protected-path surface to enforce.
        return Vec::new();
    }
    let sites: &[(&str, &str)] = &[
        (
            "crates/pii-server/src/runs.rs",
            "run_caps fs_write deny wiring",
        ),
        (
            "xtask/src/heal_collect.rs",
            "heal-collect pre-write refusal check",
        ),
    ];
    let mut out = Vec::new();
    for (rel, desc) in sites {
        let id = format!("protected-path-site:{rel}");
        let wired = std::fs::read_to_string(root.join(rel)).is_ok_and(|s| protected_path_wired(&s));
        if wired {
            continue;
        }
        out.push(format!("{id} [UNWIRED]"));
        violations.push(format!(
            "SILENT-NOOP FAIL: PROTECTED_RELATIVE_PATHS is declared but not enforced at {rel} ({desc})"
        ));
    }
    out
}

/// `true` if `src` actually references the shared protected-path list.
pub(crate) fn protected_path_wired(src: &str) -> bool {
    src.contains("PROTECTED_RELATIVE_PATHS")
}
