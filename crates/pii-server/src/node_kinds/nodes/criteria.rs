//! Named criteria for the `coder` node (spec 215).
//!
//! A criterion is a NAME, resolved to a fixed argv here and run exactly as that argv. Nothing is interpolated and
//! nothing runs through a shell; an unknown name is refused by the caller, never guessed at.

use std::collections::BTreeMap;

/// A criteria table: each name to the argv it runs. The first element of each argv is the program.
pub type Table = BTreeMap<String, Vec<String>>;

/// The production table. Every entry is a program and its arguments, fixed at build time.
const PRODUCTION: &[(&str, &[&str])] = &[
    (
        "cargo:test:pii-server/coder_spawns_a_process",
        &[
            "cargo",
            "test",
            "-p",
            "pii-server",
            "--test",
            "coder_spawns_a_process",
        ],
    ),
    (
        "cargo:test:pii-server/criteria_are_named_not_shell",
        &[
            "cargo",
            "test",
            "-p",
            "pii-server",
            "--test",
            "criteria_are_named_not_shell",
        ],
    ),
    (
        "cargo:test:pii-server/grant_ceiling_refuses",
        &[
            "cargo",
            "test",
            "-p",
            "pii-server",
            "--test",
            "grant_ceiling_refuses",
        ],
    ),
];

/// The production table as an owned [`Table`].
#[must_use]
pub fn production() -> Table {
    PRODUCTION
        .iter()
        .map(|(name, argv)| {
            (
                (*name).to_owned(),
                argv.iter().map(|arg| (*arg).to_owned()).collect(),
            )
        })
        .collect()
}

/// Every program a table can exec: the grant a coder node needs to run its criteria, and nothing more.
#[must_use]
pub fn programs(table: &Table) -> Vec<String> {
    let mut programs: Vec<String> = table
        .values()
        .filter_map(|argv| argv.first().cloned())
        .collect();
    programs.sort();
    programs.dedup();
    programs
}
