//! Spec 127: `pii memory show|tiers|archive|restore` and `entity list --rank` against the real binary.
#![allow(
    clippy::print_stdout,
    clippy::disallowed_macros,
    clippy::too_many_lines,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

fn pii(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(args)
        .output()
        .unwrap()
}

fn ok(args: &[&str]) -> String {
    let o = pii(args);
    assert_eq!(o.status.code(), Some(0), "{args:?}: {o:?}");
    String::from_utf8(o.stdout).unwrap()
}

const T: [&str; 6] = ["--hot-secs", "10", "--warm-secs", "100", "--hot-count", "3"];

fn args<'a>(base: &[&'a str], dir: &'a str, now: &'a str) -> Vec<&'a str> {
    let mut v = base.to_vec();
    v.extend(["--dir", dir, "--now-secs", now]);
    v.extend(T);
    v
}

#[test]
fn the_cli_records_access_explains_the_tier_ranks_and_archives_with_a_dump_first() {
    let d = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cli-tier-").unwrap();
    let dir = d.path().to_str().unwrap();
    let add = |name: &str, now: &str| {
        let out = ok(&args(
            &[
                "memory", "entity", "add", "--kind", "device", "--name", name, "--source",
                "actor:op",
            ],
            dir,
            now,
        ));
        serde_json::from_str::<serde_json::Value>(&out).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let a = add("a", "1000");
    let b = add("b", "1000");

    // show at t=1050 explains: 50s old, warm, 0 reads — and is itself a read
    let shown: serde_json::Value =
        serde_json::from_str(&ok(&args(&["memory", "show", &a], dir, "1050"))).unwrap();
    println!("SHOW: {}", shown["tier"]);
    assert_eq!(shown["tier"]["tier"], "warm");
    assert_eq!(shown["tier"]["age_secs"], 50);
    assert_eq!(shown["tier"]["access_count"], 0);
    assert!(
        shown["tier"]["why"]
            .as_str()
            .unwrap()
            .contains("warm_secs=100")
    );
    // the read was recorded by the CLI path: a is now hot with one read, b is still warm
    let again: serde_json::Value =
        serde_json::from_str(&ok(&args(&["memory", "show", &a], dir, "1051"))).unwrap();
    assert_eq!(again["tier"]["tier"], "hot");
    assert_eq!(again["tier"]["access_count"], 1);

    // ranking is deterministic: two identical listings, byte for byte
    let rank = || {
        ok(&args(
            &["memory", "entity", "list", "--kind", "device", "--rank"],
            dir,
            "1060",
        ))
    };
    let ids_of = |s: &str| -> Vec<String> {
        serde_json::from_str::<serde_json::Value>(s)
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["entity"]["id"].as_str().unwrap().to_owned())
            .collect()
    };
    let first = ids_of(&rank());
    assert_eq!(first[0], a, "the one read recently ranks first");
    assert_eq!(first[1], b);
    assert_eq!(ids_of(&rank()), first);

    // tiers report
    let t: serde_json::Value =
        serde_json::from_str(&ok(&args(&["memory", "tiers"], dir, "1061"))).unwrap();
    assert_eq!(t["tiers"]["hot"], 2);

    // archive: everything last read before now-1000s... nothing is that old
    let none = ok(&args(
        &["memory", "archive", "--before", "30d"],
        dir,
        "1062",
    ));
    assert!(none.contains("records=0"), "{none}");
    // far in the future everything is cold; the archive reports counts and the dump exists first
    let out = ok(&args(
        &["memory", "archive", "--before", "1d"],
        dir,
        "200000",
    ));
    println!("ARCHIVE: {out}");
    assert!(out.contains("records=2") && out.contains("bytes="), "{out}");
    let file = out.split("file=").nth(1).unwrap().trim().to_owned();
    assert!(std::path::Path::new(&file).is_file());
    assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 2);
    let gone = pii(&args(&["memory", "show", &a], dir, "200001"));
    assert_ne!(
        gone.status.code(),
        Some(0),
        "archived records are gone from the graph"
    );

    let back = ok(&args(&["memory", "restore", &file], dir, "200002"));
    println!("RESTORE: {back}");
    assert!(
        back.contains("restored=2") && back.contains("refused=0"),
        "{back}"
    );
    let again: serde_json::Value =
        serde_json::from_str(&ok(&args(&["memory", "show", &a], dir, "200003"))).unwrap();
    assert_eq!(again["name"], "a");
    assert_eq!(again["provenance"], serde_json::json!(["actor:op"]));
}

#[test]
fn an_archive_that_cannot_be_written_removes_nothing() {
    let d = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cli-tier-fail-").unwrap();
    let dir = d.path().to_str().unwrap();
    ok(&args(
        &[
            "memory", "entity", "add", "--kind", "device", "--name", "a", "--source", "actor:op",
        ],
        dir,
        "1000",
    ));
    std::fs::create_dir(d.path().join("memory/archive-200000.jsonl")).unwrap();
    let o = pii(&args(
        &["memory", "archive", "--before", "1d"],
        dir,
        "200000",
    ));
    assert_eq!(o.status.code(), Some(2), "{o:?}");
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("archive_failed"),
        "{o:?}"
    );
    let t: serde_json::Value =
        serde_json::from_str(&ok(&args(&["memory", "tiers"], dir, "200000"))).unwrap();
    assert_eq!(t["entities"], 1);
    assert_eq!(t["archive_ops"]["refused"], 1);
}
