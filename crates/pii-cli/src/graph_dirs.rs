//! The graph directories and fragments dir a file-based graph command (`check`, `show`, `run`) loads a host with
//! (spec 260 R5). One place builds them, so no command builds its own list: a host's `call` nodes resolve in its own
//! directory first, then in the installed graph dirs the daemon searches.

// reason: bin crate: pub(crate) is the reachable form.
#![allow(clippy::redundant_pub_crate)]

use std::path::{Path, PathBuf};

/// The directories a host file's `call` nodes are looked up in: the file's own directory first, then the installed
/// graph dirs that exist, the same search the daemon makes (`pii_server::graph_search_dirs`) over `store`.
pub(crate) fn dirs_for(file: &Path, store: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = vec![parent_of(file)];
    if let Some(store) = store {
        let primary = store
            .parent()
            .map_or_else(|| PathBuf::from("."), |p| p.join("graphs"));
        dirs.extend(
            pii_server::graph_search_dirs(&primary, store)
                .into_iter()
                .filter(|d| d.is_dir()),
        );
    }
    dirs
}

/// The fragments dir a host file is woven from: `<file's directory>/fragments` (spec 212).
pub(crate) fn fragments_for(file: &Path) -> PathBuf {
    parent_of(file).join("fragments")
}

/// The store the installed graphs are searched beside when no `--db` is given: the default runtime's.
pub(crate) fn default_store() -> Option<PathBuf> {
    pii_uds::runtime_dir("default").map(|r| r.join("store.redb"))
}

fn parent_of(file: &Path) -> PathBuf {
    file.parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}
