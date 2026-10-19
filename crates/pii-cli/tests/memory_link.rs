//! Spec 204: `pii memory link <from> <to> --rel <name>` round-trips, and `pii index code` writes
//! both relationship families idempotently.
#![allow(
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
        .env("PII_RUNTIME_DIR", env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap()
}

fn json(out: &std::process::Output) -> serde_json::Value {
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    serde_json::from_slice(&out.stdout).unwrap()
}

fn add(dir: &str, name: &str) -> String {
    let v = json(&pii(&[
        "memory",
        "entity",
        "add",
        "--kind",
        "module",
        "--name",
        name,
        "--source",
        "file:a.rs:1",
        "--dir",
        dir,
    ]));
    v["id"].as_str().unwrap().to_owned()
}

#[test]
fn a_link_made_with_rel_returns_its_id_and_is_listed_by_neighbours() {
    let d = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cli-link-").unwrap();
    let dir = d.path().to_str().unwrap();
    let (a, b) = (add(dir, "m::a"), add(dir, "m::b"));
    let link = json(&pii(&[
        "memory", "link", &a, &b, "--rel", "contains", "--dir", dir,
    ]));
    assert!(link["id"].as_str().is_some_and(|i| !i.is_empty()), "{link}");
    assert_eq!(link["kind"], "contains");
    assert!(!link["provenance"].as_array().unwrap().is_empty());
    let around = json(&pii(&["memory", "neighbours", &a, "--dir", dir]));
    assert_eq!(around["entities"][0]["entity"]["id"], b.as_str());
    assert_eq!(around["relationships"][0]["id"], link["id"]);
}

#[test]
fn a_link_to_an_unknown_entity_is_refused() {
    let d = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cli-link-bad-").unwrap();
    let dir = d.path().to_str().unwrap();
    let a = add(dir, "m::a");
    let ghost = "e-00000000000000000000000000000000";
    let out = pii(&[
        "memory", "link", &a, ghost, "--rel", "contains", "--dir", dir,
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown_entity"));
}

#[test]
fn index_code_writes_contains_and_depends_on_and_is_idempotent() {
    let root =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cli-idx-root-").unwrap();
    let store =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cli-idx-store-").unwrap();
    let r = root.path();
    for (c, deps) in [("alpha", ""), ("beta", "alpha = { path = \"../alpha\" }\n")] {
        std::fs::create_dir_all(r.join("crates").join(c).join("src")).unwrap();
        std::fs::write(
            r.join("crates").join(c).join("Cargo.toml"),
            format!("[package]\nname = \"{c}\"\n[dependencies]\n{deps}"),
        )
        .unwrap();
    }
    std::fs::write(
        r.join("crates/alpha/src/lib.rs"),
        "pub mod inner;\npub fn top() {}\n",
    )
    .unwrap();
    std::fs::write(
        r.join("crates/alpha/src/inner.rs"),
        "pub struct Thing;\nfn helper() {}\n",
    )
    .unwrap();
    std::fs::write(r.join("crates/beta/src/lib.rs"), "pub fn b() {}\n").unwrap();
    let (rs, ds) = (r.to_str().unwrap(), store.path().to_str().unwrap());
    let run = || {
        let o = pii(&["index", "code", "--root", rs, "--dir", ds, "--no-docs"]);
        assert_eq!(o.status.code(), Some(0), "{o:?}");
        String::from_utf8(o.stdout).unwrap()
    };
    let first = run();
    // 2 crates + 3 modules + 4 symbols; 1 depends_on + 1 mod decl + 4 symbol edges.
    assert!(first.starts_with("entities=9 relationships=6 "), "{first}");
    assert_eq!(run(), first, "re-indexing must not change the counts");
    let tiers = json(&pii(&["memory", "tiers", "--dir", ds]));
    assert_eq!(tiers["entities"], 9, "{tiers}");
    assert_eq!(tiers["relationships"], 6, "{tiers}");
}
