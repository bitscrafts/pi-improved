//! Where the daemon and the CLI find graph files: the search order, the lookup, and the not-found
//! refusal. `pii_server` re-exports these at its root, so every caller keeps its path.

use std::path::PathBuf;

use pii_abi::PiiError;

/// Where the daemon loads graphs from.
///
/// `<runtime_dir>/graphs` when it exists, else the `graphs` directory beside
/// the store (where `deploy/install-pii.sh` puts them), else the runtime path.
#[must_use]
pub fn resolve_graph_dir(runtime_dir: &std::path::Path, store_path: &std::path::Path) -> PathBuf {
    let runtime = runtime_dir.join("graphs");
    if runtime.is_dir() {
        return runtime;
    }
    match store_path.parent().map(|p| p.join("graphs")) {
        Some(state) if state.is_dir() => state,
        _ => runtime,
    }
}

/// The directories a graph name is looked up in, in order: `primary`, the `graphs` dir beside the
/// store, then the installer's state dir under `$HOME`. A dir listed twice is searched once.
///
/// This is the one search: the daemon's `run start` and the CLI's `graph` verbs both read it.
#[must_use]
pub fn graph_search_dirs(primary: &std::path::Path, store_path: &std::path::Path) -> Vec<PathBuf> {
    let home = pii_core::env_var("HOME").map(|h| PathBuf::from(h).join(".local/state/pii/graphs"));
    let beside = store_path.parent().map(|p| p.join("graphs"));
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in std::iter::once(primary.to_path_buf())
        .chain(beside)
        .chain(home)
    {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// The graph file a name resolves to, or `None` when the name is ambiguous.
///
/// Spec 384 (derivation 047's first enforcement layer): a user graph may call a system graph, never the
/// reverse — and the search path is what decides that. First-match silently defeats it: a user file named
/// like a system graph would win by being found first. So a name present in MORE THAN ONE search dir is
/// refused rather than resolved, and the caller reports which dirs disagree.
#[must_use]
pub fn find_graph(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    let hits: Vec<PathBuf> = dirs
        .iter()
        .map(|d| d.join(format!("{name}.toml")))
        .filter(|p| p.is_file())
        .collect();
    match hits.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    }
}

/// The refusal for a name no search dir holds: every dir searched, and how many graphs it holds.
#[must_use]
pub fn graph_not_found(name: &str, dirs: &[PathBuf]) -> PiiError {
    let searched: Vec<String> = dirs
        .iter()
        .map(|d| format!("{} ({})", d.display(), toml_count(d)))
        .collect();
    PiiError::NotFound(format!("graph '{name}' — searched {}", searched.join(" ")))
}

/// The `.toml` files a dir holds, not counting dotfiles; a dir that cannot be read holds none.
fn toml_count(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |entries| {
        entries
            .filter_map(Result::ok)
            .filter(|e| {
                let file = e.file_name();
                let file = file.to_string_lossy();
                !file.starts_with('.') && file.ends_with(".toml")
            })
            .count()
    })
}
