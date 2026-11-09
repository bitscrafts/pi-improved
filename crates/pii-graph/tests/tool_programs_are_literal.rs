//! Spec 400 R1/R2: a `tool = "run"` node's `programs` is a literal array of program names, refused at load, naming the node.
//! A `$.` expression, a string, an empty array or a blank entry is refused, and so is a declared shell (`sh`, `/bin/bash`,
//! `env`), whatever its path. Authority comes from the committed graph file, so run input can never supply the program.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_graph::toml_load::load_toml;

/// A graph with one `tool = "run"` node `build` whose `programs` is the given TOML value.
fn run_with_programs(programs: &str) -> String {
    format!(
        "name = \"g\"\nentry = \"build\"\n\n[[node]]\nid = \"build\"\nkind = \"tool\"\ntool = \"run\"\n\
         programs = {programs}\nargs = {{ argv = [\"cargo\", \"test\"] }}\n"
    )
}

#[test]
fn a_programs_expression_is_refused_at_load() {
    let err = load_toml(&run_with_programs(r#""$.input.programs""#))
        .expect_err("a `$.` expression in `programs` must be refused at load")
        .to_string();
    assert!(err.contains("build"), "names the node: {err}");
    assert!(err.contains("programs"), "names what it refused: {err}");
}

#[test]
#[allow(non_snake_case)] // reason: the spec names its controls with the CONTROL_ prefix
fn CONTROL_a_programs_array_of_literals_loads() {
    load_toml(&run_with_programs(r#"["cargo"]"#)).unwrap();
}

#[test]
fn a_declared_shell_is_refused_at_load() {
    for programs in [r#"["sh"]"#, r#"["/bin/bash"]"#, r#"["env"]"#] {
        let err = load_toml(&run_with_programs(programs))
            .expect_err(programs)
            .to_string();
        assert!(err.contains("build"), "{programs} names the node: {err}");
        assert!(err.contains("shell"), "{programs} names the shell: {err}");
    }
}

#[test]
#[allow(non_snake_case)] // reason: the spec names its controls with the CONTROL_ prefix
fn CONTROL_a_declared_program_named_like_a_shell_prefix_loads() {
    load_toml(&run_with_programs(r#"["shfmt"]"#)).unwrap();
}

#[test]
fn an_empty_or_blank_programs_is_refused() {
    for programs in [r"[]", r#"[""]"#, r#"["  "]"#] {
        let err = load_toml(&run_with_programs(programs))
            .expect_err(programs)
            .to_string();
        assert!(err.contains("build"), "{programs} names the node: {err}");
    }
}
