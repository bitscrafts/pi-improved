//! Dependency DAG purity gate (pass-2 §12.2): four hard invariants, each a
//! CI assertion that must provably fail when violated.

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

/// Invariant 2: `pii-proto` (default features) must never depend on
/// `pii-core`/`pii-abi`/`pii-store`/`pii-graph`/`axum`. `tokio` is an
/// *optional* dep (behind the `client` feature) so it is allowed; the
/// resolved graph legitimately contains it when a client enables the feature.
const PROTO_FORBIDDEN: &[&str] = &["pii-core", "pii-abi", "pii-store", "pii-graph", "axum"];
/// Direct non-optional deps of `pii-proto` must be exactly these.
const PROTO_ALLOWED_DIRECT: &[&str] = &["serde", "serde_json", "schemars", "thiserror"];

/// Invariant 3: client crates must not have `pii-graph` in their dependency
/// tree. B2 (011 A.11): generalised from `pii-tui` alone to cover every
/// client (`pii-tui`, `pii-web`) — the machine-checkable proof that the
/// client/server split (002 §12.2) is real, not a proof about one crate.
const CLIENT_CRATES: &[&str] = &["pii-tui", "pii-web"];
const CLIENT_FORBIDDEN: &[&str] = &["pii-graph"];

/// Invariant 4: exactly one version each of these in the resolved graph.
const SINGLE_VERSION: &[&str] = &["redb", "reqwest", "wasmtime", "serde"];

/// Invariant 5 (F4.2/F7.3): `redb` may appear only under `pii-store` — the
/// memory index lives in `pii-store/src/mem.rs`; `pii-memory` consumes it
/// without touching redb.
const REDB_ALLOWED: &[&str] = &["pii-store"];

/// Invariant 5: no package outside `pii-store` may depend on `redb`.
/// Node deps are `{"pkg": id}` entries resolved against the package list,
/// matching the shape `check_transitive_deps` already consumes.
///
/// Spec 17 (R5) extends the same machinery to rustls: TLS is a `pii-net`
/// implementation detail behind the single-connect chokepoint — no other
/// crate may depend on it (G8/G9).
fn check_confined(metadata: &Value, dep: &str, allowed: &[&str], violations: &mut Vec<String>) {
    let nodes = metadata
        .get("resolve")
        .and_then(|r| r.get("nodes"))
        .and_then(Value::as_array);
    let Some(nodes) = nodes else {
        violations.push(String::from(
            "metadata has no resolve.nodes (run with --format-version 1)",
        ));
        return;
    };
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let name_of_id = |id: &str| -> String {
        packages
            .iter()
            .find_map(|p| {
                (p.get("id").and_then(Value::as_str) == Some(id))
                    .then(|| p.get("name").and_then(Value::as_str).unwrap_or_default())
            })
            .unwrap_or_default()
            .to_string()
    };
    for node in nodes {
        let node_id = node.get("id").and_then(Value::as_str).unwrap_or_default();
        let name = name_of_id(node_id);
        if allowed.contains(&name.as_str()) {
            continue;
        }
        for d in node
            .get("deps")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let dep_id = d.get("pkg").and_then(Value::as_str).unwrap_or_default();
            if name_of_id(dep_id) == dep {
                violations.push(format!(
                    "deps-check: '{name}' depends on {dep} outside {allowed:?}"
                ));
            }
        }
    }
}

fn check_redb_confined(metadata: &Value, violations: &mut Vec<String>) {
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
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let name_of_id = |id: &str| -> String {
        packages
            .iter()
            .find_map(|p| {
                (p.get("id").and_then(Value::as_str) == Some(id))
                    .then(|| p.get("name").and_then(Value::as_str).unwrap_or_default())
            })
            .unwrap_or_default()
            .to_string()
    };
    for node in nodes {
        let node_id = node.get("id").and_then(Value::as_str).unwrap_or_default();
        let name = name_of_id(node_id);
        if REDB_ALLOWED.contains(&name.as_str()) {
            continue;
        }
        for d in node
            .get("deps")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let dep_id = d.get("pkg").and_then(Value::as_str).unwrap_or_default();
            if name_of_id(dep_id) == "redb" {
                violations.push(format!(
                    "deps-check: invariant 5 — '{name}' depends on redb outside pii-store"
                ));
            }
        }
    }
}

/// Runs every invariant and returns the combined violation list.
pub(crate) fn check_all(metadata: &Value) -> Result<(), Vec<String>> {
    let mut violations = Vec::new();
    check_abi_purity(metadata, &mut violations);
    check_proto_purity(metadata, &mut violations);
    check_clients_no_graph(metadata, &mut violations);
    check_single_versions(metadata, &mut violations);
    check_redb_confined(metadata, &mut violations);
    check_confined(metadata, "rustls", &["pii-net"], &mut violations);
    // P4.6 (spec 19 N2): axum is a pii-web-only dependency — the workspace
    // root and pii-cli stay lean; the web viewer is a separate crate.
    check_confined(metadata, "axum", &["pii-web"], &mut violations);
    // P4.7 (spec 21 N1): rmcp is a pii-mcp-only dependency — it quarantines
    // rmcp's reqwest 0.13 requirement so core crates don't drag in a
    // conflicting HTTP stack.
    check_confined(metadata, "rmcp", &["pii-mcp"], &mut violations);
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

/// Invariant 1: every non-dev dependency of `pii-abi` is in the direct
/// allowlist, and BFS over `resolve.nodes` never reaches a forbidden crate.
fn check_abi_purity(metadata: &Value, violations: &mut Vec<String>) {
    let Some(pkg_id) = find_package_id(metadata, ABI_CRATE) else {
        violations.push(format!("package '{ABI_CRATE}' not found in metadata"));
        return;
    };
    check_direct_deps(metadata, pkg_id, ABI_ALLOWED_DIRECT, ABI_CRATE, violations);
    check_transitive_deps(
        metadata,
        pkg_id,
        FORBIDDEN_TRANSITIVE,
        ABI_CRATE,
        violations,
    );
}

/// Invariant 2: `pii-proto` (default features) must not reach any forbidden
/// crate transitively, and its non-optional direct deps must be the allowlist.
fn check_proto_purity(metadata: &Value, violations: &mut Vec<String>) {
    let Some(pkg_id) = find_package_id(metadata, "pii-proto") else {
        violations.push(String::from("package 'pii-proto' not found in metadata"));
        return;
    };
    check_direct_deps(
        metadata,
        pkg_id,
        PROTO_ALLOWED_DIRECT,
        "pii-proto",
        violations,
    );
    check_transitive_deps(metadata, pkg_id, PROTO_FORBIDDEN, "pii-proto", violations);
}

/// Invariant 3: every client crate must not reach `pii-graph` transitively.
fn check_clients_no_graph(metadata: &Value, violations: &mut Vec<String>) {
    for client in CLIENT_CRATES {
        let Some(pkg_id) = find_package_id(metadata, client) else {
            violations.push(format!("package '{client}' not found in metadata"));
            continue;
        };
        check_transitive_deps(metadata, pkg_id, CLIENT_FORBIDDEN, client, violations);
    }
}

/// Invariant 4: exactly one version of each of the four crates.
fn check_single_versions(metadata: &Value, violations: &mut Vec<String>) {
    let Some(packages) = metadata.get("packages").and_then(Value::as_array) else {
        return;
    };
    for name in SINGLE_VERSION {
        let versions: Vec<&str> = packages
            .iter()
            .filter_map(Value::as_object)
            .filter(|p| p.get("name").and_then(Value::as_str) == Some(name))
            .filter_map(|p| p.get("version").and_then(Value::as_str))
            .collect();
        if versions.len() > 1 {
            violations.push(format!(
                "single-version invariant: '{name}' has {} versions: {}",
                versions.len(),
                versions.join(", ")
            ));
        }
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

fn check_direct_deps(
    metadata: &Value,
    pkg_id: &str,
    allowlist: &[&str],
    crate_name: &str,
    violations: &mut Vec<String>,
) {
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
        let optional = dep
            .get("optional")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if kind == Some("dev") || optional {
            continue;
        }
        if !allowlist.contains(&name) {
            violations.push(format!(
                "{crate_name}: direct dependency '{name}' is not in the allowlist"
            ));
        }
    }
}

fn check_transitive_deps(
    metadata: &Value,
    pkg_id: &str,
    forbidden: &[&str],
    crate_name: &str,
    violations: &mut Vec<String>,
) {
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
                && forbidden.contains(&dep_name)
            {
                violations.push(format!(
                    "{crate_name}: transitive dependency '{dep_name}' is forbidden"
                ));
            }
            if !visited.contains(&dep_pkg) {
                queue.push(dep_pkg);
            }
        }
    }
}

/// Shells out to `cargo metadata` and runs every invariant.
///
/// # Errors
/// Returns the violation list, or an error line if metadata cannot be
/// produced.
// reason: xtask is a dev-tooling crate that must shell out to cargo; the
// Command ban targets the agent runtime (Ctx::proc), not build tooling.
#[allow(clippy::disallowed_methods)]
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

    check_all(&metadata)
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::needless_pass_by_value)] // test fixture builders take owned Values
    #![allow(clippy::indexing_slicing)] // test fixtures index into synthetic metadata

    use super::check_all;
    use serde_json::{Value, json};

    /// Builds a metadata doc with the given packages and resolve nodes.
    pub(super) fn metadata(packages: Value, nodes: Value) -> Value {
        json!({ "packages": packages, "resolve": { "nodes": nodes } })
    }

    pub(super) fn pkg(id: &str, name: &str, deps: Value) -> Value {
        json!({ "id": id, "name": name, "version": "1.0.0", "dependencies": deps })
    }

    pub(crate) fn node(id: &str, deps: Value) -> Value {
        json!({ "id": id, "deps": deps })
    }

    pub(crate) fn dep(pkg: &str) -> Value {
        json!({ "pkg": pkg })
    }

    /// A clean graph: pii-abi pure, pii-proto pure, pii-tui no pii-graph,
    /// single versions.
    ///
    /// Appends a package (and its node) to the fixture; used by invariant-5.
    pub(crate) fn push_package(m: Value, pkg_v: Value, deps_v: Value) -> Value {
        let id = pkg_v["id"].as_str().unwrap_or("x").to_owned();
        let mut m = m;
        m["packages"].as_array_mut().expect("packages").push(pkg_v);
        m["resolve"]["nodes"]
            .as_array_mut()
            .expect("nodes")
            .push(node(&id, deps_v));
        m
    }

    pub(crate) fn clean_metadata() -> Value {
        let packages = json!([
            pkg(
                "abi",
                "pii-abi",
                json!([
                    {"name":"serde","kind":null},{"name":"serde_json","kind":null},
                    {"name":"thiserror","kind":null},{"name":"schemars","kind":null}
                ])
            ),
            pkg(
                "proto",
                "pii-proto",
                json!([
                    {"name":"serde","kind":null},{"name":"serde_json","kind":null}
                ])
            ),
            pkg(
                "tui",
                "pii-tui",
                json!([
                    {"name":"pii-proto","kind":null},{"name":"ratatui","kind":null}
                ])
            ),
            pkg(
                "web",
                "pii-web",
                json!([
                    {"name":"pii-proto","kind":null},{"name":"axum","kind":null}
                ])
            ),
            pkg(
                "serde",
                "serde",
                json!([{"name":"serde_derive","kind":null}])
            ),
            pkg("serde_json", "serde_json", json!([])),
            pkg("thiserror", "thiserror", json!([])),
            pkg("schemars", "schemars", json!([])),
            pkg("ratatui", "ratatui", json!([])),
            pkg("axum", "axum", json!([])),
            pkg("redb", "redb", json!([])),
        ]);
        let nodes = json!([
            node("abi", json!([dep("serde"), dep("schemars")])),
            node("proto", json!([dep("serde"), dep("serde_json")])),
            node("tui", json!([dep("proto"), dep("ratatui")])),
            node("web", json!([dep("proto"), dep("axum")])),
            node("serde", json!([])),
            node("serde_json", json!([])),
            node("thiserror", json!([])),
            node("schemars", json!([])),
            node("ratatui", json!([])),
            node("axum", json!([])),
            node("redb", json!([])),
        ]);
        metadata(packages, nodes)
    }

    #[test]
    fn deps_check_accepts_clean_graph() {
        assert!(check_all(&clean_metadata()).is_ok());
    }

    #[test]
    fn deps_check_rejects_tokio_in_pii_abi() {
        let mut m = clean_metadata();
        m["packages"][0]["dependencies"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"tokio","kind":null}));
        let errs = check_all(&m).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("tokio")));
    }

    #[test]
    fn deps_check_rejects_tui_graph_edge() {
        // pii-tui -> pii-graph (invariant 3, the load-bearing gate).
        let mut m = clean_metadata();
        m["packages"][2]["dependencies"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"pii-graph","kind":null}));
        m["resolve"]["nodes"][2]["deps"]
            .as_array_mut()
            .unwrap()
            .push(dep("graph"));
        m["packages"]
            .as_array_mut()
            .unwrap()
            .push(pkg("graph", "pii-graph", json!([])));
        m["resolve"]["nodes"]
            .as_array_mut()
            .unwrap()
            .push(node("graph", json!([])));
        let errs = check_all(&m).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("pii-tui") && e.contains("pii-graph"))
        );
    }

    #[test]
    fn deps_check_rejects_proto_core_edge() {
        // pii-proto -> pii-core (invariant 2).
        let mut m = clean_metadata();
        m["packages"][1]["dependencies"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"pii-core","kind":null}));
        m["resolve"]["nodes"][1]["deps"]
            .as_array_mut()
            .unwrap()
            .push(dep("core"));
        m["packages"]
            .as_array_mut()
            .unwrap()
            .push(pkg("core", "pii-core", json!([])));
        m["resolve"]["nodes"]
            .as_array_mut()
            .unwrap()
            .push(node("core", json!([])));
        let errs = check_all(&m).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("pii-proto") && e.contains("pii-core"))
        );
    }

    #[test]
    fn deps_check_rejects_duplicate_serde_versions() {
        // Two serde versions (invariant 4).
        let mut m = clean_metadata();
        m["packages"]
            .as_array_mut()
            .unwrap()
            .push(pkg("serde2", "serde", json!([])));
        let errs = check_all(&m).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("single-version") && e.contains("serde"))
        );
    }
}
