//! Spec 113: the SOP format, its lint, and the deterministic compiler.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_graph::sop;

const GOOD: &str = r#"
id = "rel"
title = "Release"
caps = ["proc", "fs_read"]

[[step]]
id = "a"
kind = "act"
prompt = "do a"

[[step]]
id = "b"
kind = "check"
requires = ["a"]
argv = ["true"]

[[step]]
id = "c"
kind = "approve"
prompt = "ship?"
deadline_ms = 86400000

[[step]]
id = "d"
kind = "notify"
message = "done"
"#;

fn graph_of(src: &str) -> toml::Table {
    let sop = sop::load(src).unwrap();
    sop::compile(&sop).parse().unwrap()
}

fn node_ids(g: &toml::Table) -> Vec<String> {
    g["node"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap().to_owned())
        .collect()
}

fn lint_errors(src: &str) -> Vec<String> {
    sop::load(src).expect_err("expected the SOP to be refused")
}

/// R2: the same SOP compiles to the same bytes, twice and across a reparse.
#[test]
fn compile_is_byte_deterministic() {
    let a = sop::compile(&sop::load(GOOD).unwrap());
    let b = sop::compile(&sop::load(GOOD).unwrap());
    assert_eq!(a, b);
    assert_ne!(a, "");
    // Caps are normalised, so their written order cannot change the bytes.
    let swapped = GOOD.replace(r#"["proc", "fs_read"]"#, r#"["fs_read", "proc"]"#);
    assert_eq!(a, sop::compile(&sop::load(&swapped).unwrap()));
}

/// R8: the compiler is pinned — every step appears, in declaration order,
/// chained `end -> begin`. Dropping or reordering a step fails here.
#[test]
fn compile_keeps_every_step_in_order() {
    let g = graph_of(GOOD);
    let ids = node_ids(&g);
    let begins: Vec<&str> = ids
        .iter()
        .filter(|i| i.rsplit('.').next() == Some("begin"))
        .map(String::as_str)
        .collect();
    assert_eq!(begins, ["a.begin", "b.begin", "c.begin", "d.begin"]);
    assert_eq!(g["entry"].as_str().unwrap(), "a.begin");
    let edges: Vec<(String, String)> = g["edge"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["from"].as_str().unwrap().to_owned(),
                e["to"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    for (prev, next) in [("a", "b"), ("b", "c"), ("c", "d")] {
        assert!(
            edges.contains(&(format!("{prev}.end"), format!("{next}.begin"))),
            "{prev} must chain into {next}: {edges:?}"
        );
    }
}

/// R4: an `approve` step is a `gate` node named after the step, and the
/// prompt a human reads carries the SOP and step ids.
#[test]
fn approve_compiles_to_a_gate_named_after_the_step() {
    let g = graph_of(GOOD);
    let gate = g["node"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"].as_str() == Some("c"))
        .unwrap();
    assert_eq!(gate["kind"].as_str().unwrap(), "gate");
    let prompt = gate["prompt"].as_str().unwrap();
    assert!(prompt.contains("rel") && prompt.contains("'c'") && prompt.contains("ship?"));
}

/// Authority: the graph's caps are exactly the SOP's, nothing wider.
#[test]
fn compiled_caps_are_the_sops_caps() {
    let g = graph_of(GOOD);
    let caps: Vec<&str> = g["caps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert_eq!(caps, ["fs_read", "proc"]);
}

#[test]
fn lint_refuses_a_missing_step_id() {
    let errs = lint_errors(&GOOD.replace("id = \"b\"\n", ""));
    assert!(
        errs.iter().any(|e| e.contains("step #2 has no id")),
        "{errs:?}"
    );
}

#[test]
fn lint_refuses_a_duplicate_step_id() {
    let errs = lint_errors(&GOOD.replace("id = \"c\"", "id = \"a\""));
    assert!(
        errs.iter().any(|e| e.contains("duplicate step id 'a'")),
        "{errs:?}"
    );
}

#[test]
fn lint_refuses_a_dependency_cycle() {
    // `a` requires `b`, which is declared after `a`: a -> b (order) and b -> a.
    let errs = lint_errors(&GOOD.replace(
        "kind = \"act\"\nprompt = \"do a\"",
        "kind = \"act\"\nprompt = \"do a\"\nrequires = [\"b\"]",
    ));
    assert!(
        errs.iter().any(|e| e.contains("dependency cycle")),
        "{errs:?}"
    );
    let errs = lint_errors(&GOOD.replace(
        "kind = \"act\"\nprompt = \"do a\"",
        "kind = \"act\"\nprompt = \"do a\"\nrequires = [\"a\"]",
    ));
    assert!(
        errs.iter()
            .any(|e| e.contains("dependency cycle") && e.contains("a -> a")),
        "{errs:?}"
    );
}

#[test]
fn lint_refuses_unknown_requires_kind_and_missing_fields() {
    let errs = lint_errors(&GOOD.replace("requires = [\"a\"]", "requires = [\"zzz\"]"));
    assert!(
        errs.iter()
            .any(|e| e.contains("requires unknown step 'zzz'")),
        "{errs:?}"
    );
    let errs = lint_errors(&GOOD.replace("kind = \"check\"", "kind = \"frobnicate\""));
    assert!(
        errs.iter().any(|e| e.contains("unknown kind 'frobnicate'")),
        "{errs:?}"
    );
    let errs = lint_errors(&GOOD.replace("argv = [\"true\"]\n", ""));
    assert!(
        errs.iter()
            .any(|e| e.contains("step 'b'") && e.contains("'argv'")),
        "{errs:?}"
    );
    let errs =
        lint_errors(&GOOD.replace("caps = [\"proc\", \"fs_read\"]", "caps = [\"teleport\"]"));
    assert!(
        errs.iter()
            .any(|e| e.contains("unknown capability 'teleport'")),
        "{errs:?}"
    );
    let errs = lint_errors("id = \"x\"\n");
    assert!(
        errs.iter().any(|e| e.contains("at least one step")),
        "{errs:?}"
    );
}

/// A string the graph machinery would evaluate as an expression is refused,
/// so an SOP author cannot smuggle `$.` into a literal field.
#[test]
fn lint_refuses_expression_looking_literals() {
    let errs = lint_errors(&GOOD.replace("argv = [\"true\"]", "argv = [\"echo\", \"$.input.x\"]"));
    assert!(
        errs.iter().any(|e| e.contains("must not start with '$.'")),
        "{errs:?}"
    );
}

/// Spec 397: a suspension with no end is a decision nobody made, so an `approve` step must say how long it may
/// wait, and the gate it compiles to carries exactly that.
#[test]
fn approve_deadline_reaches_the_gate() {
    let g = graph_of(GOOD);
    let gate = g["node"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"].as_str() == Some("c"))
        .unwrap();
    assert_eq!(gate["deadline_ms"].as_integer(), Some(86_400_000));
}

fn with_approve(extra: &str) -> String {
    format!(
        "id = \"r\"\ntitle = \"R\"\n\n[[step]]\nid = \"a\"\nkind = \"approve\"\nprompt = \"ok?\"\n{extra}"
    )
}

#[test]
fn an_approve_without_a_deadline_is_refused_naming_the_step() {
    let errs = sop::load(&with_approve("")).unwrap_err();
    assert!(
        errs.iter()
            .any(|e| e.contains("step 'a'") && e.contains("deadline_ms")),
        "{errs:?}"
    );
}

#[test]
fn an_approve_with_a_zero_deadline_is_refused() {
    let errs = sop::load(&with_approve("deadline_ms = 0\n")).unwrap_err();
    assert!(errs.iter().any(|e| e.contains("deadline_ms")), "{errs:?}");
}

#[test]
fn control_an_approve_with_a_positive_deadline_loads() {
    assert!(sop::load(&with_approve("deadline_ms = 1\n")).is_ok());
}

#[test]
fn control_only_an_approve_takes_a_deadline() {
    let src = "id = \"r\"\ntitle = \"R\"\n\n[[step]]\nid = \"a\"\nkind = \"check\"\nargv = [\"true\"]\ndeadline_ms = 5\n";
    let errs = sop::load(src).unwrap_err();
    assert!(
        errs.iter()
            .any(|e| e.contains("step 'a'") && e.contains("deadline_ms")),
        "{errs:?}"
    );
}
