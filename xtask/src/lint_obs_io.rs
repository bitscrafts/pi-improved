//! Spec 109 R8: a crate-wide observability exemption is INVALID when any module
//! of the crate imports `tokio`, `std::net`, `std::io` or `std::process` — such
//! a module owns failures an operator must be able to see. A module that must
//! stay dormant says so in a module-scoped registry entry (`crate:module  # reason`).
//!
//! The rule is by import: a crate reaching a socket only through another crate
//! is not caught; it closes the common case, not every case.

// reason: see lint_files.rs — binary-internal helpers; clippy
// `redundant_pub_crate` conflicts with rustc `unreachable_pub`.
#![allow(clippy::redundant_pub_crate)]

/// Path fragments whose appearance in code marks a module that owns I/O failures.
const IO_MARKERS: &[&str] = &[
    "tokio::",
    "use tokio",
    "std::net",
    "std::io",
    "std::process",
];

/// Line numbers (1-based) of code lines naming an I/O import; comments are skipped.
pub(crate) fn io_imports(src: &str) -> Vec<usize> {
    src.lines()
        .enumerate()
        .filter(|(_, l)| {
            let code = l.split("//").next().unwrap_or("");
            IO_MARKERS.iter().any(|m| code.contains(m))
        })
        .map(|(i, _)| i + 1)
        .collect()
}

/// The module a source file `rel` (relative to the crate's `src/`) defines:
/// `client.rs` → `client`, `a/b.rs` → `a/b`, `a/mod.rs` → `a`.
pub(crate) fn module_of(rel: &str) -> String {
    let stem = rel.strip_suffix(".rs").unwrap_or(rel);
    stem.strip_suffix("/mod").unwrap_or(stem).to_owned()
}

/// The finding for an exempt crate whose module `module` imports I/O at `line`.
pub(crate) fn io_finding(krate: &str, module: &str, line: usize) -> String {
    format!(
        "crates/{krate}/src/{module}.rs:{line}: crate {krate} is exempt but {krate}/src/{module}.rs imports I/O (tokio / std::net / std::io / std::process) — a module with I/O cannot be covered by a crate-wide exemption; emit through pii-observe, or add a module-scoped entry `{krate}:{module}  # reason` for a dormant module"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_imports_are_found_and_comments_skipped() {
        let s = "//! tokio::net docs\nuse std::path::Path;\nuse tokio::net::UnixStream;\n// std::io\nfn f(_: std::io::Error) {}\n";
        assert_eq!(io_imports(s), vec![3, 5]);
    }

    #[test]
    fn a_types_only_source_has_no_io_imports() {
        assert_eq!(
            io_imports("pub struct A;\npub const B: u8 = 1;\nuse std::fmt;\n"),
            [] as [usize; 0]
        );
    }

    #[test]
    fn module_names_follow_the_file_layout() {
        assert_eq!(module_of("client.rs"), "client");
        assert_eq!(module_of("a/b.rs"), "a/b");
        assert_eq!(module_of("a/mod.rs"), "a");
    }
}
