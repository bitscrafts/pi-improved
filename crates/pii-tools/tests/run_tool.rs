//! Spec 399 R1/R2: the `run` builtin executes an argv exactly, without `/bin/sh`, and refuses a shell (or an empty argv)
//! as the program, whether the argv was written in the graph or arrived as run input.
//!
//! The tool is reached through `ToolSet::builtin`, the way a `tool` node reaches it, so these tests compile against the
//! tree as it is and fail at the lookup until `run` is registered.

#![allow(
    non_snake_case,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;

use pii_abi::PiiError;
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_tools::ToolSet;
use serde_json::{Value, json};

/// The programs this context admits. `shfmt` is here so the prefix control is a real exec attempt, not a denial.
const ADMITTED: &[&str] = &[
    "echo",
    "printf",
    "touch",
    "sh",
    "/bin/bash",
    "env",
    "/usr/bin/env",
    "shfmt",
];

fn workspace() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let canon = std::fs::canonicalize(dir.path()).unwrap();
    (dir, canon)
}

/// A context whose only capability is `PROC`, admitting [`ADMITTED`].
fn proc_ctx() -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::PROC;
    caps.proc_argv0 = ArgvScope::new(ADMITTED.iter().map(|p| (*p).to_owned()).collect());
    Ctx::root(caps, Budget::unlimited())
}

fn call(ws: &std::path::Path, args: &Value) -> Result<Value, PiiError> {
    let tools = ToolSet::builtin(ws.to_path_buf());
    let tool = tools.get("run").expect("the `run` builtin is registered");
    tool.call(&proc_ctx(), args)
}

#[test]
fn run_executes_an_argv_without_a_shell() {
    let (_dir, ws) = workspace();
    // A shell would split `a;b` into two commands; an argv passes it as one argument to echo.
    let res = call(&ws, &json!({"argv": ["echo", "a;b"]})).expect("echo runs");
    assert_eq!(res["exit_code"], json!(0));
    assert_eq!(res["stdout"].as_str().unwrap().trim_end(), "a;b");
}

#[test]
#[allow(non_snake_case)] // reason: the spec names its controls with the CONTROL_ prefix
fn CONTROL_run_passes_arguments_with_spaces_whole() {
    let (_dir, ws) = workspace();
    let res = call(&ws, &json!({"argv": ["printf", "%s", "x y"]})).expect("printf runs");
    assert_eq!(res["stdout"].as_str().unwrap(), "x y");
}

#[test]
fn a_shell_argv0_is_refused_even_when_it_arrives_as_data() {
    let (_dir, ws) = workspace();
    // Each argv would create a marker file if it reached a shell, so the marker is the proof that nothing ran.
    let marker = ws.join("ran");
    let attempts: [(Vec<String>, &str); 4] = [
        (
            vec![
                "sh".into(),
                "-c".into(),
                format!("touch {}", marker.display()),
            ],
            "sh",
        ),
        (
            vec![
                "/bin/bash".into(),
                "-c".into(),
                format!("touch {}", marker.display()),
            ],
            "/bin/bash",
        ),
        (
            vec![
                "env".into(),
                "sh".into(),
                "-c".into(),
                format!("touch {}", marker.display()),
            ],
            "env",
        ),
        (vec!["/usr/bin/env".into(), "dash".into()], "/usr/bin/env"),
    ];
    for (argv, program) in attempts {
        let err = call(&ws, &json!({ "argv": argv }))
            .expect_err(&format!("{program} as argv0 must be refused"));
        assert!(
            matches!(err, PiiError::Denied(_)),
            "{program}: must be Denied, got {err:?}"
        );
        assert!(
            err.to_string().contains(program),
            "the refusal names the program {program}: {err}"
        );
        assert!(!marker.exists(), "{program} ran: the marker was created");
    }
}

#[test]
fn a_denied_command_in_an_argv_is_refused_by_the_deny_list() {
    let (_dir, ws) = workspace();
    // The bash deny list is the one list: an argv is joined with spaces and matched the same way. `sudo` is not installed on
    // the machine and is not admitted either, so the only way to the message "deny list" is the list itself.
    for argv in [json!(["sudo", "ls"]), json!(["dd", "of=/dev/null"])] {
        let err =
            call(&ws, &json!({ "argv": argv })).expect_err("a denied command must be refused");
        assert!(matches!(err, PiiError::Denied(_)), "got {err:?}");
        assert!(
            err.to_string().contains("deny list"),
            "names the list: {err}"
        );
    }
}

#[test]
#[allow(non_snake_case)] // reason: the spec names its controls with the CONTROL_ prefix
fn CONTROL_an_argument_that_names_a_denied_word_is_not_denied() {
    let (_dir, ws) = workspace();
    // The bash list matches a SUBSTRING of a shell string. On an argv that would deny `grep reboot docs`; an entry applies only
    // when the PROGRAM is the entry's first word, so a program that merely talks about one still runs.
    let res = call(&ws, &json!({"argv": ["echo", "reboot", "sudo", "mkfs"]})).expect("echo runs");
    assert_eq!(
        res["stdout"].as_str().unwrap().trim_end(),
        "reboot sudo mkfs"
    );
}

#[test]
#[allow(non_snake_case)] // reason: the spec names its controls with the CONTROL_ prefix
fn CONTROL_a_near_miss_of_a_deny_entry_still_runs() {
    let (_dir, ws) = workspace();
    let res = call(&ws, &json!({"argv": ["echo", "sue"]})).expect("echo sue is not on the list");
    assert_eq!(res["exit_code"], json!(0));
}

#[test]
fn an_empty_argv_is_refused() {
    let (_dir, ws) = workspace();
    let err = call(&ws, &json!({"argv": []})).expect_err("an empty argv must be refused");
    assert!(matches!(err, PiiError::Invalid(_)), "got {err:?}");
}

#[test]
#[allow(non_snake_case)] // reason: the spec names its controls with the CONTROL_ prefix
fn CONTROL_a_program_named_like_a_shell_prefix_is_not_a_shell() {
    let (_dir, ws) = workspace();
    // The shell list matches the whole final path component, so `shfmt` is an ordinary program. It may be absent
    // from the machine; what matters is that the refusal is not the shell refusal.
    let res = call(&ws, &json!({"argv": ["shfmt", "--version"]}));
    assert!(
        !matches!(res, Err(PiiError::Denied(_))),
        "shfmt is not a shell: {res:?}"
    );
}
