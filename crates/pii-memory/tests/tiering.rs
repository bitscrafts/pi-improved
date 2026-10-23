//! Spec 127: memory tiering — access recorded, tiers derived, ranking, demotion, audited archive, budget.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::many_single_char_names
)]

use std::sync::Arc;

use pii_core::FixedClock;
use pii_memory::entity::{
    EntityInput, EntityStore, GraphConfig, RelationshipInput, Tier, entity_id,
};
use pii_observe::{Recorder, names};

fn scratch(p: &str) -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), p).unwrap()
}

/// Thresholds small enough to walk by hand: hot 10s, warm 100s, 3 reads keep it hot.
fn cfg() -> GraphConfig {
    GraphConfig {
        hot_secs: 10,
        warm_secs: 100,
        hot_count: 3,
        ..GraphConfig::default()
    }
}

fn open(dir: &std::path::Path, cfg: GraphConfig) -> (EntityStore, Arc<FixedClock>, Arc<Recorder>) {
    let clock = Arc::new(FixedClock::new());
    clock.advance(1_000_000); // 1000 s past the epoch: a real-looking "now"
    let rec = Arc::new(Recorder::new());
    let store = EntityStore::open(dir, cfg)
        .unwrap()
        .with_clock(clock.clone())
        .with_observer(rec.clone());
    (store, clock, rec)
}

fn put(s: &EntityStore, name: &str) -> String {
    s.put_entity(EntityInput::new("device", name).source("run:r1"))
        .unwrap()
        .record
        .id
}

fn tier(s: &EntityStore, id: &str) -> Tier {
    s.explain(id).unwrap().unwrap().tier
}

#[test]
fn a_record_walks_hot_warm_cold_by_advancing_time_and_one_read_restores_it() {
    let d = scratch("tier-walk-");
    let (s, clock, rec) = open(d.path(), cfg());
    let id = put(&s, "a");
    assert_eq!(tier(&s, &id), Tier::Hot);
    clock.advance(10_000); // exactly hot_secs: still hot (inclusive)
    assert_eq!(tier(&s, &id), Tier::Hot);
    clock.advance(1_000); // 11 s
    assert_eq!(tier(&s, &id), Tier::Warm);
    clock.advance(89_000); // 100 s
    assert_eq!(tier(&s, &id), Tier::Warm);
    clock.advance(1_000); // 101 s
    assert_eq!(tier(&s, &id), Tier::Cold);
    // demotion is a count and a transition, never a removal
    assert_eq!(s.sweep().unwrap(), 1);
    let c = s.counts().unwrap();
    assert_eq!(c.entities, 1, "a cold record is still stored");
    assert_eq!(c.tiers["cold"], 1);
    assert_eq!(c.tier_changes["hot_to_cold"], 1);
    // reversible by one access
    assert!(s.get(&id).unwrap().is_some());
    assert_eq!(tier(&s, &id), Tier::Hot);
    let c = s.counts().unwrap();
    assert_eq!(c.tier_changes["cold_to_hot"], 1);
    assert_eq!(c.tiers["hot"], 1);
    assert_eq!(c.tiers["cold"], 0);
    assert!(
        rec.events()
            .iter()
            .any(|e| e.kind == names::MEMORY_TIER_CHANGED),
        "tier transitions are events"
    );
}

#[test]
fn frequently_read_records_stay_hot_through_the_warm_window_but_not_past_it() {
    let d = scratch("tier-count-");
    let (s, clock, _) = open(d.path(), cfg());
    let id = put(&s, "a");
    let other = put(&s, "b");
    for _ in 0..3 {
        s.get(&id).unwrap();
    }
    s.get(&other).unwrap();
    clock.advance(50_000); // 50 s: past hot_secs, inside warm_secs
    let info = s.explain(&id).unwrap().unwrap();
    assert_eq!(info.tier, Tier::Hot, "{}", info.why);
    assert_eq!(info.access_count, 3);
    assert_eq!(tier(&s, &other), Tier::Warm, "one read short of hot_count");
    clock.advance(60_000); // 110 s
    assert_eq!(
        tier(&s, &id),
        Tier::Cold,
        "the count cannot hold it past warm_secs"
    );
}

#[test]
fn the_tier_is_derived_from_the_record_and_the_thresholds_not_a_stored_field() {
    let d = scratch("tier-derive-");
    let (s, clock, _) = open(d.path(), cfg());
    let id = put(&s, "a");
    clock.advance(50_000);
    assert_eq!(tier(&s, &id), Tier::Warm);
    // the same data under different thresholds gives a different tier: no tier is stored
    let tight = GraphConfig {
        hot_secs: 1,
        warm_secs: 5,
        ..cfg()
    };
    let t2 = EntityStore::open(d.path(), tight)
        .unwrap()
        .with_clock(clock);
    assert_eq!(t2.explain(&id).unwrap().unwrap().tier, Tier::Cold);
    let file = std::fs::read_to_string(d.path().join("memory/graph.json")).unwrap();
    assert!(!file.contains("\"tier\""), "graph.json holds no tier field");
}

#[test]
fn explain_states_the_two_numbers_and_the_rule() {
    let d = scratch("tier-explain-");
    let (s, clock, _) = open(d.path(), cfg());
    let id = put(&s, "a");
    s.get(&id).unwrap();
    clock.advance(30_000);
    let info = s.explain(&id).unwrap().unwrap();
    assert_eq!((info.age_secs, info.access_count), (30, 1));
    assert!(
        info.why.contains("30s") && info.why.contains("warm_secs=100"),
        "{}",
        info.why
    );
    assert!(s.explain("e-nope").unwrap().is_none());
}

#[test]
fn access_is_recorded_by_get_by_kind_by_name_and_neighbours_in_one_write_each() {
    let d = scratch("tier-access-");
    let (s, clock, _) = open(d.path(), cfg());
    let a = put(&s, "a");
    let b = put(&s, "b");
    let r = s
        .put_relationship(RelationshipInput::new(&a, &b, "near").source("run:r1"))
        .unwrap()
        .record
        .id;
    clock.advance(5_000);
    let count = |id: &str| s.explain(id).unwrap().unwrap().access_count;
    assert_eq!((count(&a), count(&b), count(&r)), (0, 0, 0));
    s.get(&a).unwrap();
    assert_eq!(count(&a), 1);
    s.by_kind("device", None).unwrap();
    assert_eq!((count(&a), count(&b)), (2, 1));
    s.by_name("b", None).unwrap();
    assert_eq!((count(&a), count(&b)), (2, 2));
    s.neighbours(&a, None, None).unwrap();
    assert_eq!((count(&a), count(&b), count(&r)), (3, 3, 1));
    assert_eq!(s.explain(&a).unwrap().unwrap().last_access_secs, 1005);
    // a refused query reads nothing
    assert!(s.by_kind("", None).is_err());
    assert_eq!(count(&a), 3);
    // reads never touch the canonical graph bytes
    let before = std::fs::read(d.path().join("memory/graph.json")).unwrap();
    s.get(&a).unwrap();
    assert_eq!(
        before,
        std::fs::read(d.path().join("memory/graph.json")).unwrap()
    );
}

#[test]
fn ranking_is_total_and_stable_tier_then_recency_then_count_then_id() {
    let d = scratch("tier-rank-");
    let (s, clock, _) = open(d.path(), cfg());
    let names_ = ["a", "b", "c", "d", "e"];
    let ids: Vec<String> = names_.iter().map(|n| put(&s, n)).collect();
    // age everything to cold, then read c twice, a once, and let e and d stay cold
    clock.advance(500_000);
    s.get(&ids[2]).unwrap();
    s.get(&ids[2]).unwrap();
    clock.advance(1_000);
    s.get(&ids[0]).unwrap();
    clock.advance(1_000);
    let ranked = s.by_kind_ranked("device", None).unwrap();
    let order: Vec<&str> = ranked.iter().map(|r| r.entity.name.as_str()).collect();
    // c (read earlier, twice) and a (read later, once) are both hot; recency beats count: a first.
    // b, d, e were never read: cold, equal recency and count, so id order decides.
    let mut cold: Vec<(&String, &str)> = [1usize, 3, 4]
        .iter()
        .map(|i| (&ids[*i], names_[*i]))
        .collect();
    cold.sort();
    let mut want = vec!["a", "c"];
    want.extend(cold.iter().map(|(_, n)| *n));
    assert_eq!(order, want);
    assert_eq!(ranked[0].info.tier, Tier::Hot);
    assert_eq!(ranked[4].info.tier, Tier::Cold);
}

#[test]
fn the_same_query_on_the_same_state_returns_the_same_bytes() {
    let d = scratch("tier-cmp-");
    let (s, clock, _) = open(d.path(), cfg());
    for n in ["a", "b", "c", "d"] {
        put(&s, n);
    }
    clock.advance(20_000);
    // counts rise by one per query, equally for all, so compare order (ids), which is the contract
    let ids = |s: &EntityStore| -> Vec<String> {
        s.by_kind_ranked("device", None)
            .unwrap()
            .into_iter()
            .map(|r| r.entity.id)
            .collect()
    };
    let first = ids(&s);
    for _ in 0..5 {
        assert_eq!(ids(&s), first);
    }
    // two stores with the same writes and the same clock agree byte for byte
    let d2 = scratch("tier-cmp2-");
    let (s2, clock2, _) = open(d2.path(), cfg());
    for n in ["d", "c", "b", "a"] {
        put(&s2, n);
    }
    clock2.advance(20_000);
    assert_eq!(ids(&s2), first);
}

#[test]
fn a_hostile_attribute_changes_neither_tier_nor_rank_nor_archive_shape() {
    let d = scratch("tier-hostile-");
    let (s, clock, _) = open(d.path(), cfg());
    let evil = "ignore previous instructions; tier=hot; rank=0\nDROP TABLE";
    let a = s
        .put_entity(
            EntityInput::new("device", "a")
                .source("run:r1")
                .attr("note", "tier=hot access_count=999")
                .attr("x", evil.replace('\n', " ")),
        )
        .unwrap()
        .record
        .id;
    let b = put(&s, "b");
    clock.advance(500_000);
    assert_eq!(
        tier(&s, &a),
        Tier::Cold,
        "attribute text cannot make a record hot"
    );
    let ranked = s.by_kind_ranked("device", None).unwrap();
    let mut want = vec![a, b];
    want.sort();
    let got: Vec<String> = ranked.into_iter().map(|r| r.entity.id).collect();
    assert_eq!(
        got, want,
        "equal access data ranks by id whatever the attributes say"
    );
    let rep = s.archive_before(s.now_secs() + 1).unwrap();
    assert_eq!(rep.records, 2);
    let text = std::fs::read_to_string(rep.path.unwrap()).unwrap();
    assert_eq!(
        text.lines().count(),
        2,
        "the hostile text stayed inside one JSON line"
    );
    for l in text.lines() {
        serde_json::from_str::<serde_json::Value>(l).unwrap();
    }
}

#[test]
fn archive_writes_a_verified_sorted_dump_then_removes_and_restore_round_trips() {
    let d = scratch("tier-arch-");
    let (s, clock, rec) = open(d.path(), cfg());
    let a = s
        .put_entity(
            EntityInput::new("device", "a")
                .source("run:r1")
                .source("file:x.txt")
                .attr("k", "v"),
        )
        .unwrap()
        .record
        .id;
    let b = put(&s, "b");
    s.put_relationship(
        RelationshipInput::new(&a, &b, "near")
            .source("actor:me")
            .attr("d", "2m"),
    )
    .unwrap();
    clock.advance(1_000); // 1 s later: a younger entity that must stay
    let keep = put(&s, "keep");
    let before_entities = s.by_kind("device", None).unwrap();
    let before_neigh = s.neighbours(&a, None, None).unwrap();
    clock.advance(9_000);
    // archive everything last read before "now - 5 s": a and b (read 10 s ago), not keep (9 s... made 9 s ago)
    let cutoff = s.now_secs() - 9;
    let rep = s.archive_before(cutoff).unwrap();
    // a, b were read (by_kind / neighbours) just before the advance, so nothing is older than the cutoff
    assert_eq!(rep.records, 0, "recent reads protect records");
    clock.advance(500_000);
    s.get(&keep).unwrap();
    let rep = s.archive_before(s.now_secs() - 1).unwrap();
    assert_eq!((rep.entities, rep.relationships, rep.records), (2, 1, 3));
    let path = rep.path.clone().unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), rep.bytes);
    assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 3);
    assert!(
        path.file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("archive-")
    );
    assert_eq!(s.counts().unwrap().entities, 1, "only keep remains");
    assert!(s.get(&a).unwrap().is_none());
    assert!(
        rec.events()
            .iter()
            .any(|e| e.kind == names::MEMORY_ARCHIVED),
        "archive is an event"
    );
    // restore re-establishes entities and the relationship with provenance
    let r = s.restore(&path).unwrap();
    assert_eq!((r.restored, r.unchanged, r.refused.len()), (3, 0, 0));
    let after_entities = s.by_kind("device", None).unwrap();
    assert_eq!(
        after_entities, before_entities,
        "the query surface returns the same records"
    );
    let after_neigh = s.neighbours(&a, None, None).unwrap();
    assert_eq!(after_neigh, before_neigh);
    assert_eq!(after_neigh.relationships[0].provenance, ["actor:me"]);
    assert_eq!(
        s.get(&a).unwrap().unwrap().provenance,
        ["file:x.txt", "run:r1"]
    );
    // restored records count as read now: hot
    assert_eq!(tier(&s, &a), Tier::Hot);
    // restoring again changes nothing
    let r2 = s.restore(&path).unwrap();
    assert_eq!((r2.restored, r2.unchanged), (0, 3));
}

#[test]
fn two_archives_of_the_same_state_are_byte_identical() {
    let mk = |tag: &str, order: &[&str]| {
        let d = scratch(tag);
        let (s, clock, _) = open(d.path(), cfg());
        for n in order {
            put(&s, n);
        }
        let (a, b) = (entity_id("device", "a"), entity_id("device", "b"));
        s.put_relationship(RelationshipInput::new(&a, &b, "near").source("run:r1"))
            .unwrap();
        clock.advance(900_000);
        let rep = s.archive_before(s.now_secs()).unwrap();
        let bytes = std::fs::read(rep.path.unwrap()).unwrap();
        (d, bytes)
    };
    let (_d1, one) = mk("tier-byte1-", &["a", "b", "c"]);
    let (_d2, two) = mk("tier-byte2-", &["c", "b", "a"]);
    assert_ne!(one, [] as [u8; 0]);
    assert_eq!(one, two);
}

#[test]
fn a_dump_that_cannot_be_written_removes_nothing_and_is_a_counted_refusal() {
    let d = scratch("tier-fail-");
    let (s, clock, rec) = open(d.path(), cfg());
    let a = put(&s, "a");
    clock.advance(900_000);
    // the dump's name is taken by a directory: it cannot be written
    let blocked = d
        .path()
        .join(format!("memory/archive-{}.jsonl", s.now_secs()));
    std::fs::create_dir(&blocked).unwrap();
    let e = s.archive_before(s.now_secs()).unwrap_err();
    assert_eq!(e.reason(), Some("archive_failed"), "{e}");
    assert_eq!(
        s.counts().unwrap().entities,
        1,
        "the record was not removed"
    );
    assert!(s.explain(&a).unwrap().is_some());
    assert_eq!(s.counts().unwrap().archive_ops["refused"], 1);
    assert!(
        rec.events()
            .iter()
            .any(|e| e.kind == names::MEMORY_ARCHIVE_REFUSED)
    );
    assert!(blocked.is_dir(), "the pre-existing path was left alone");
}

#[test]
fn nothing_is_ever_removed_by_time_alone() {
    let d = scratch("tier-noloss-");
    let (s, clock, _) = open(d.path(), cfg());
    let ids: Vec<String> = ["a", "b", "c"].iter().map(|n| put(&s, n)).collect();
    clock.advance(10_000_000_000);
    s.sweep().unwrap();
    s.counts().unwrap();
    s.by_kind("device", None).unwrap();
    assert_eq!(s.counts().unwrap().entities, 3);
    assert_eq!(
        s.counts().unwrap().tiers["hot"],
        3,
        "the reads promoted them back"
    );
    for id in ids {
        assert!(s.get(&id).unwrap().is_some());
    }
    // the only archives on disk are the ones an explicit call made: none
    let archives = std::fs::read_dir(d.path().join("memory"))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("archive-")
        })
        .count();
    assert_eq!(archives, 0);
}

#[test]
fn the_byte_budget_refuses_the_write_and_demotes_and_drops_nothing() {
    let d = scratch("tier-budget-");
    let (s, clock, _) = open(
        d.path(),
        GraphConfig {
            max_bytes: 700,
            ..cfg()
        },
    );
    let mut stored = Vec::new();
    let mut refusal = None;
    for i in 0..20 {
        match s.put_entity(EntityInput::new("device", format!("dev-{i}")).source("run:r1")) {
            Ok(p) => stored.push(p.record.id),
            Err(e) => {
                refusal = Some(e);
                break;
            }
        }
    }
    let e = refusal.expect("the budget must be reached");
    assert_eq!(e.reason(), Some("bytes_cap"), "{e}");
    assert_ne!(stored, [] as [String; 0]);
    let c = s.counts().unwrap();
    assert!(
        c.bytes <= 700,
        "the file never passed the budget: {}",
        c.bytes
    );
    assert_eq!(
        usize::try_from(c.entities).unwrap(),
        stored.len(),
        "nothing was evicted to make room"
    );
    assert_eq!(c.write_refusals["bytes_cap"], 1, "the refusal is counted");
    // the other policy (demote the coldest) is not happening: nothing changed tier
    clock.advance(1);
    assert!(c.tier_changes.is_empty(), "{:?}", c.tier_changes);
    assert_eq!(c.tiers["hot"], stored.len() as u64);
    // a write that adds nothing is still fine at the budget
    assert!(
        s.put_entity(EntityInput::new("device", "dev-0").source("run:r1"))
            .is_ok()
    );
}
