//! The development queue as stored objects (spec 208).
//!
//! A work item is a row, its status changes are events in an append-only journal, and selection is one write
//! transaction, so two ticks can never claim the same item.
//!
//! `item: id -> encoded WorkItem`, `event: (id, seq) -> encoded QueueEvent`. Like the approval store, each method
//! opens and commits its own transaction; the write lock redb takes per transaction is the single writer.

use pii_abi::PiiError;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

use crate::codec::{from_bytes, to_bytes};

/// Work items: id -> encoded [`WorkItem`].
const ITEM: TableDefinition<'_, &str, &[u8]> = TableDefinition::new("queue_item");

/// Status journal: `(id, seq)` -> encoded [`QueueEvent`].
const EVENT: TableDefinition<'_, (&str, u64), &[u8]> = TableDefinition::new("queue_event");

/// Lifecycle of one work item. `Pending` is the only state `select` may claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemStatus {
    /// Waiting to be selected.
    Pending,
    /// Claimed by a tick and in dispatch.
    Running,
    /// Its criteria passed.
    Done,
    /// Its criteria failed or its dispatch was refused; it waits for an operator.
    Parked,
}

impl ItemStatus {
    /// The lowercase name used in events and summaries.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Done => "done",
            Self::Parked => "parked",
        }
    }
}

/// One development work item. Identity is `id`; `order` is the queue position, assigned on first insert.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItem {
    /// Stable identifier, e.g. `A75` or `208-the-queue-is-a-graph`.
    pub id: String,
    /// Spec path the coder node is pointed at.
    pub spec: String,
    /// One-line title.
    pub title: String,
    /// Criteria commands, verbatim; the judge node runs them in order.
    pub criteria: Vec<String>,
    /// Paths the item is expected to touch.
    pub touches: Vec<String>,
    /// Ids that must be `Done` before this item may be selected.
    pub needs: Vec<String>,
    /// Current status.
    pub status: ItemStatus,
    /// How many times the item has been claimed.
    pub attempts: u32,
    /// The worker session, recorded as soon as it is known, so a cut worker can be resumed.
    pub session: Option<String>,
    /// The last reason the status changed (a refusal, a failing criterion, a blocker).
    pub reason: Option<String>,
    /// Queue position; `0` until the store assigns it.
    pub order: u64,
    /// The run that claimed the item (spec 353 R4), so a restart can tell a live claim from a dead one.
    #[serde(default)]
    pub claimed_by: Option<String>,
    /// When the claim was made, in unix milliseconds.
    #[serde(default)]
    pub claimed_at_ms: Option<u64>,
}

impl WorkItem {
    /// A pending item with no spec, criteria or touches yet: the minimum a queue row needs.
    #[must_use]
    pub fn new(id: &str, needs: &[&str]) -> Self {
        Self {
            id: id.to_owned(),
            spec: String::new(),
            title: String::new(),
            criteria: Vec::new(),
            touches: Vec::new(),
            needs: needs.iter().map(|n| (*n).to_owned()).collect(),
            status: ItemStatus::Pending,
            attempts: 0,
            session: None,
            reason: None,
            order: 0,
            claimed_by: None,
            claimed_at_ms: None,
        }
    }
}

/// One unmet edge: `item` waits on `needs`, which is in `need_status` (`missing` when no such item exists).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blocker {
    /// The waiting item.
    pub item: String,
    /// The item it waits on.
    pub needs: String,
    /// The status of `needs`, or `missing`.
    pub need_status: String,
}

/// What one selection decided. Every variant is distinct so a caller can never read a block as an empty queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// The first pending item whose needs are all done, now `Running`.
    Claimed(Box<WorkItem>),
    /// Pending items exist but every one waits on an unmet edge.
    Blocked(Vec<Blocker>),
    /// Nothing is pending.
    Empty,
    /// An item is already `Running`; the queue is single-lane, so nothing else is claimed.
    AlreadyRunning(String),
}

/// One journalled status change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueEvent {
    /// Position in the item's journal, from 1.
    pub seq: u64,
    /// `item.claimed`, or `item.<status>`.
    pub kind: String,
    /// The status after the change.
    pub status: String,
    /// The reason given with the change, if any.
    pub reason: Option<String>,
    /// Who made the change (a node name or an operator).
    pub by: String,
}

/// The work-item store. One redb file, opened once by the daemon and shared by the queue nodes.
pub struct QueueStore {
    db: Database,
}

fn internal(op: &str, e: impl std::fmt::Display) -> PiiError {
    PiiError::Internal(format!("queue: {op}: {e}"))
}

impl QueueStore {
    /// The persisted format version (spec 105), as the other stores report it.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn format_version(&self) -> Result<u64, PiiError> {
        crate::identity::read_stamp(&self.db)
    }

    /// Opens (creating if needed) the queue store at `path`.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn open(path: &std::path::Path) -> Result<Self, PiiError> {
        let db = Database::builder()
            .set_cache_size(4 * 1024 * 1024)
            .create(path)
            .map_err(|e| internal("open", format!("{}: {e}", path.display())))?;
        Self::init(db, path)
    }

    /// Opens a queue store that lives only in memory (spec 371 R1): lint needs the kinds, not a file.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn open_in_memory() -> Result<Self, PiiError> {
        let db = Database::builder()
            .create_with_backend(redb::backends::InMemoryBackend::new())
            .map_err(|e| internal("open", format!("in-memory: {e}")))?;
        Self::init(db, std::path::Path::new(":memory:"))
    }

    /// Defines the tables and stamps the format on an open database.
    fn init(db: Database, path: &std::path::Path) -> Result<Self, PiiError> {
        {
            let write = db.begin_write().map_err(|e| internal("begin_write", e))?;
            write
                .open_table(ITEM)
                .map_err(|e| internal("open item table", e))?;
            write
                .open_table(EVENT)
                .map_err(|e| internal("open event table", e))?;
            write.commit().map_err(|e| internal("commit tables", e))?;
        }
        crate::identity::stamp(&db, path)?;
        Ok(Self { db })
    }

    /// Inserts or replaces an item. A new id gets the next queue position; an existing id keeps its own.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb or encode failure.
    pub fn put_item(&self, item: &WorkItem) -> Result<(), PiiError> {
        let write = self
            .db
            .begin_write()
            .map_err(|e| internal("begin_write", e))?;
        {
            let mut table = write
                .open_table(ITEM)
                .map_err(|e| internal("open item", e))?;
            let mut row = item.clone();
            let existing = table
                .get(row.id.as_str())
                .map_err(|e| internal("get", e))?
                .map(|v| from_bytes::<WorkItem>(v.value()))
                .transpose()?;
            row.order = match existing {
                Some(prev) => prev.order,
                None => next_order(&table)?,
            };
            let bytes = to_bytes(&row)?;
            table
                .insert(row.id.as_str(), bytes.as_slice())
                .map_err(|e| internal("insert", e))?;
        }
        write.commit().map_err(|e| internal("commit put", e))
    }

    /// The item with `id`, if it exists.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb or decode failure.
    pub fn get_item(&self, id: &str) -> Result<Option<WorkItem>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| internal("begin_read", e))?;
        let table = read
            .open_table(ITEM)
            .map_err(|e| internal("open item", e))?;
        table
            .get(id)
            .map_err(|e| internal("get", e))?
            .map(|v| from_bytes(v.value()))
            .transpose()
    }

    /// Every item in queue order.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb or decode failure.
    pub fn list_items(&self) -> Result<Vec<WorkItem>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| internal("begin_read", e))?;
        let table = read
            .open_table(ITEM)
            .map_err(|e| internal("open item", e))?;
        let mut items = Vec::new();
        for row in table.iter().map_err(|e| internal("iter", e))? {
            let (_, v) = row.map_err(|e| internal("iter row", e))?;
            items.push(from_bytes::<WorkItem>(v.value())?);
        }
        items.sort_by_key(|i| i.order);
        Ok(items)
    }

    /// Selects the next item in one write transaction. A `Running` item short-circuits to
    /// [`Claim::AlreadyRunning`]; otherwise the first pending item whose needs are all `Done` is claimed.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb or codec failure.
    pub fn claim_next(&self, by: &str) -> Result<Claim, PiiError> {
        let write = self
            .db
            .begin_write()
            .map_err(|e| internal("begin_write", e))?;
        let claim = {
            let mut table = write
                .open_table(ITEM)
                .map_err(|e| internal("open item", e))?;
            let mut items = Vec::new();
            for row in table.iter().map_err(|e| internal("iter", e))? {
                let (_, v) = row.map_err(|e| internal("iter row", e))?;
                items.push(from_bytes::<WorkItem>(v.value())?);
            }
            items.sort_by_key(|i| i.order);
            let decided = decide(&items);
            if let Claim::Claimed(item) = &decided {
                let mut claimed = item.clone();
                claimed.status = ItemStatus::Running;
                claimed.attempts += 1;
                claimed.claimed_by = Some(by.to_owned());
                claimed.claimed_at_ms = Some(now_ms());
                let bytes = to_bytes(&claimed)?;
                table
                    .insert(claimed.id.as_str(), bytes.as_slice())
                    .map_err(|e| internal("insert claim", e))?;
                append_event(
                    &write,
                    &claimed.id,
                    "item.claimed",
                    ItemStatus::Running,
                    None,
                    by,
                )?;
                Claim::Claimed(claimed)
            } else {
                decided
            }
        };
        write.commit().map_err(|e| internal("commit claim", e))?;
        Ok(claim)
    }

    /// Sets an item's status and journals the change in the same transaction.
    ///
    /// # Errors
    /// [`PiiError::Internal`] when the item is unknown, or on a redb or codec failure.
    pub fn set_status(
        &self,
        id: &str,
        status: ItemStatus,
        reason: Option<&str>,
        by: &str,
    ) -> Result<(), PiiError> {
        let write = self
            .db
            .begin_write()
            .map_err(|e| internal("begin_write", e))?;
        {
            let mut table = write
                .open_table(ITEM)
                .map_err(|e| internal("open item", e))?;
            let mut item = table
                .get(id)
                .map_err(|e| internal("get", e))?
                .map(|v| from_bytes::<WorkItem>(v.value()))
                .transpose()?
                .ok_or_else(|| internal("set_status", format!("no item '{id}'")))?;
            item.status = status;
            item.reason = reason.map(str::to_owned);
            let bytes = to_bytes(&item)?;
            table
                .insert(id, bytes.as_slice())
                .map_err(|e| internal("insert status", e))?;
            let kind = format!("item.{}", status.name());
            append_event(&write, id, &kind, status, reason, by)?;
        }
        write.commit().map_err(|e| internal("commit status", e))
    }

    /// Offers again every `Running` row whose claiming run is dead (spec 353 R4). `is_dead` is asked with the
    /// recorded claimant, or `""` for a row with no recorded claim. Each such row goes back to `Pending`, and the
    /// change is journalled as `requeued_after_restart`. Returns how many rows went back.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb or codec failure.
    pub fn requeue_dead_claims(&self, is_dead: impl Fn(&str) -> bool) -> Result<u64, PiiError> {
        let write = self
            .db
            .begin_write()
            .map_err(|e| internal("begin_write", e))?;
        let mut requeued = 0;
        {
            let mut table = write
                .open_table(ITEM)
                .map_err(|e| internal("open item", e))?;
            let mut dead = Vec::new();
            for row in table.iter().map_err(|e| internal("iter", e))? {
                let (_, v) = row.map_err(|e| internal("iter row", e))?;
                let item = from_bytes::<WorkItem>(v.value())?;
                if item.status == ItemStatus::Running
                    && is_dead(item.claimed_by.as_deref().unwrap_or(""))
                {
                    dead.push(item);
                }
            }
            for mut item in dead {
                item.status = ItemStatus::Pending;
                item.reason = Some(String::from(
                    "requeued after restart: the claiming run is gone",
                ));
                item.claimed_by = None;
                item.claimed_at_ms = None;
                let bytes = to_bytes(&item)?;
                table
                    .insert(item.id.as_str(), bytes.as_slice())
                    .map_err(|e| internal("insert requeue", e))?;
                append_event(
                    &write,
                    &item.id,
                    REQUEUED_AFTER_RESTART,
                    ItemStatus::Pending,
                    item.reason.as_deref(),
                    "boot",
                )?;
                requeued += 1;
            }
        }
        write.commit().map_err(|e| internal("commit requeue", e))?;
        Ok(requeued)
    }

    /// The status journal of `id`, oldest first.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb or decode failure.
    pub fn events(&self, id: &str) -> Result<Vec<QueueEvent>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| internal("begin_read", e))?;
        let table = read
            .open_table(EVENT)
            .map_err(|e| internal("open event", e))?;
        let mut out = Vec::new();
        for row in table
            .range((id, 0)..=(id, u64::MAX))
            .map_err(|e| internal("range", e))?
        {
            let (_, v) = row.map_err(|e| internal("range row", e))?;
            out.push(from_bytes::<QueueEvent>(v.value())?);
        }
        Ok(out)
    }
}

/// The selection rule, pure over the stored items (already in queue order).
fn decide(items: &[WorkItem]) -> Claim {
    if let Some(running) = items.iter().find(|i| i.status == ItemStatus::Running) {
        return Claim::AlreadyRunning(running.id.clone());
    }
    let pending: Vec<&WorkItem> = items
        .iter()
        .filter(|i| i.status == ItemStatus::Pending)
        .collect();
    if pending.is_empty() {
        return Claim::Empty;
    }
    let mut blockers = Vec::new();
    for item in &pending {
        let mut unmet = false;
        for need in &item.needs {
            match items.iter().find(|i| &i.id == need) {
                Some(dep) if dep.status == ItemStatus::Done => {}
                Some(dep) => {
                    unmet = true;
                    blockers.push(Blocker {
                        item: item.id.clone(),
                        needs: need.clone(),
                        need_status: dep.status.name().to_owned(),
                    });
                }
                None => {
                    unmet = true;
                    blockers.push(Blocker {
                        item: item.id.clone(),
                        needs: need.clone(),
                        need_status: "missing".to_owned(),
                    });
                }
            }
        }
        if !unmet {
            return Claim::Claimed(Box::new((*item).clone()));
        }
    }
    Claim::Blocked(blockers)
}

/// The journal kind of a row put back to `pending` because its claiming run died.
const REQUEUED_AFTER_RESTART: &str = "requeued_after_restart";

/// Unix time in milliseconds, for a claim's `claimed_at_ms`.
// reason: the claim time is an operator-facing stamp on the row; nothing replays from it.
#[allow(clippy::disallowed_methods)]
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The next queue position: one past the largest assigned so far.
fn next_order(table: &redb::Table<'_, &str, &[u8]>) -> Result<u64, PiiError> {
    let mut max = 0;
    for row in table.iter().map_err(|e| internal("iter", e))? {
        let (_, v) = row.map_err(|e| internal("iter row", e))?;
        max = max.max(from_bytes::<WorkItem>(v.value())?.order);
    }
    Ok(max + 1)
}

/// Appends one event to `id`'s journal inside an open write transaction.
fn append_event(
    write: &redb::WriteTransaction,
    id: &str,
    kind: &str,
    status: ItemStatus,
    reason: Option<&str>,
    by: &str,
) -> Result<(), PiiError> {
    let mut events = write
        .open_table(EVENT)
        .map_err(|e| internal("open event", e))?;
    let last = events
        .range((id, 0)..=(id, u64::MAX))
        .map_err(|e| internal("range", e))?
        .next_back()
        .transpose()
        .map_err(|e| internal("range row", e))?
        .map_or(0, |(k, _)| k.value().1);
    let event = QueueEvent {
        seq: last + 1,
        kind: kind.to_owned(),
        status: status.name().to_owned(),
        reason: reason.map(str::to_owned),
        by: by.to_owned(),
    };
    let bytes = to_bytes(&event)?;
    events
        .insert((id, event.seq), bytes.as_slice())
        .map_err(|e| internal("insert event", e))?;
    Ok(())
}
