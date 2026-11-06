//! Spec 389 exit criterion: chapter 05 of the architecture book names `fragment/call_output.rs`, the file that
//! turns a call's exit ports into the caller's outputs. The section "How a call passes data" must point at it,
//! so a reader can check the claim against the source.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;

#[test]
fn the_composition_chapter_names_the_call_output_file() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chapter = root.join("_docs/manuals/src/arch-dev/src/chapters/05-composition.tex");
    let text = std::fs::read_to_string(&chapter).expect("chapter 05 is readable");
    assert!(
        text.contains("fragment/call_output.rs"),
        "{} does not name fragment/call_output.rs",
        chapter.display()
    );
}
