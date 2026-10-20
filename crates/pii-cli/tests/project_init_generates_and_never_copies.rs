//! Spec 225 R1, R5, R6: `pii project init <dir>` writes only generated files, copies nothing from `$HOME`, and
//! renders the contract text into `.pi` and `.claude` from one source, so the two agents cannot differ.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its output.
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};

const HOME_MARKER: &str = "HOME-MARKER-never-copied";

/// Every file under `root`, relative to it, with `/` separators, sorted.
fn files_under(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path: PathBuf = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                let rel = path.strip_prefix(root).unwrap();
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

#[test]
fn init_writes_only_generated_files_and_copies_nothing_from_home() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let project = tmp.path().join("project");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::create_dir_all(home.join(".pi/agent")).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(home.join(".claude/settings.json"), HOME_MARKER).unwrap();
    std::fs::write(home.join(".pi/agent/settings.json"), HOME_MARKER).unwrap();

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(["project", "init"])
        .arg(&project)
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "init must succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert_eq!(
        files_under(&project),
        [
            ".claude/CLAUDE.md",
            ".claude/settings.json",
            ".gitignore",
            ".pi/APPEND_SYSTEM.md",
            ".pi/settings.json",
            ".pii/policy.toml",
            ".pii/settings.toml",
        ],
        "exactly the generated files, nothing else"
    );

    let gitignore = std::fs::read_to_string(project.join(".gitignore")).unwrap();
    for line in [
        ".pii/runtime/",
        ".pi/sessions/",
        ".claude/local/",
        "*.env",
        "*_env",
    ] {
        assert!(
            gitignore.lines().any(|l| l == line),
            "gitignore names {line}"
        );
    }

    let pi_contract = std::fs::read(project.join(".pi/APPEND_SYSTEM.md")).unwrap();
    let claude_contract = std::fs::read(project.join(".claude/CLAUDE.md")).unwrap();
    assert_eq!(
        pi_contract, claude_contract,
        "one template, rendered into both agents' files"
    );
    let contract = String::from_utf8(pi_contract).unwrap();
    assert!(
        contract.contains("named criterion"),
        "states the definition of done"
    );
    assert!(
        contract.contains("read-only"),
        "states the tests are read-only"
    );

    let claude_settings: serde_json::Value =
        serde_json::from_slice(&std::fs::read(project.join(".claude/settings.json")).unwrap())
            .unwrap();
    assert!(
        claude_settings["hooks"]["PreToolUse"].is_array(),
        "PreToolUse deny hook"
    );
    assert!(
        claude_settings["hooks"]["PostToolUse"].is_array(),
        "PostToolUse formatter hook"
    );

    for file in files_under(&project) {
        let text = std::fs::read_to_string(project.join(&file)).unwrap();
        assert!(
            !text.contains(HOME_MARKER),
            "{file} carries nothing from $HOME"
        );
    }
}
