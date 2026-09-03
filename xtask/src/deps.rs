//! ABI dependency purity gate: direct allowlist + transitive denylist.

// reason: `pub(super)` is the widest these internal helpers need (used only by
// the parent `xtask::main` module and in-module tests); clippy pedantic
// `redundant_pub_crate` conflicts with rustc `unreachable_pub` (deny) here, so
// it is allowed narrowly per spec F16.
#![allow(clippy::redundant_pub_crate)]

use serde_json::Value;

const ABI_CRATE: &str = "pii-abi";
const ABI_ALLOWED_DIRECT: &[&str] = &["serde", "serde_json", "thiserror", "schemars", "bitflags"];
const FORBIDDEN_TRANSITIVE: &[&str] = &[
    "tokio", "reqwest", "redb", "wasmtime", "axum", "hyper", "rustls",
];

/// Pure check over parsed `cargo metadata` JSON.
///
/// Rule 1: every non-dev dependency of `pii-abi` must be in the direct
/// allowlist. Rule 2: BFS over `resolve.nodes` must never reach a crate
/// on the transitive denylist.
pub(super) fn check_abi_purity(metadata: &Value) -> Result<(), Vec<String>> {
    let mut violations = Vec::new();

    let pkg_id = find_package_id(metadata, ABI_CRATE);
    let Some(pkg_id) = pkg_id else {
        violations.push(format!("package '{ABI_CRATE}' not found in metadata"));
        return Err(violations);
    };

    check_direct_deps(metadata, pkg_id, &mut violations);
    check_transitive_deps(metadata, pkg_id, &mut violations);

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

fn find_package_id<'a>(metadata: &'a Value, name: &str) -> Option<&'a str> {
    metadata
        .get("packages")?
        .as_array()?
        .iter()
        .filter_map(Value::as_object)
        .find(|pkg| pkg.get("name").and_then(Value::as_str) == Some(name))
        .and_then(|pkg| pkg.get("id"))
        .and_then(Value::as_str)
}

fn check_direct_deps(metadata: &Value, pkg_id: &str, violations: &mut Vec<String>) {
    let Some(packages) = metadata.get("packages").and_then(Value::as_array) else {
        return;
    };
    let Some(pkg) = packages
        .iter()
        .find(|p| p.get("id").and_then(Value::as_str) == Some(pkg_id))
    else {
        return;
    };
    let Some(deps) = pkg.get("dependencies").and_then(Value::as_array) else {
        return;
    };
    for dep in deps {
        let Some(name) = dep.get("name").and_then(Value::as_str) else {
            continue;
        };
        let kind = dep.get("kind").and_then(Value::as_str);
        if kind == Some("dev") {
            continue;
        }
        if !ABI_ALLOWED_DIRECT.contains(&name) {
            violations.push(format!(
                "pii-abi: direct dependency '{name}' is not in the allowlist"
            ));
        }
    }
}

fn check_transitive_deps(metadata: &Value, pkg_id: &str, violations: &mut Vec<String>) {
    let Some(nodes) = metadata
        .get("resolve")
        .and_then(|r| r.get("nodes"))
        .and_then(Value::as_array)
    else {
        violations.push(String::from(
            "metadata has no resolve.nodes (run with --format-version 1)",
        ));
        return;
    };

    let name_of = |id: &str| -> Option<&str> {
        metadata
            .get("packages")?
            .as_array()?
            .iter()
            .filter_map(Value::as_object)
            .find(|p| p.get("id").and_then(Value::as_str) == Some(id))
            .and_then(|p| p.get("name"))
            .and_then(Value::as_str)
    };

    let mut visited: Vec<&str> = Vec::new();
    let mut queue: Vec<&str> = vec![pkg_id];
    while let Some(current) = queue.pop() {
        if visited.contains(&current) {
            continue;
        }
        visited.push(current);
        let Some(node) = nodes
            .iter()
            .find(|n| n.get("id").and_then(Value::as_str) == Some(current))
        else {
            continue;
        };
        let Some(deps) = node.get("deps").and_then(Value::as_array) else {
            continue;
        };
        for dep in deps {
            let Some(dep_pkg) = dep.get("pkg").and_then(Value::as_str) else {
                continue;
            };
            if let Some(dep_name) = name_of(dep_pkg)
                && FORBIDDEN_TRANSITIVE.contains(&dep_name)
            {
                violations.push(format!(
                    "pii-abi: transitive dependency '{dep_name}' is forbidden"
                ));
            }
            if !visited.contains(&dep_pkg) {
                queue.push(dep_pkg);
            }
        }
    }
}

/// Shells out to `cargo metadata` and runs the pure check.
///
/// # Errors
/// Returns the violation list, or an error line if metadata cannot be
/// produced.
pub(super) fn run() -> Result<(), Vec<String>> {
    let out = std::process::Command::new("cargo")
        .args(["metadata", "--format-version", "1"])
        .output()
        .map_err(|e| vec![format!("failed to run cargo metadata: {e}")])?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(vec![format!("cargo metadata failed: {}", stderr.trim())]);
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let metadata: Value = serde_json::from_str(stdout.trim())
        .map_err(|e| vec![format!("failed to parse cargo metadata JSON: {e}")])?;

    check_abi_purity(&metadata)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::check_abi_purity;
    use serde_json::{Value, json};

    fn metadata_with_abi_deps(deps: &Value, resolve_nodes: &Value) -> Value {
        json!({
            "packages": [
                {
                    "id": "registry+abi-1.0.0",
                    "name": "pii-abi",
                    "dependencies": deps
                }
            ],
            "resolve": { "nodes": resolve_nodes }
        })
    }

    /// Like `metadata_with_abi_deps` but also records the given packages so
    /// `name_of` can resolve `resolve.nodes` pkg ids to crate names.
    fn metadata_with_extra_packages(deps: &Value, resolve_nodes: &Value, extra: &Value) -> Value {
        let mut packages = serde_json::json!([
            { "id": "registry+abi-1.0.0", "name": "pii-abi", "dependencies": deps }
        ]);
        packages
            .as_array_mut()
            .unwrap()
            .extend(extra.as_array().cloned().unwrap_or_default());
        json!({ "packages": packages, "resolve": { "nodes": resolve_nodes } })
    }

    fn clean_direct_deps() -> Value {
        json!([
            { "name": "serde", "kind": null },
            { "name": "serde_json", "kind": null },
            { "name": "thiserror", "kind": null },
            { "name": "schemars", "kind": null }
        ])
    }

    #[test]
    fn deps_check_accepts_clean_abi() {
        let nodes = json!([
            { "id": "registry+abi-1.0.0", "deps": [
                { "pkg": "registry+serde-1.0.0" }
            ]},
            { "id": "registry+serde-1.0.0", "deps": [] }
        ]);
        let metadata = metadata_with_abi_deps(&clean_direct_deps(), &nodes);
        assert!(check_abi_purity(&metadata).is_ok());
    }

    #[test]
    fn deps_check_rejects_tokio_in_pii_abi() {
        let mut deps = clean_direct_deps();
        deps.as_array_mut()
            .unwrap()
            .push(json!({ "name": "tokio", "kind": null }));
        let nodes = json!([{ "id": "registry+abi-1.0.0", "deps": [] }]);
        let metadata = metadata_with_abi_deps(&deps, &nodes);
        let errs = check_abi_purity(&metadata).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("tokio")));
    }

    #[test]
    fn deps_check_rejects_forbidden_transitive_dep() {
        // pii-abi -> schemars (allowed) -> redb (forbidden), via resolve nodes.
        let nodes = json!([
            { "id": "registry+abi-1.0.0", "deps": [
                { "pkg": "registry+schemars-1.0.0" }
            ]},
            { "id": "registry+schemars-1.0.0", "deps": [
                { "pkg": "registry+redb-2.0.0" }
            ]},
            { "id": "registry+redb-2.0.0", "deps": [] }
        ]);
        let extra = json!([
            { "id": "registry+schemars-1.0.0", "name": "schemars", "dependencies": [] },
            { "id": "registry+redb-2.0.0", "name": "redb", "dependencies": [] }
        ]);
        let metadata = metadata_with_extra_packages(&clean_direct_deps(), &nodes, &extra);
        let errs = check_abi_purity(&metadata).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("redb")));
    }

    #[test]
    fn deps_check_ignores_dev_dependencies() {
        let mut deps = clean_direct_deps();
        deps.as_array_mut()
            .unwrap()
            .push(json!({ "name": "walkdir", "kind": "dev" }));
        let nodes = json!([{ "id": "registry+abi-1.0.0", "deps": [] }]);
        let metadata = metadata_with_abi_deps(&deps, &nodes);
        assert!(check_abi_purity(&metadata).is_ok());
    }
}
