//! Spec 399 (the SOP producer): a `check` step runs a PROGRAM, never a shell string.
//!
//! The SOP compiler used to turn every `check` into a `tool = "bash"` node with `command` as a shell string. Row 399 closes
//! that surface, so the step takes an `argv` array whose first entry is the program, and the compiled node is a `run` node
//! that declares exactly that program. A `command` string, a shell as argv0 and a `$.` entry are refused at lint, naming
//! the step. The near-miss controls keep each refusal from becoming "every check is refused".

#![allow(
    non_snake_case,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_graph::sop;
use pii_graph::toml_load::load_toml;

const SOP: &str = r#"
id = "rel"
title = "Release"
caps = ["proc"]

[[step]]
id = "b"
kind = "check"
argv = ["grep", "-q", "^version", "Cargo.toml"]
"#;

fn with_check(argv_line: &str) -> String {
    SOP.replace(
        r#"argv = ["grep", "-q", "^version", "Cargo.toml"]"#,
        argv_line,
    )
}

fn errors(src: &str) -> Vec<String> {
    sop::load(src).expect_err("expected the SOP to be refused")
}

fn node<'a>(g: &'a toml::Table, id: &str) -> &'a toml::Value {
    g["node"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("no node {id}"))
}

#[test]
fn a_check_step_compiles_to_a_run_node_declaring_its_program() {
    let text = sop::compile(&sop::load(SOP).unwrap());
    let g: toml::Table = text.parse().unwrap();
    let n = node(&g, "b");
    assert_eq!(n["kind"].as_str(), Some("tool"), "{text}");
    assert_eq!(n["tool"].as_str(), Some("run"), "{text}");
    let programs: Vec<&str> = n["programs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect();
    assert_eq!(
        programs,
        ["grep"],
        "the node declares exactly argv0: {text}"
    );
    let argv: Vec<&str> = n["args"]["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect();
    assert_eq!(argv, ["grep", "-q", "^version", "Cargo.toml"], "{text}");
    assert!(
        !text.contains("\"bash\""),
        "no shell tool in the compiled graph: {text}"
    );
}

#[test]
fn a_check_command_string_is_refused_and_says_what_to_write() {
    let errs = errors(&SOP.replace(
        r#"argv = ["grep", "-q", "^version", "Cargo.toml"]"#,
        r#"command = "test -f Cargo.toml && grep -q version Cargo.toml""#,
    ));
    assert!(
        errs.iter().any(|e| e.contains("step 'b'")
            && e.contains("`command`")
            && e.contains("shell string")
            && e.contains("argv")),
        "{errs:?}"
    );
}

#[test]
fn a_check_argv_whose_program_is_a_shell_is_refused() {
    for line in [
        r#"argv = ["sh", "-c", "true"]"#,
        r#"argv = ["/bin/bash", "-c", "true"]"#,
        r#"argv = ["env", "sh"]"#,
        r#"argv = ["/usr/bin/env", "dash"]"#,
    ] {
        let errs = errors(&with_check(line));
        assert!(
            errs.iter()
                .any(|e| e.contains("step 'b'") && e.contains("shell")),
            "{line}: {errs:?}"
        );
    }
}

#[test]
fn a_check_argv_entry_that_looks_like_an_expression_is_refused() {
    let errs = errors(&with_check(r#"argv = ["echo", "$.input.x"]"#));
    assert!(
        errs.iter()
            .any(|e| e.contains("step 'b'") && e.contains("must not start with '$.'")),
        "{errs:?}"
    );
}

#[test]
fn a_check_without_a_program_is_refused() {
    for line in ["argv = []", "# no argv at all"] {
        let errs = errors(&with_check(line));
        assert!(
            errs.iter()
                .any(|e| e.contains("step 'b'") && e.contains("'argv'")),
            "{line}: {errs:?}"
        );
    }
}

#[test]
fn only_a_check_takes_an_argv() {
    let src = r#"
id = "x"
title = "X"

[[step]]
id = "a"
kind = "act"
prompt = "do it"
argv = ["true"]
"#;
    let errs = errors(src);
    assert!(
        errs.iter()
            .any(|e| e.contains("step 'a'") && e.contains("only a 'check' step takes an 'argv'")),
        "{errs:?}"
    );
}

#[test]
fn CONTROL_a_program_named_like_a_shell_prefix_is_a_program() {
    // The shell list matches the whole final path component, so `shfmt` and `bashful` are ordinary programs.
    for line in [
        r#"argv = ["shfmt", "--version"]"#,
        r#"argv = ["/usr/bin/bashful"]"#,
    ] {
        sop::load(&with_check(line)).unwrap_or_else(|e| panic!("{line} must load: {e:?}"));
    }
}

#[test]
fn CONTROL_the_shipped_example_loads_and_its_graph_passes_the_tool_surface() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/cut-release.sop.toml"),
    )
    .unwrap();
    let text = sop::compile(&sop::load(&src).expect("the shipped example is a valid SOP"));
    load_toml(&text)
        .unwrap_or_else(|e| panic!("the example's compiled graph is refused: {e}\n{text}"));
}
