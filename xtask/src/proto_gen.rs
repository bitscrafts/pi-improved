//! `proto-gen` (F7, opus review): regenerate the checked-in JSON Schema for
//! the wire contract from the `schemars::JsonSchema` derives, and verify it
//! matches what's committed (the schema is the artifact, not a byproduct).

// reason: `pub(super)` is the correct visibility for these binary-internal
// helpers; clippy `redundant_pub_crate` conflicts with rustc `unreachable_pub`
// (deny), allowed narrowly per spec F16.
#![allow(clippy::redundant_pub_crate)]

use std::path::Path;

use schemars::schema_for;
use serde_json::Value;

const SCHEMA_DIR: &str = "crates/pii-proto/schema";

/// Regenerates `crates/pii-proto/schema/*.json` from the wire types.
///
/// # Errors
/// Returns a message on I/O failure.
pub(super) fn generate() -> Result<(), String> {
    let dir = Path::new(SCHEMA_DIR);
    std::fs::create_dir_all(dir).map_err(|e| format!("proto-gen: mkdir {SCHEMA_DIR}: {e}"))?;
    generate_into(dir)
}

/// Verifies the committed schema matches a fresh generation (CI gate).
///
/// # Errors
/// Returns a message listing any file that differs.
pub(super) fn check() -> Result<(), Vec<String>> {
    let mut violations = Vec::new();
    let dir = Path::new(SCHEMA_DIR);
    // Generate to a temp dir and diff file-by-file (guarantees the check and
    // the generator use the identical serialisation path).
    let tmp = std::env::temp_dir().join(format!("pii-proto-schema-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| vec![format!("proto-check: mkdir tmp: {e}")])?;
    generate_into(&tmp).map_err(|e| vec![e])?;

    for name in ["event", "session", "run", "sys"] {
        let committed = dir.join(format!("{name}.json"));
        let fresh = tmp.join(format!("{name}.json"));
        if !committed.exists() {
            violations.push(format!(
                "proto-check: {} is missing (run cargo xtask proto-gen)",
                committed.display()
            ));
            continue;
        }
        let committed_src = std::fs::read_to_string(&committed)
            .map_err(|e| vec![format!("proto-check: read {}: {e}", committed.display())])?;
        let fresh_src = std::fs::read_to_string(&fresh)
            .map_err(|e| vec![format!("proto-check: read {}: {e}", fresh.display())])?;
        if committed_src.trim() != fresh_src.trim() {
            violations.push(format!(
                "proto-check: {} differs from generated (run cargo xtask proto-gen)",
                committed.display()
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

/// Writes the four schemas into `dir`.
fn generate_into(dir: &Path) -> Result<(), String> {
    let schemas: Vec<(&str, Value)> = vec![
        (
            "event",
            serde_json::to_value(schema_for!(pii_proto::event::Event))
                .map_err(|e| format!("proto-gen: event schema: {e}"))?,
        ),
        (
            "session",
            serde_json::to_value(schema_for!(pii_proto::session::SessionStateResult))
                .map_err(|e| format!("proto-gen: session schema: {e}"))?,
        ),
        (
            "run",
            serde_json::to_value(schema_for!(pii_proto::run::TraceResult))
                .map_err(|e| format!("proto-gen: run schema: {e}"))?,
        ),
        (
            "sys",
            serde_json::to_value(schema_for!(pii_proto::sys::HelloResult))
                .map_err(|e| format!("proto-gen: sys schema: {e}"))?,
        ),
    ];
    for (name, schema) in schemas {
        let path = dir.join(format!("{name}.json"));
        let doc = serde_json::to_vec_pretty(&schema)
            .map_err(|e| format!("proto-gen: ser {name}: {e}"))?;
        std::fs::write(&path, doc)
            .map_err(|e| format!("proto-gen: write {}: {e}", path.display()))?;
    }
    Ok(())
}
