//! A `coder` node with an empty or blank brief must be refused before anything spawns. The matrix measured the
//! opposite: `brief = ""` ran pi for 92 ms with no prompt, exited 0, and the node SUCCEEDED because its criterion
//! already passed on HEAD. Controls: a real brief still runs, and a spec-less node is still refused (the existing rule).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_server::node_kinds::nodes::coder::brief_is_usable;

#[test]
fn an_empty_brief_is_not_usable() {
    assert!(!brief_is_usable(""));
}

#[test]
fn a_blank_brief_is_not_usable() {
    assert!(!brief_is_usable("   \n\t  "));
}

#[test]
fn a_real_brief_is_usable() {
    assert!(brief_is_usable("Implement the spec named above."));
}

#[test]
fn a_brief_with_surrounding_whitespace_is_still_usable() {
    assert!(brief_is_usable("  fix it  "));
}
