//! Spec 126 R10: `pii memory entity|link|neighbours|compact` against the real binary.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

use pii_store::Store;

fn pii(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(args)
        .output()
        .unwrap()
}

fn json(out: &std::process::Output) -> serde_json::Value {
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn add_show_list_link_and_neighbours_round_trip_with_stable_ids() {
    let d = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cli-ent-").unwrap();
    let dir = d.path().to_str().unwrap();
    let add = |kind: &str, name: &str, src: &str| {
        pii(&[
            "memory",
            "entity",
            "add",
            "--kind",
            kind,
            "--name",
            name,
            "--source",
            src,
            "--attr",
            "color=green",
            "--dir",
            dir,
        ])
    };
    let board = json(&add("device", "esp32-board", "actor:operator"));
    let again = json(&add("device", "esp32-board", "run:r7"));
    assert_eq!(board["id"], again["id"], "same description, same id");
    assert_eq!(
        again["provenance"],
        serde_json::json!(["actor:operator", "run:r7"])
    );
    let office = json(&add("room", "office", "file:notes.md"));
    let (b, o) = (
        board["id"].as_str().unwrap(),
        office["id"].as_str().unwrap(),
    );

    let shown = json(&pii(&["memory", "entity", "show", b, "--dir", dir]));
    assert_eq!(shown["name"], "esp32-board");
    let listed = json(&pii(&[
        "memory", "entity", "list", "--kind", "device", "--dir", dir,
    ]));
    assert_eq!(listed.as_array().unwrap().len(), 1);

    let link = json(&pii(&[
        "memory",
        "link",
        b,
        o,
        "--kind",
        "located_in",
        "--source",
        "run:r7",
        "--dir",
        dir,
    ]));
    assert_eq!(link["from"], b);
    let around = json(&pii(&["memory", "neighbours", b, "--dir", dir]));
    assert_eq!(around["entities"][0]["entity"]["id"], o);
    assert_eq!(around["entities"][0]["depth"], 1);
}

#[test]
fn a_refused_write_or_query_exits_2_naming_the_reason() {
    let d = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cli-ent-refuse-").unwrap();
    let dir = d.path().to_str().unwrap();
    let no_prov = pii(&[
        "memory", "entity", "add", "--kind", "device", "--name", "x", "--dir", dir,
    ]);
    assert_eq!(no_prov.status.code(), Some(2), "{no_prov:?}");
    assert!(String::from_utf8_lossy(&no_prov.stderr).contains("no_provenance"));
    let ghost = "e-00000000000000000000000000000000";
    let link = pii(&[
        "memory", "link", ghost, ghost, "--kind", "k", "--source", "run:a", "--dir", dir,
    ]);
    assert_eq!(link.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&link.stderr).contains("unknown_entity"));
    let deep = pii(&["memory", "neighbours", ghost, "--depth", "9", "--dir", dir]);
    assert_eq!(deep.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&deep.stderr).contains("depth_cap"));
}

#[test]
fn compact_refuses_while_a_daemon_holds_the_store_and_runs_when_it_does_not() {
    let d =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cli-ent-compact-").unwrap();
    let dir = d.path().to_str().unwrap();
    json(&pii(&[
        "memory", "entity", "add", "--kind", "t", "--name", "n", "--source", "run:a", "--dir", dir,
    ]));
    let held = Store::open(&d.path().join("store.redb")).unwrap();
    let out = pii(&["memory", "compact", "--dir", dir]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("stop the daemon"));
    drop(held);
    let out = pii(&["memory", "compact", "--dir", dir]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("compacted memory/graph.json"));
}
