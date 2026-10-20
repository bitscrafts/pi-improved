//! `pii project init <dir>` (spec 225 R1, R2, R5, R6): writes the generated project scaffold and nothing else.
//!
//! Every file is rendered from a template compiled into this binary. Nothing is copied from `$HOME`, and a target
//! that already holds a credential is refused before anything is written.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::cmd_ctl::next_str;

/// The contract text, written byte for byte into `.pi/APPEND_SYSTEM.md` and `.claude/CLAUDE.md` (spec 225 R5).
const CONTRACT: &str = include_str!("../templates/project/contract.md");
const PII_SETTINGS: &str = include_str!("../templates/project/settings.toml");
const POLICY: &str = include_str!("../templates/project/policy.toml");
const PI_SETTINGS: &str = include_str!("../templates/project/pi-settings.json");
const CLAUDE_SETTINGS: &str = include_str!("../templates/project/claude-settings.json");

/// The generated files and their content. `.gitignore` is merged separately.
const GENERATED: [(&str, &str); 6] = [
    (".pii/settings.toml", PII_SETTINGS),
    (".pii/policy.toml", POLICY),
    (".pi/settings.json", PI_SETTINGS),
    (".pi/APPEND_SYSTEM.md", CONTRACT),
    (".claude/settings.json", CLAUDE_SETTINGS),
    (".claude/CLAUDE.md", CONTRACT),
];

/// The directories created empty: the store's graphs and skills, and the mountpoints a run's skills are bound over,
/// which podman would otherwise create inside the project (spec 225 R3).
const EMPTY_DIRS: [&str; 4] = [".pii/graphs", ".pii/skills", ".pi/skills", ".claude/skills"];

/// The `.gitignore` lines the scaffold owns (spec 225 R1).
const GITIGNORE_LINES: [&str; 5] = [
    ".pii/runtime/",
    ".pi/sessions/",
    ".claude/local/",
    "*.env",
    "*_env",
];

/// Directories the credential scan never enters: VCS metadata and build output.
const SKIPPED_DIRS: [&str; 2] = [".git", "target"];

/// `pii project {init} ...`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_project(args: &mut std::env::ArgsOs) -> ExitCode {
    match next_str(args).as_deref() {
        Some("init") => next_str(args).map_or_else(usage, |dir| init(Path::new(&dir))),
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!("usage: pii project init <dir>");
    ExitCode::from(2)
}

/// Writes the scaffold under `dir`, or refuses (exit 1) and writes nothing.
fn init(dir: &Path) -> ExitCode {
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("pii project init: cannot create {}: {e}", dir.display());
        return ExitCode::from(1);
    }
    match find_credential(dir) {
        Ok(None) => {}
        Ok(Some((rel, why))) => {
            eprintln!(
                "pii project init: refusing: {} {why}; remove it or choose another directory",
                rel.display()
            );
            return ExitCode::from(1);
        }
        Err(e) => {
            eprintln!("pii project init: cannot scan {}: {e}", dir.display());
            return ExitCode::from(1);
        }
    }
    if let Some((rel, _)) = GENERATED.iter().find(|(rel, _)| dir.join(rel).exists()) {
        eprintln!("pii project init: refusing: {rel} already exists; the project is initialised");
        return ExitCode::from(1);
    }
    match write_scaffold(dir) {
        Ok(()) => {
            println!("project initialised: {}", dir.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("pii project init: {e}");
            ExitCode::from(1)
        }
    }
}

fn write_scaffold(dir: &Path) -> std::io::Result<()> {
    for (rel, content) in GENERATED {
        write_file(&dir.join(rel), content)?;
    }
    for rel in EMPTY_DIRS {
        std::fs::create_dir_all(dir.join(rel))?;
    }
    merge_gitignore(&dir.join(".gitignore"))
}

fn write_file(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)
}

/// Creates `.gitignore` with the scaffold's lines, or appends the ones it does not already carry.
fn merge_gitignore(path: &Path) -> std::io::Result<()> {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let missing: Vec<&str> = GITIGNORE_LINES
        .into_iter()
        .filter(|line| !existing.lines().any(|l| l == *line))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let mut text = existing;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    for line in missing {
        text.push_str(line);
        text.push('\n');
    }
    std::fs::write(path, text)
}

/// The first credential under `root`, as its path relative to `root` and the reason it counts. Walks the tree but
/// not `.git` or `target`; a symlink is not followed.
fn find_credential(root: &Path) -> std::io::Result<Option<(PathBuf, &'static str)>> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if file_type.is_dir() {
                if !SKIPPED_DIRS.contains(&name.as_str()) {
                    stack.push(path);
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            if let Some(why) = credential_reason(&name, &path) {
                let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
                return Ok(Some((rel, why)));
            }
        }
    }
    Ok(None)
}

/// Why a file counts as a credential, if it does. Name rules first; then content, for text files only.
fn credential_reason(name: &str, path: &Path) -> Option<&'static str> {
    if name == "auth.json" {
        return Some("is a pi auth file");
    }
    if name.starts_with(".credentials") {
        return Some("is a credentials file");
    }
    let text = std::fs::read_to_string(path).ok()?;
    let lower = name.to_ascii_lowercase();
    if lower.strip_suffix(".env").is_some() && text.lines().any(assigns_a_value) {
        return Some("is an env file with a value");
    }
    if text.lines().any(is_api_key_line) {
        return Some("holds an *_API_KEY= value");
    }
    None
}

/// `KEY=value` with a non-empty value.
fn assigns_a_value(line: &str) -> bool {
    line.split_once('=')
        .is_some_and(|(key, value)| !key.trim().is_empty() && !value.trim().is_empty())
}

/// A line that sets an `*_API_KEY` variable to a non-empty value.
fn is_api_key_line(line: &str) -> bool {
    line.split_once("_API_KEY=")
        .is_some_and(|(_, value)| !value.trim().is_empty())
}
