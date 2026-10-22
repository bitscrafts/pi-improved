//! A digest of the agent's workdir, taken before and after the agent runs.
//!
//! It lets the journal say whether the run changed anything; without it a no-op agent and one that wrote a file are
//! indistinguishable.
//!
//! The digest covers each file's relative path, size and modification time. Directories that hold build output, VCS
//! data or pii's own scratch are skipped by exact name, because an agent that runs a build must not look like it
//! edited the source. Symlinks are not followed.

use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

/// Directory names never walked: build output, VCS data, dependency caches and pii's own scratch and sessions.
const SKIPPED: &[&str] = &["target", ".git", "_tmp", ".pii-sessions", "node_modules"];

/// Files counted before the walk stops, so a pathological workdir cannot stall the node.
const MAX_FILES: usize = 200_000;

/// The digest of a workdir at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeDigest {
    /// Lowercase hex sha256 over the sorted `path size mtime` lines.
    pub hash: String,
    /// How many files the digest covers.
    pub files: usize,
}

/// Digests `root`. A missing or unreadable root digests as empty, never as an error: the digest is evidence about
/// the run, and failing to take it must not fail the run.
#[must_use]
pub fn digest(root: &Path) -> TreeDigest {
    let mut lines = Vec::new();
    walk(root, root, &mut lines);
    lines.sort();
    let mut hasher = Sha256::new();
    for line in &lines {
        hasher.update(line.as_bytes());
        hasher.update(b"\n");
    }
    TreeDigest {
        hash: format!("{:x}", hasher.finalize()),
        files: lines.len(),
    }
}

fn walk(root: &Path, dir: &Path, lines: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if lines.len() >= MAX_FILES {
            return;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            let name = entry.file_name();
            if !SKIPPED.iter().any(|s| name == *s) {
                walk(root, &path, lines);
            }
        } else if kind.is_file()
            && let Ok(meta) = entry.metadata()
        {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            let rel = path.strip_prefix(root).unwrap_or(&path);
            lines.push(format!("{} {} {mtime}", rel.display(), meta.len()));
        }
    }
}
