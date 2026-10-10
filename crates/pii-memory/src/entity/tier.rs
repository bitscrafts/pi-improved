//! Tiers (spec 127): what is in active use, what has gone quiet.
//!
//! A tier is **derived**, never stored as the truth: it is a pure function of a record's
//! access data (`last_access_secs`, `access_count`), the clock and the configured thresholds,
//! so it cannot disagree with the data it explains. The access data lives in
//! `<dir>/memory/access.json`, apart from `graph.json`: a read writes one small file (one
//! write per query, not one per attribute) and never disturbs the canonical graph bytes.
//!
//! Nothing here removes a record. A cold record is *demoted* (ranked last, counted); the only
//! removal path is the audited archive in [`super::archive`].

use std::cmp::Reverse;
use std::collections::BTreeMap;

use pii_observe::{Event, Level, names};
use serde::{Deserialize, Serialize};

use super::store::{Counters, EntityStore, GraphConfig, bump};
use super::types::{Entity, GraphError};

/// How active a record is. Ordered hottest first, so ascending order is the ranking order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Read recently, or read often and not long ago.
    Hot,
    /// Read within the warm window.
    Warm,
    /// Not read within the warm window: demoted, still stored, restored to hot by one read.
    Cold,
}

impl Tier {
    /// All tiers, hottest first.
    pub const ALL: [Self; 3] = [Self::Hot, Self::Warm, Self::Cold];

    /// The stable word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hot => "hot",
            Self::Warm => "warm",
            Self::Cold => "cold",
        }
    }
}

/// When a record was last read and how often. Written at creation and by every read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Access {
    /// Epoch seconds of the last read (or of creation).
    pub last_access_secs: u64,
    /// Reads so far (saturating); a freshly created record has zero.
    pub access_count: u64,
    /// The tier last reported, so a transition is counted once. Not an input to the tier.
    #[serde(default)]
    pub observed: Option<Tier>,
}

impl Access {
    pub(super) const fn new(now: u64) -> Self {
        Self {
            last_access_secs: now,
            access_count: 0,
            observed: Some(Tier::Hot),
        }
    }

    /// The tier these numbers give at `now`: hot when read within `hot_secs`, or read
    /// `hot_count` times and within `warm_secs`; warm within `warm_secs`; otherwise cold.
    #[must_use]
    pub fn tier(&self, cfg: &GraphConfig, now: u64) -> Tier {
        let age = now.saturating_sub(self.last_access_secs);
        if age <= cfg.hot_secs || (age <= cfg.warm_secs && self.access_count >= cfg.hot_count) {
            Tier::Hot
        } else if age <= cfg.warm_secs {
            Tier::Warm
        } else {
            Tier::Cold
        }
    }
}

/// The on-disk access map. A record with no entry counts as never read (epoch 0), hence cold.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct AccessFile {
    #[serde(default)]
    pub(super) entries: BTreeMap<String, Access>,
}

const NEVER: Access = Access {
    last_access_secs: 0,
    access_count: 0,
    observed: None,
};

impl AccessFile {
    pub(super) fn get(&self, id: &str) -> Access {
        self.entries.get(id).copied().unwrap_or(NEVER)
    }

    pub(super) fn tier(&self, cfg: &GraphConfig, id: &str, now: u64) -> Tier {
        self.get(id).tier(cfg, now)
    }
}

/// Why a record is in its tier: the two numbers and the rule they met.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TierInfo {
    /// The tier now.
    pub tier: Tier,
    /// Epoch seconds of the last read.
    pub last_access_secs: u64,
    /// Seconds since then.
    pub age_secs: u64,
    /// Reads so far.
    pub access_count: u64,
    /// One sentence naming the thresholds that decided it.
    pub why: String,
}

pub(super) fn explain(cfg: &GraphConfig, a: &Access, now: u64) -> TierInfo {
    let tier = a.tier(cfg, now);
    let age = now.saturating_sub(a.last_access_secs);
    let why = match tier {
        Tier::Hot if age <= cfg.hot_secs => {
            format!(
                "hot: last read {age}s ago, within hot_secs={}",
                cfg.hot_secs
            )
        }
        Tier::Hot => format!(
            "hot: last read {age}s ago (within warm_secs={}) and read {} times, at least hot_count={}",
            cfg.warm_secs, a.access_count, cfg.hot_count
        ),
        Tier::Warm => format!(
            "warm: last read {age}s ago, past hot_secs={} but within warm_secs={}, read {} times (under hot_count={})",
            cfg.hot_secs, cfg.warm_secs, a.access_count, cfg.hot_count
        ),
        Tier::Cold => format!(
            "cold: last read {age}s ago, past warm_secs={}; demoted, not removed; one read makes it hot",
            cfg.warm_secs
        ),
    };
    TierInfo {
        tier,
        last_access_secs: a.last_access_secs,
        age_secs: age,
        access_count: a.access_count,
        why,
    }
}

/// An entity with the tier it ranks under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ranked {
    /// The entity.
    pub entity: Entity,
    /// Its tier and the numbers behind it, as of just before this read.
    pub info: TierInfo,
}

/// A tier change: `(id, from, to)`.
pub(super) type Change = (String, Tier, Tier);

fn note(counters: &mut Counters, changes: &mut Vec<Change>, id: &str, from: Tier, to: Tier) {
    if from != to {
        bump(
            &mut counters.tier_changes,
            &format!("{}_to_{}", from.as_str(), to.as_str()),
        );
        changes.push((id.to_owned(), from, to));
    }
}

impl EntityStore {
    pub(super) fn report_changes(&self, changes: &[Change]) {
        for (id, from, to) in changes {
            self.observer.event(
                &Event::new("memory", Level::Info, names::MEMORY_TIER_CHANGED)
                    .field("id", id.clone())
                    .field("from", from.as_str())
                    .field("to", to.as_str()),
            );
        }
    }

    /// Records a read of `ids` (those that exist): one transaction, one write of `access.json`.
    /// A tier change that happened by the passage of time (hot to cold) is counted first, then
    /// the read's own (cold to hot), so a demotion and its reversal are both visible.
    pub(super) fn record_access(&self, ids: &[&str]) -> Result<(), GraphError> {
        if ids.is_empty() {
            return Ok(());
        }
        let cfg = self.cfg;
        let now = self.now_secs();
        let changes = self.txn(true, |state, counters, access| {
            let mut changes = Vec::new();
            for id in ids {
                if !(state.entities.contains_key(*id) || state.relationships.contains_key(*id)) {
                    continue;
                }
                let cur = access.get(id);
                let derived = cur.tier(&cfg, now);
                if let Some(seen) = cur.observed {
                    note(counters, &mut changes, id, seen, derived);
                }
                let next = Access {
                    last_access_secs: now,
                    access_count: cur.access_count.saturating_add(1),
                    observed: Some(Tier::Hot),
                };
                note(counters, &mut changes, id, derived, Tier::Hot);
                access.entries.insert((*id).to_owned(), next);
            }
            Ok((changes, false))
        })?;
        self.report_changes(&changes);
        Ok(())
    }

    /// Compares every record's derived tier with the one last reported, counting and reporting
    /// each change. Bounded by the entity and relationship caps; removes nothing.
    ///
    /// # Errors
    /// [`GraphError::Store`] when a file cannot be read or written.
    pub fn sweep(&self) -> Result<u64, GraphError> {
        let cfg = self.cfg;
        let now = self.now_secs();
        let changes = self.txn(true, |state, counters, access| {
            let mut changes = Vec::new();
            for id in state.entities.keys().chain(state.relationships.keys()) {
                let Some(cur) = access.entries.get(id).copied() else {
                    continue;
                };
                let derived = cur.tier(&cfg, now);
                if let Some(seen) = cur.observed {
                    note(counters, &mut changes, id, seen, derived);
                }
                if cur.observed != Some(derived) {
                    access.entries.insert(
                        id.clone(),
                        Access {
                            observed: Some(derived),
                            ..cur
                        },
                    );
                }
            }
            Ok((changes, false))
        })?;
        self.report_changes(&changes);
        Ok(changes.len() as u64)
    }

    /// The tier of the record `id` (entity or relationship) and why; `None` when there is no such record.
    /// Reading the explanation is not itself a read of the record.
    ///
    /// # Errors
    /// [`GraphError::Store`] when a file cannot be read.
    pub fn explain(&self, id: &str) -> Result<Option<TierInfo>, GraphError> {
        let state = self.load()?;
        if !(state.entities.contains_key(id) || state.relationships.contains_key(id)) {
            return Ok(None);
        }
        let access: AccessFile = super::store::read_json(&self.access_path())?;
        Ok(Some(explain(&self.cfg, &access.get(id), self.now_secs())))
    }

    /// Entities of `kind` ranked by tier (hot first), then last read (latest first), then read
    /// count (most first), then id: a total order, so the same state gives the same list.
    ///
    /// # Errors
    /// As [`EntityStore::by_kind`].
    pub fn by_kind_ranked(
        &self,
        kind: &str,
        limit: Option<usize>,
    ) -> Result<Vec<Ranked>, GraphError> {
        self.select_ranked("by_kind", kind, limit, |e| e.kind == kind)
    }

    /// As [`EntityStore::by_kind_ranked`], for entities named exactly `name`.
    ///
    /// # Errors
    /// As [`EntityStore::by_name`].
    pub fn by_name_ranked(
        &self,
        name: &str,
        limit: Option<usize>,
    ) -> Result<Vec<Ranked>, GraphError> {
        self.select_ranked("by_name", name, limit, |e| e.name == name)
    }

    fn select_ranked(
        &self,
        op: &'static str,
        key: &str,
        limit: Option<usize>,
        keep: impl Fn(&Entity) -> bool,
    ) -> Result<Vec<Ranked>, GraphError> {
        let found = self.select_untouched(op, key, limit, keep)?;
        let access: AccessFile = super::store::read_json(&self.access_path())?;
        let now = self.now_secs();
        let mut ranked: Vec<Ranked> = found
            .into_iter()
            .map(|entity| {
                let info = explain(&self.cfg, &access.get(&entity.id), now);
                Ranked { entity, info }
            })
            .collect();
        ranked.sort_by(|a, b| {
            (
                a.info.tier,
                Reverse(a.info.last_access_secs),
                Reverse(a.info.access_count),
                &a.entity.id,
            )
                .cmp(&(
                    b.info.tier,
                    Reverse(b.info.last_access_secs),
                    Reverse(b.info.access_count),
                    &b.entity.id,
                ))
        });
        let ids: Vec<&str> = ranked.iter().map(|r| r.entity.id.as_str()).collect();
        self.record_access(&ids)?;
        Ok(ranked)
    }
}
