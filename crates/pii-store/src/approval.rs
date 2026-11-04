//! Approval records (F5.10): `approval: approval_id -> encoded record`.
//! Resolve writes the decision idempotently; unknown ids are `NotFound`.

use pii_abi::PiiError;
use redb::{Database, ReadableDatabase, ReadableTable};
use serde::{Deserialize, Serialize};

use crate::codec::{from_bytes, to_bytes};
use crate::tables::APPROVAL;

/// One gate approval request (F5.10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    /// Deterministic id: `blake3(run_id, node_id)[..16]` hex.
    pub approval_id: String,
    /// The suspended run.
    pub run_id: String,
    /// The gate node.
    pub node: String,
    /// Policy name.
    pub policy: String,
    /// Prompt for the approver.
    pub prompt: String,
    /// Requested-at epoch millis.
    pub requested_at_ms: u64,
    /// The decision (`None` = pending).
    pub decision: Option<bool>,
    /// Operator note.
    pub note: Option<String>,
    /// Decided-at epoch millis (0 while pending).
    pub decided_at_ms: u64,
}

/// The approval table. Open/commit inside each method; no transaction is
/// stored or returned (single-writer rule in lib.rs).
pub struct ApprovalStore {
    db: Database,
}

/// Outcome of a resolve (F5.10: idempotent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveOutcome {
    /// `true` when the decision was newly applied (first resolve).
    pub resumed: bool,
}

impl ApprovalStore {
    /// Opens (creating if needed) the approval store on `db`.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn open(path: &std::path::Path) -> Result<Self, PiiError> {
        // RSS-gate fix (2026-09-05): redb defaults to a 1 GiB cache; this
        // store is small (approvals/channels/heal/memory/timers), so 4 MiB
        // is ample and avoids each auxiliary store compounding the
        // daemon's total cache footprint under soak (see pii-store/src/
        // lib.rs's Store::open for the full rationale).
        let db = Database::builder()
            .set_cache_size(4 * 1024 * 1024)
            .create(path)
            .map_err(|e| PiiError::Internal(format!("approval: open {}: {e}", path.display())))?;
        Self::init(db, path)
    }

    /// Opens an approval store that lives only in memory (spec 371 R1): lint needs the kind, not a file.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn open_in_memory() -> Result<Self, PiiError> {
        let db = Database::builder()
            .create_with_backend(redb::backends::InMemoryBackend::new())
            .map_err(|e| PiiError::Internal(format!("approval: in-memory open: {e}")))?;
        Self::init(db, std::path::Path::new(":memory:"))
    }

    /// Defines the table and stamps the format on an open database.
    fn init(db: Database, path: &std::path::Path) -> Result<Self, PiiError> {
        {
            let write = db
                .begin_write()
                .map_err(|e| PiiError::Internal(format!("approval: begin_write: {e}")))?;
            write
                .open_table(APPROVAL)
                .map_err(|e| PiiError::Internal(format!("approval: open table: {e}")))?;
            write
                .commit()
                .map_err(|e| PiiError::Internal(format!("approval: commit tables: {e}")))?;
        }
        crate::identity::stamp(&db, path)?;
        Ok(Self { db })
    }

    /// The persisted format version (spec 105).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn format_version(&self) -> Result<u64, PiiError> {
        crate::identity::read_stamp(&self.db)
    }

    /// Creates or updates a pending approval request.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb or encode failure.
    pub fn put_approval(&self, a: &Approval) -> Result<(), PiiError> {
        let bytes = to_bytes(a)?;
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("approval: begin_write: {e}")))?;
        {
            let mut t = write
                .open_table(APPROVAL)
                .map_err(|e| PiiError::Internal(format!("approval: open table: {e}")))?;
            t.insert(a.approval_id.as_str(), bytes.as_slice())
                .map_err(|e| PiiError::Internal(format!("approval: insert: {e}")))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("approval: commit put: {e}")))
    }

    /// Inserts `a` only when no row exists for its id, check-and-insert in
    /// one write transaction. Returns `true` when inserted; an existing row
    /// (pending or decided) is never touched.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb or encode failure.
    pub fn put_approval_if_absent(&self, a: &Approval) -> Result<bool, PiiError> {
        let bytes = to_bytes(a)?;
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("approval: begin_write: {e}")))?;
        let inserted = {
            let mut t = write
                .open_table(APPROVAL)
                .map_err(|e| PiiError::Internal(format!("approval: open table: {e}")))?;
            let exists = t
                .get(a.approval_id.as_str())
                .map_err(|e| PiiError::Internal(format!("approval: get: {e}")))?
                .is_some();
            if !exists {
                t.insert(a.approval_id.as_str(), bytes.as_slice())
                    .map_err(|e| PiiError::Internal(format!("approval: insert: {e}")))?;
            }
            !exists
        };
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("approval: commit put: {e}")))?;
        Ok(inserted)
    }

    /// Reads one approval (`None` when unknown).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb or decode failure.
    pub fn get_approval(&self, id: &str) -> Result<Option<Approval>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("approval: begin_read: {e}")))?;
        let t = read
            .open_table(APPROVAL)
            .map_err(|e| PiiError::Internal(format!("approval: open table: {e}")))?;
        let Some(v) = t
            .get(id)
            .map_err(|e| PiiError::Internal(format!("approval: get: {e}")))?
        else {
            return Ok(None);
        };
        Ok(Some(from_bytes(v.value())?))
    }

    /// Applies a decision idempotently (F5.10): the FIRST resolve writes the
    /// decision; later resolves return `resumed == false` and change nothing.
    ///
    /// # Errors
    /// [`PiiError::NotFound`] for an unknown id; redb errors otherwise.
    pub fn resolve_approval(
        &self,
        id: &str,
        allow: bool,
        note: Option<&str>,
        now_ms: u64,
    ) -> Result<ResolveOutcome, PiiError> {
        let existing = self
            .get_approval(id)?
            .ok_or_else(|| PiiError::NotFound(format!("approval '{id}'")))?;
        if existing.decision.is_some() {
            return Ok(ResolveOutcome { resumed: false });
        }
        let mut updated = existing;
        updated.decision = Some(allow);
        updated.note = note.map(str::to_owned);
        updated.decided_at_ms = now_ms;
        self.put_approval(&updated)?;
        Ok(ResolveOutcome { resumed: true })
    }

    /// Every still-pending approval, in id order.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb or decode failure.
    pub fn pending_approvals(&self) -> Result<Vec<Approval>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("approval: begin_read: {e}")))?;
        let t = read
            .open_table(APPROVAL)
            .map_err(|e| PiiError::Internal(format!("approval: open table: {e}")))?;
        let mut out = Vec::new();
        for row in t
            .iter()
            .map_err(|e| PiiError::Internal(format!("approval: iter: {e}")))?
        {
            let Ok(row) = row else { continue };
            let a: Approval = from_bytes(row.1.value())?;
            if a.decision.is_none() {
                out.push(a);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    // test-only codec round-trip asserts
    #![allow(clippy::expect_used)]

    use super::{Approval, ApprovalStore};

    fn scratch() -> std::path::PathBuf {
        let dir = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/_tmp/phase3/approvaltest"
        ));
        let _ = std::fs::create_dir_all(dir);
        dir.join(format!("ap-{}.redb", std::process::id()))
    }

    fn sample(id: &str) -> Approval {
        Approval {
            approval_id: String::from(id),
            run_id: String::from("r1"),
            node: String::from("g"),
            policy: String::from("bash"),
            prompt: String::from("run ls?"),
            requested_at_ms: 5,
            decision: None,
            note: None,
            decided_at_ms: 0,
        }
    }

    #[test]
    fn resolve_is_idempotent_and_typed() {
        let path = scratch();
        let s = ApprovalStore::open(&path).unwrap();
        s.put_approval(&sample("a1")).unwrap();
        // Unknown id is NotFound (typed), not a panic.
        let err = s.resolve_approval("nope", true, None, 1).unwrap_err();
        assert!(matches!(err, pii_abi::PiiError::NotFound(_)), "{err:?}");
        let first = s.resolve_approval("a1", true, None, 7).unwrap();
        assert!(first.resumed);
        let second = s.resolve_approval("a1", false, None, 9).unwrap();
        assert!(!second.resumed, "second resolve is a no-op");
        let a = s.get_approval("a1").unwrap().unwrap();
        assert_eq!(a.decision, Some(true), "first decision sticks");
        assert_eq!(a.decided_at_ms, 7);
        assert_eq!(s.pending_approvals().unwrap().len(), 0);
        let _ = std::fs::remove_file(&path);
    }
}
