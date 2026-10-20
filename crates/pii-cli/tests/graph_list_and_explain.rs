//! Spec 213 R1-R5: `pii graph list` and `pii graph explain` against the real binary. The installed
//! directory and the repo directory are fixtures under a scratch root; the daemon socket is absent
//! (`PII_RUNTIME_DIR` points at an empty dir), so the armed-run lookup falls back to the store.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};

/// The repo's own graphs, as the test sees them: `<crate>/../../graphs`.
fn repo_graphs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("graphs")
}

/// Repo copy holds every top-level graph; the installed copy holds the three the installer names.
/// Returns (root, installed dir, repo dir).
fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cli-graph-list-").unwrap();
    let installed = root.path().join("state").join("graphs");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&installed).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    for entry in std::fs::read_dir(repo_graphs()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "toml") {
            std::fs::copy(&path, repo.join(path.file_name().unwrap())).unwrap();
        }
    }
    for name in ["discord-digest", "discord-triage", "intervention-report"] {
        std::fs::copy(
            repo.join(format!("{name}.toml")),
            installed.join(format!("{name}.toml")),
        )
        .unwrap();
    }
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

#[test]
fn list_reports_installed_only() {
    let (root, installed, repo) = fixture();
    let args = [
        "graph",
        "list",
        "--dir",
        installed.to_str().unwrap(),
        "--repo",
        repo.to_str().unwrap(),
    ];
    let out = pii_in(root.path(), &args);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let rows: Vec<&str> = stdout.lines().collect();
    assert_eq!(rows.len(), 3, "only the installed three: {stdout}");
    for name in ["discord-digest", "discord-triage", "intervention-report"] {
        assert!(rows.iter().any(|r| r.starts_with(name)), "{stdout}");
    }
    assert!(
        !stdout.contains("queue"),
        "repo-only rows are hidden: {stdout}"
    );

    let mut all = args.to_vec();
    all.push("--all");
    let out = pii_in(root.path(), &all);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("queue"),
        "--all shows the repo-only graph: {stdout}"
    );
    assert!(stdout.contains("repo-only"), "{stdout}");
}

#[test]
fn list_reports_drift() {
    let (root, installed, repo) = fixture();
    let digest = installed.join("discord-digest.toml");
    let mut edited = std::fs::read_to_string(&digest).unwrap();
    edited.push_str("# hand edit on the installed copy\n");
    std::fs::write(&digest, edited).unwrap();

    let out = pii_in(
        root.path(),
        &[
            "graph",
            "list",
            "--dir",
            installed.to_str().unwrap(),
            "--repo",
            repo.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let digest_row = stdout
        .lines()
        .find(|r| r.starts_with("discord-digest"))
        .unwrap();
    assert!(digest_row.contains("DRIFT"), "{stdout}");
    let triage_row = stdout
        .lines()
        .find(|r| r.starts_with("discord-triage"))
        .unwrap();
    assert!(triage_row.contains("same"), "{stdout}");
}

#[test]
fn list_json_is_an_array_of_rows() {
    let (root, installed, repo) = fixture();
    let out = pii_in(
        root.path(),
        &[
            "graph",
            "list",
            "--json",
            "--dir",
            installed.to_str().unwrap(),
            "--repo",
            repo.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 3, "{v}");
    assert!(rows.iter().all(|r| r.get("drift").is_some()), "{v}");
}

#[test]
fn explain_names_trigger_and_effects() {
    let (root, installed, repo) = fixture();
    let out = pii_in(
        root.path(),
        &[
            "graph",
            "explain",
            "discord-digest",
            "--dir",
            installed.to_str().unwrap(),
            "--repo",
            repo.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("TRIGGER timer 0 12 * * *"), "{stdout}");
    assert!(stdout.contains("  post"), "EFFECTS lists post: {stdout}");
    assert!(
        stdout.contains("  memory"),
        "EFFECTS lists memory: {stdout}"
    );
    assert!(stdout.contains("sha256 "), "{stdout}");
    assert!(stdout.contains("same"), "drift verdict is stated: {stdout}");
}

#[test]
fn explain_states_drift_and_names_the_node() {
    let (root, installed, repo) = fixture();
    let digest = installed.join("discord-digest.toml");
    let edited = std::fs::read_to_string(&digest)
        .unwrap()
        .replace("limit = 50", "limit = 7");
    std::fs::write(&digest, edited).unwrap();

    let out = pii_in(
        root.path(),
        &[
            "graph",
            "explain",
            "discord-digest",
            "--dir",
            installed.to_str().unwrap(),
            "--repo",
            repo.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("DRIFT"), "{stdout}");
    assert!(
        stdout.contains("devices"),
        "the changed node id is named: {stdout}"
    );
}

#[test]
fn show_by_name_prints_the_installed_toml_verbatim() {
    let (root, installed, _repo) = fixture();
    let out = pii_in(
        root.path(),
        &[
            "graph",
            "show",
            "discord-triage",
            "--dir",
            installed.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let on_disk = std::fs::read(installed.join("discord-triage.toml")).unwrap();
    assert_eq!(out.stdout, on_disk);
}
