//! Spec 213 R4: `pii graph edit` lints first, refuses an armed run unless `--force`, and moves the
//! previous file aside rather than deleting it. Every refusal leaves the installed bytes untouched.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};

use pii_abi::{RunEvent, RunHeader, RunId, RunStatus};
use pii_store::Store;

fn repo_graphs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("graphs")
}

/// The installed dir holds a copy of `discord-digest`; the state dir beside it holds the store.
/// Returns (root, installed dir, repo dir).
fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cli-graph-edit-").unwrap();
    let installed = root.path().join("state").join("graphs");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&installed).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::copy(
        repo_graphs().join("discord-digest.toml"),
        installed.join("discord-digest.toml"),
    )
    .unwrap();
    std::fs::copy(
        repo_graphs().join("discord-digest.toml"),
        repo.join("discord-digest.toml"),
    )
    .unwrap();
    (root, installed, repo)
}

fn pii_in(cwd: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .current_dir(cwd)
        .env("PII_RUNTIME_DIR", cwd.join("no-daemon-here"))
        .args(args)
        .output()
        .unwrap()
}

fn edit_args<'a>(name: &'a str, file: &'a str, installed: &'a str, repo: &'a str) -> Vec<&'a str> {
    vec![
        "graph", "edit", name, "--file", file, "--dir", installed, "--repo", repo,
    ]
}

fn prev_files(installed: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(installed)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("discord-digest.toml.prev-"))
        })
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

#[test]
fn edit_refuses_invalid_toml() {
    let (root, installed, repo) = fixture();
    let target = installed.join("discord-digest.toml");
    let before = std::fs::read(&target).unwrap();
    let broken = root.path().join("broken.toml");
    std::fs::write(&broken, "name = \"discord-digest\nentry = [[\n").unwrap();

    let out = pii_in(
        root.path(),
        &edit_args(
            "discord-digest",
            broken.to_str().unwrap(),
            installed.to_str().unwrap(),
            repo.to_str().unwrap(),
        ),
    );
    assert_ne!(out.status.code(), Some(0), "{out:?}");
    // A usage error would also exit non-zero: the refusal must come from the lint.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("usage:"),
        "refused by the linter: {stderr}"
    );
    assert_eq!(
        std::fs::read(&target).unwrap(),
        before,
        "installed unchanged"
    );
    assert!(prev_files(&installed).is_empty(), "nothing moved aside");
}

#[test]
fn edit_moves_previous_aside() {
    let (root, installed, repo) = fixture();
    let target = installed.join("discord-digest.toml");
    let before = std::fs::read(&target).unwrap();
    let edited = [before.as_slice(), b"# reviewed by a human\n"].concat();
    let new_file = root.path().join("new.toml");
    std::fs::write(&new_file, &edited).unwrap();

    let out = pii_in(
        root.path(),
        &edit_args(
            "discord-digest",
            new_file.to_str().unwrap(),
            installed.to_str().unwrap(),
            repo.to_str().unwrap(),
        ),
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let before_sha = sha256_hex(&before);
    let after_sha = sha256_hex(&edited);
    assert!(
        stdout.contains(&format!("before-sha {before_sha}")),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!("after-sha {after_sha}")),
        "{stdout}"
    );

    let prev = prev_files(&installed);
    assert_eq!(prev.len(), 1, "one .prev-<epoch> file: {prev:?}");
    assert_eq!(
        std::fs::read(&prev[0]).unwrap(),
        before,
        "previous kept intact"
    );
    assert_eq!(
        std::fs::read(&target).unwrap(),
        edited,
        "installed is the new file"
    );
}

#[test]
fn edit_refuses_armed_run() {
    let (root, installed, repo) = fixture();
    let target = installed.join("discord-digest.toml");
    let before = std::fs::read(&target).unwrap();
    let edited = [before.as_slice(), b"# edit under a live run\n"].concat();
    let new_file = root.path().join("new.toml");
    std::fs::write(&new_file, &edited).unwrap();

    // A suspended run of `discord-digest` in the store beside the installed dir.
    let store = Store::open(&root.path().join("state").join("store.redb")).unwrap();
    let mut header = RunHeader {
        run: RunId::from("run-armed-213"),
        graph: String::from("discord-digest"),
        status: RunStatus::Suspended,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 0,
        created_ms: 0,
    };
    store
        .append_event(
            &mut header,
            &RunEvent::Started {
                graph: String::from("discord-digest"),
                input_hash: String::new(),
            },
        )
        .unwrap();
    drop(store);

    let args = edit_args(
        "discord-digest",
        new_file.to_str().unwrap(),
        installed.to_str().unwrap(),
        repo.to_str().unwrap(),
    );
    let refused = pii_in(root.path(), &args);
    assert_ne!(refused.status.code(), Some(0), "{refused:?}");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&refused.stdout),
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(text.contains("run-armed-213"), "names the run id: {text}");
    assert_eq!(
        std::fs::read(&target).unwrap(),
        before,
        "installed unchanged"
    );
    assert!(prev_files(&installed).is_empty(), "nothing moved aside");

    let mut forced = args.clone();
    forced.push("--force");
    let out = pii_in(root.path(), &forced);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        std::fs::read(&target).unwrap(),
        edited,
        "--force writes the edit"
    );
    assert_eq!(
        prev_files(&installed).len(),
        1,
        "and keeps the previous file"
    );
}
