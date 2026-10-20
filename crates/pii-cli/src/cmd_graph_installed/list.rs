//! `pii graph list`: one row per installed graph, and how each copy stands against the repo.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pii_graph::NodeRegistry;
use pii_graph::fragment::sha256_hex;
use serde_json::{Value, json};

use super::{
    Drift, Opts, Row, Suspended, check, error, fragments_dir, graph_files, installed_dir,
    installed_dirs, is_hash, registry, repo_dir, store_path, suspended_runs, trigger,
};

/// `pii graph list`: one row per installed graph (every repo graph under `--all`).
pub(super) fn run(o: &Opts) -> ExitCode {
    let installed = match installed_dirs(o) {
        Ok(d) => d,
        Err(code) => return code,
    };
    let primary = match installed_dir(o) {
        Ok(d) => d,
        Err(code) => return code,
    };
    let repo = repo_dir(o);
    let reg = registry();
    let mut names = BTreeSet::new();
    for dir in &installed {
        match graph_files(dir) {
            Ok(n) => names.extend(n),
            Err(e) => return error(e),
        }
    }
    if o.all {
        names.extend(graph_files(&repo).unwrap_or_default());
    }
    let armed = suspended_runs(&store_path(o, &primary));
    if let Err(e) = &armed {
        eprintln!("armed runs unknown: {e}");
    }
    // A run whose graph could not be named may belong to any row, so every count says so.
    let unnamed = armed
        .as_ref()
        .is_ok_and(|runs| runs.iter().any(|r| is_hash(&r.graph)));
    let rows: Vec<Row> = names
        .iter()
        .filter_map(|name| row_of(name, &installed, &repo, &reg, &armed))
        .collect();
    if o.json {
        print_json(&rows);
    } else {
        print_text(&rows, unnamed);
    }
    ExitCode::SUCCESS
}

/// The row for one name, from whichever of its two copies exists; `None` when neither does.
fn row_of(
    name: &str,
    installed: &[PathBuf],
    repo: &Path,
    reg: &NodeRegistry,
    armed: &Result<Vec<Suspended>, String>,
) -> Option<Row> {
    let inst_path = pii_server::find_graph(installed, name);
    let inst = inst_path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok());
    let inst_dir = inst_path.as_deref().and_then(Path::parent).unwrap_or(repo);
    let rep = std::fs::read_to_string(repo.join(format!("{name}.toml"))).ok();
    let (shown, drift) = match (&inst, &rep) {
        (Some(i), Some(r)) if sha256_hex(i.as_bytes()) == sha256_hex(r.as_bytes()) => {
            (i, Drift::Same)
        }
        (Some(i), Some(_)) => (i, Drift::Changed),
        (Some(i), None) => (i, Drift::InstalledOnly),
        (None, Some(r)) => (r, Drift::RepoOnly),
        (None, None) => return None,
    };
    // A repo-only graph is not running anywhere, so it has no armed runs to count.
    let armed = match (armed, drift) {
        (Ok(runs), Drift::Same | Drift::Changed | Drift::InstalledOnly) => {
            Some(runs.iter().filter(|r| r.graph == name).count())
        }
        _ => None,
    };
    Some(Row {
        name: name.to_owned(),
        sha: sha256_hex(shown.as_bytes()),
        checked: check(shown, &fragments_dir(inst_dir, repo), reg),
        armed,
        drift,
    })
}

fn print_text(rows: &[Row], unnamed: bool) {
    for r in rows {
        let (nodes, trig) = r.checked.as_ref().map_or_else(
            |_| (String::from("unloadable"), String::from("-")),
            |c| (c.spec.nodes.len().to_string(), trigger(&c.spec)),
        );
        let armed = match (r.armed, r.drift) {
            (_, Drift::RepoOnly) => String::from("-"),
            (Some(n), _) if unnamed => format!("{n}+unnamed"),
            (Some(n), _) => n.to_string(),
            (None, _) => String::from("?"),
        };
        println!(
            "{} · {} · nodes {nodes} · {trig} · armed {armed} · {}",
            r.name,
            r.sha.get(..12).unwrap_or(r.sha.as_str()),
            r.drift.label(),
        );
    }
}

fn print_json(rows: &[Row]) {
    let out: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "name": r.name,
                "sha256": r.sha,
                "nodes": r.checked.as_ref().ok().map(|c| c.spec.nodes.len()),
                "trigger": r.checked.as_ref().ok().map(|c| trigger(&c.spec)),
                "error": r.checked.as_ref().err(),
                "armed": r.armed,
                "drift": r.drift.label(),
            })
        })
        .collect();
    println!("{}", Value::Array(out));
}
