//! `cargo xtask lint-graph-kinds [--root <dir>]` (spec 393): the fence around the graph vocabulary. Parses the
//! `[[node]]` tables of `graphs/**/*.toml` and refuses a kind outside `graphs/VOCABULARY`, a kind only one graph uses
//! (`specialised`), and a graph whose node kinds and call targets repeat another graph's (`duplicate-of <graph>`).
//!
//! NEVER A GREP. A `kind =` grep also counts edge kinds: that mistake invented a `data` kind and misdirected a whole
//! session (docs/03-reference/graph-vocabulary.md). There is no exception list: if a graph fails, the graph moves.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use toml::Table;

/// What one graph file contributes to the measurement.
#[derive(Debug, PartialEq, Eq)]
pub struct Graph {
    pub name: String,
    pub kinds: Vec<String>,
    pub call_targets: Vec<String>,
}

/// Parses one graph document. Only `[[node]]` tables are read, so an `[[edge]]`'s `kind` can never be counted.
pub fn parse_graph(fallback_name: &str, src: &str) -> Result<Graph, String> {
    let doc: Table = toml::from_str(src).map_err(|e| format!("not TOML: {e}"))?;
    let name = doc
        .get("name")
        .and_then(toml::Value::as_str)
        .unwrap_or(fallback_name);
    let nodes = doc.get("node").and_then(toml::Value::as_array);
    let mut kinds = Vec::new();
    let mut call_targets = Vec::new();
    for node in nodes.into_iter().flatten() {
        let kind = node
            .get("kind")
            .and_then(toml::Value::as_str)
            .ok_or("a [[node]] has no string `kind`")?;
        kinds.push(kind.to_string());
        if kind == "call" {
            let target = node.get("graph").and_then(toml::Value::as_str);
            call_targets.push(target.unwrap_or_default().to_string());
        }
    }
    if kinds.is_empty() {
        return Err("no [[node]] tables".into());
    }
    kinds.sort();
    call_targets.sort();
    Ok(Graph {
        name: name.to_string(),
        kinds,
        call_targets,
    })
}

/// The first column of `graphs/VOCABULARY`: `<name> <capability> <effect class>`, `#` starting a comment.
pub fn parse_vocabulary(src: &str) -> Result<BTreeSet<String>, String> {
    let mut names = BTreeSet::new();
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let columns: Vec<&str> = line.split_whitespace().collect();
        let [name, _capability, _effect] = columns.as_slice() else {
            return Err(format!(
                "VOCABULARY line `{line}` is not `<name> <capability> <effect class>`"
            ));
        };
        names.insert((*name).to_string());
    }
    Ok(names)
}

/// The outcome of one measurement: the counts a pass reports (R6) and every refusal.
#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    pub kinds: usize,
    pub graphs: usize,
    pub specialised: usize,
    pub refusals: Vec<String>,
}

impl Report {
    /// `23 kinds over 32 graphs, 0 specialised`.
    pub fn summary(&self) -> String {
        format!(
            "{} kinds over {} graphs, {} specialised",
            self.kinds, self.graphs, self.specialised
        )
    }
}

/// The names in `graphs/minimal.txt`, the manifest of what an installation ships (spec 384).
pub fn parse_shipped(src: &str) -> BTreeSet<String> {
    src.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Applies the three refusals. `graphs` is in a stable order: of two equal flows, the later one is the duplicate.
/// R1 refuses re-implementing what a SHIPPED graph provides, so only a shipped graph can be the original: samples may
/// share a shape with each other. `shipped` is `None` when a tree has no manifest, and then everything in it counts.
pub fn measure(
    vocabulary: &BTreeSet<String>,
    shipped: Option<&BTreeSet<String>>,
    graphs: &[Graph],
) -> Report {
    let mut users: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for graph in graphs {
        for kind in &graph.kinds {
            users.entry(kind).or_default().insert(graph.name.as_str());
        }
    }
    let mut refusals = Vec::new();
    let mut specialised = 0;
    for (kind, graph_names) in &users {
        if !vocabulary.contains(*kind) {
            let list: Vec<&str> = graph_names.iter().copied().collect();
            refusals.push(format!(
                "kind `{kind}` is outside graphs/VOCABULARY (used by {})",
                list.join(", ")
            ));
        } else if let [only] = graph_names.iter().copied().collect::<Vec<_>>().as_slice() {
            specialised += 1;
            refusals.push(format!(
                "kind `{kind}` is specialised: only graph `{only}` uses it"
            ));
        }
    }
    let mut flows: BTreeMap<(&[String], &[String]), &str> = BTreeMap::new();
    for graph in graphs {
        if shipped.is_none_or(|s| s.contains(&graph.name)) {
            flows
                .entry((graph.kinds.as_slice(), graph.call_targets.as_slice()))
                .or_insert(&graph.name);
        }
    }
    for graph in graphs {
        let flow = (graph.kinds.as_slice(), graph.call_targets.as_slice());
        if let Some(original) = flows.get(&flow).filter(|o| **o != graph.name) {
            refusals.push(format!("graph `{}` duplicate-of {original}", graph.name));
        }
    }
    Report {
        kinds: users.len(),
        graphs: graphs.len(),
        specialised,
        refusals,
    }
}

/// Every `*.toml` under `dir`, sorted, so the measurement and its refusals are stable.
fn toml_files(dir: &Path, found: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry
            .map_err(|e| format!("read {}: {e}", dir.display()))?
            .path();
        if path.is_dir() {
            toml_files(&path, found)?;
        } else if path.extension().is_some_and(|x| x == "toml") {
            found.push(path);
        }
    }
    found.sort();
    Ok(())
}

fn measure_tree(root: &Path) -> Result<Report, Vec<String>> {
    let dir = root.join("graphs");
    let vocabulary_path = dir.join("VOCABULARY");
    let vocabulary = std::fs::read_to_string(&vocabulary_path)
        .map_err(|e| format!("read {}: {e}", vocabulary_path.display()))
        .and_then(|src| parse_vocabulary(&src))
        .map_err(|e| vec![e])?;
    let mut files = Vec::new();
    toml_files(&dir, &mut files).map_err(|e| vec![e])?;
    let mut graphs = Vec::new();
    let mut broken = Vec::new();
    for file in &files {
        let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
        let parsed = std::fs::read_to_string(file)
            .map_err(|e| e.to_string())
            .and_then(|src| parse_graph(stem, &src));
        match parsed {
            Ok(graph) => graphs.push(graph),
            Err(e) => broken.push(format!("{}: {e}", file.display())),
        }
    }
    if !broken.is_empty() {
        return Err(broken);
    }
    if graphs.is_empty() {
        return Err(vec![format!(
            "no graphs under {}: a pass over nothing proves nothing",
            dir.display()
        )]);
    }
    let shipped = std::fs::read_to_string(dir.join("minimal.txt"))
        .ok()
        .map(|src| parse_shipped(&src));
    Ok(measure(&vocabulary, shipped.as_ref(), &graphs))
}

/// Entry point: measures `--root <dir>`, or the repository when no root is named.
pub fn run(args: &[String]) -> Result<(), Vec<String>> {
    let root = match args {
        [] => xtask::workspace_root(),
        [flag, dir] if flag == "--root" => PathBuf::from(dir),
        _ => {
            return Err(vec![
                "usage: cargo xtask lint-graph-kinds [--root <dir>]".to_string(),
            ]);
        }
    };
    let report = measure_tree(&root)?;
    println!("lint-graph-kinds: {}", report.summary());
    if report.refusals.is_empty() {
        Ok(())
    } else {
        Err(report.refusals)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn graph(name: &str) -> Graph {
        parse_graph(
            name,
            &format!("name = \"{name}\"\n[[node]]\nid = \"a\"\nkind = \"transform\"\n"),
        )
        .unwrap()
    }

    fn duplicates(shipped: &[&str], graphs: &[Graph]) -> Vec<String> {
        let vocabulary = parse_vocabulary("transform none pure\n").unwrap();
        let shipped: BTreeSet<String> = shipped.iter().map(ToString::to_string).collect();
        measure(&vocabulary, Some(&shipped), graphs).refusals
    }

    #[test]
    fn samples_may_share_a_shape_but_may_not_copy_a_shipped_flow() {
        assert_eq!(
            duplicates(&[], &[graph("sample-a"), graph("sample-b")]),
            Vec::<String>::new()
        );
        assert_eq!(
            duplicates(&["zz-shipped"], &[graph("sample"), graph("zz-shipped")]),
            vec!["graph `sample` duplicate-of zz-shipped"]
        );
    }

    #[test]
    fn an_edge_kind_is_never_a_node_kind() {
        let src = "name = \"g\"\n[[node]]\nid = \"a\"\nkind = \"transform\"\n\
                   [[edge]]\nfrom = \"a\"\nto = \"a\"\nkind = \"data\"\n";
        assert_eq!(parse_graph("g", src).unwrap().kinds, ["transform"]);
    }
}
