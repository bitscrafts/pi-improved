//! Spec 225 R2: `pii project init` refuses a target that already holds a credential, names the path, and writes
//! nothing.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its output.
#![allow(clippy::disallowed_methods)]

/// One planted credential per case: the path relative to the target, and its content.
const CASES: [(&str, &str); 5] = [
    ("auth.json", "{\"token\":\"x\"}"),
    ("config/.credentials.json", "{}"),
    (".env", "OPENAI_API_KEY=sk-test\n"),
    ("tools/run.sh", "export ANTHROPIC_API_KEY=sk-test\n"),
    ("deploy_env", "PROD_API_KEY=sk-test\n"),
];

#[test]
fn init_refuses_a_target_that_holds_a_credential() {
    for (rel, content) in CASES {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("project");
        let planted = project.join(rel);
        std::fs::create_dir_all(planted.parent().unwrap()).unwrap();
        std::fs::write(&planted, content).unwrap();

        let out = std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
            .args(["project", "init"])
            .arg(&project)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);

        assert!(!out.status.success(), "{rel}: init must refuse");
        assert!(
            stderr.contains(rel),
            "{rel}: refusal names the path: {stderr}"
        );
        assert!(
            !project.join(".pii").exists(),
            "{rel}: nothing is generated"
        );
        assert!(!project.join(".pi").exists(), "{rel}: nothing is generated");
        assert!(
            !project.join(".claude").exists(),
            "{rel}: nothing is generated"
        );
    }
}

#[test]
fn an_empty_env_file_is_not_a_credential() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join(".env"), "").unwrap();

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(["project", "init"])
        .arg(&project)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "an empty .env holds no value: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
