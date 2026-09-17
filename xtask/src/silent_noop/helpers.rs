//! Single-purpose helpers for the `silent-noop-check` scans.

use std::path::{Path, PathBuf};

/// Surface id for a method.
pub(crate) fn method_id(name: &str) -> String {
    format!("method:{name}")
}

/// Surface id for an event.
pub(crate) fn event_id(name: &str) -> String {
    format!("event:{name}")
}

/// Surface id for a config key.
pub(crate) fn config_id(name: &str) -> String {
    format!("config:{name}")
}

/// Surface id for a graph node key.
pub(crate) fn graph_key_id(name: &str) -> String {
    format!("graph-key:{name}")
}

/// Extracts `VariantName` items of `pub enum NAME { ... }` (the first matching
/// enum on the string is used). Stops at the enum's closing brace via brace
/// counting (handles nested braces in doc-comments-turned-inline only crudely,
/// which is fine for these flat enums).
pub(crate) fn enum_variant_names(src: &str, enum_name: &str) -> Vec<String> {
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

/// Collects every `.rs` file under `crates/`, `graphs/`, skipped bins/targets.
pub(crate) fn collect_rust_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
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

/// Collects `.rs` files under `crates/` only.
pub(crate) fn collect_crates_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
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

/// True for `subscribe.rs` and `pii-tui` paths — filter/render sites, which are
/// NOT event producers.
pub(crate) fn is_filter_or_render_site(p: &Path) -> bool {
    let s = p.to_string_lossy();
    s.contains("subscribe.rs") || s.contains("pii-tui")
}

/// R1d: an allowlist entry with an EMPTY reason is itself a violation.
pub(crate) fn require_written_reasons(
    allowlist: Vec<super::AllowEntry>,
    violations: &mut Vec<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    for a in allowlist {
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
