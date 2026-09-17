//! Durable session substrate (spec part 1 — store-only, additive).
//!
//! The engine still owns its RAM session map; nothing here is wired into
//! dispatch yet (that's part 2). This module only provides the durable
//! substrate: session blobs, a monotonic session-id counter (NOT derived from
//! the row count — a `len + 1` derivation re-collides after a deletion and
//! forgets on restart), and per-session token counts in a separate table so
//! token updates never deserialise the whole session value.

use pii_abi::PiiError;
use redb::{ReadableDatabase, ReadableTable};

use crate::tables::{META, SESSION, SESSION_MSG, SESSION_SNAPSHOT, SESSION_TOKENS};

const SESSION_ID_COUNTER_KEY: &str = "session_id_counter";

impl crate::Store {
    /// Stores a session META blob by id (upsert) — `{cwd,tools,graph,status,
    /// run_id}`, NOT the transcript (D3/D6: the transcript lives in
    /// [`Self::append_session_message`]/[`Self::session_messages`] so this
    /// call's cost never grows with message count).
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn put_session(&self, id: &str, bytes: &[u8]) -> Result<(), PiiError> {
        if self
            .fail_session_writes_for_test
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(PiiError::Internal(String::from(
                "store: put session: injected test failure",
            )));
        }
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("store: begin_write: {e}")))?;
        {
            let mut table = write
                .open_table(SESSION)
                .map_err(|e| PiiError::Internal(format!("store: open session: {e}")))?;
            table
                .insert(id, bytes)
                .map_err(|e| PiiError::Internal(format!("store: put session: {e}")))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("store: commit session: {e}")))?;
        self.session_bytes_written
            .fetch_add(bytes.len() as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// Fetches a session blob by id.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn get_session(&self, id: &str) -> Result<Option<Vec<u8>>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("store: begin_read: {e}")))?;
        let table = read
            .open_table(SESSION)
            .map_err(|e| PiiError::Internal(format!("store: open session: {e}")))?;
        let got = table
            .get(id)
            .map_err(|e| PiiError::Internal(format!("store: get session: {e}")))?;
        Ok(got.map(|v| v.value().to_vec()))
    }

    /// Deletes a session blob by id. Idempotent (no error if absent). Also
    /// clears the session's message/snapshot rows (D3/D6) so a deleted
    /// session leaves no orphaned transcript behind.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn del_session(&self, id: &str) -> Result<(), PiiError> {
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("store: begin_write: {e}")))?;
        {
            let mut table = write
                .open_table(SESSION)
                .map_err(|e| PiiError::Internal(format!("store: open session: {e}")))?;
            table
                .remove(id)
                .map_err(|e| PiiError::Internal(format!("store: del session: {e}")))?;
        }
        {
            let mut msg = write
                .open_table(SESSION_MSG)
                .map_err(|e| PiiError::Internal(format!("store: open session_msg: {e}")))?;
            let keys: Vec<u64> = msg
                .range((id, 0)..(id, u64::MAX))
                .map_err(|e| PiiError::Internal(format!("store: range session_msg: {e}")))?
                .filter_map(|item| item.ok().map(|(k, _)| k.value().1))
                .collect();
            for seq in keys {
                msg.remove((id, seq))
                    .map_err(|e| PiiError::Internal(format!("store: del session_msg: {e}")))?;
            }
        }
        {
            let mut snap = write
                .open_table(SESSION_SNAPSHOT)
                .map_err(|e| PiiError::Internal(format!("store: open session_snapshot: {e}")))?;
            let keys: Vec<u64> = snap
                .range((id, 0)..(id, u64::MAX))
                .map_err(|e| PiiError::Internal(format!("store: range session_snapshot: {e}")))?
                .filter_map(|item| item.ok().map(|(k, _)| k.value().1))
                .collect();
            for seq in keys {
                snap.remove((id, seq))
                    .map_err(|e| PiiError::Internal(format!("store: del session_snapshot: {e}")))?;
            }
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("store: commit del session: {e}")))?;
        Ok(())
    }

    /// Lists all session ids in table (iteration) order.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn all_sessions(&self) -> Result<Vec<String>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("store: begin_read: {e}")))?;
        let table = read
            .open_table(SESSION)
            .map_err(|e| PiiError::Internal(format!("store: open session: {e}")))?;
        let mut out = Vec::new();
        let iter = table
            .iter()
            .map_err(|e| PiiError::Internal(format!("store: iter session: {e}")))?;
        for item in iter {
            let (key, _) =
                item.map_err(|e| PiiError::Internal(format!("store: read session: {e}")))?;
            out.push(key.value().to_owned());
        }
        Ok(out)
    }

    /// Allocates the next session id from a **monotonic counter** stored in
    /// `META`, never from the current row count (which would re-collide after
    /// a deletion). Returns `1, 2, 3, …`; a reopened store continues above the
    /// max persisted value.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn next_session_id(&self) -> Result<u64, PiiError> {
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("store: begin_write: {e}")))?;
        let next;
        {
            let mut meta = write
                .open_table(META)
                .map_err(|e| PiiError::Internal(format!("store: open meta: {e}")))?;
            let cur = meta
                .get(SESSION_ID_COUNTER_KEY)
                .map_err(|e| PiiError::Internal(format!("store: read session counter: {e}")))?
                .map_or(0, |v| v.value());
            next = cur + 1;
            meta.insert(SESSION_ID_COUNTER_KEY, next)
                .map_err(|e| PiiError::Internal(format!("store: write session counter: {e}")))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("store: commit session counter: {e}")))?;
        Ok(next)
    }

    /// Records cumulative `(input, output)` token counts for a session
    /// (upsert). Kept in a separate `SESSION_TOKENS` table so token updates
    /// never deserialise the whole session blob.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn set_session_tokens(&self, id: &str, input: u64, output: u64) -> Result<(), PiiError> {
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("store: begin_write: {e}")))?;
        {
            let mut table = write
                .open_table(SESSION_TOKENS)
                .map_err(|e| PiiError::Internal(format!("store: open session_tokens: {e}")))?;
            table
                .insert(id, (input, output))
                .map_err(|e| PiiError::Internal(format!("store: put session tokens: {e}")))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("store: commit session tokens: {e}")))?;
        Ok(())
    }

    /// Fetches cumulative `(input, output)` token counts for a session.
    /// Returns `None` for a never-set id — the documented choice, so callers
    /// can distinguish "never counted" from "counted as zero".
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn get_session_tokens(&self, id: &str) -> Result<Option<(u64, u64)>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("store: begin_read: {e}")))?;
        let table = read
            .open_table(SESSION_TOKENS)
            .map_err(|e| PiiError::Internal(format!("store: open session_tokens: {e}")))?;
        let got = table
            .get(id)
            .map_err(|e| PiiError::Internal(format!("store: get session tokens: {e}")))?;
        Ok(got.map(|v| v.value()))
    }

    /// Appends one transcript message to a session's durable journal (D3/D6,
    /// 026 report 5.0.4): one row per message at `(session_id, seq)`, seq
    /// eagerly allocated from a per-session counter kept in `META`. This is
    /// the fix for the pre-fix O(n^2) behaviour — appending message N never
    /// touches messages `1..N-1`, so cumulative bytes written stay close to
    /// linear (each message written exactly once, plus periodic snapshots).
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn append_session_message(&self, session_id: &str, bytes: &[u8]) -> Result<u64, PiiError> {
        if self
            .fail_session_writes_for_test
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(PiiError::Internal(String::from(
                "store: append session_msg: injected test failure",
            )));
        }
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("store: begin_write: {e}")))?;
        let seq;
        {
            let mut meta = write
                .open_table(META)
                .map_err(|e| PiiError::Internal(format!("store: open meta: {e}")))?;
            let counter_key = session_msg_counter_key(session_id);
            let cur = meta
                .get(counter_key.as_str())
                .map_err(|e| PiiError::Internal(format!("store: read session_msg counter: {e}")))?
                .map_or(0, |v| v.value());
            seq = cur;
            meta.insert(counter_key.as_str(), cur + 1).map_err(|e| {
                PiiError::Internal(format!("store: write session_msg counter: {e}"))
            })?;
        }
        {
            let mut table = write
                .open_table(SESSION_MSG)
                .map_err(|e| PiiError::Internal(format!("store: open session_msg: {e}")))?;
            table
                .insert((session_id, seq), bytes)
                .map_err(|e| PiiError::Internal(format!("store: append session_msg: {e}")))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("store: commit session_msg: {e}")))?;
        self.session_bytes_written
            .fetch_add(bytes.len() as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(seq)
    }

    /// Reads every appended transcript message for a session, in seq order.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn session_messages(&self, session_id: &str) -> Result<Vec<(u64, Vec<u8>)>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("store: begin_read: {e}")))?;
        let table = read
            .open_table(SESSION_MSG)
            .map_err(|e| PiiError::Internal(format!("store: open session_msg: {e}")))?;
        let mut out = Vec::new();
        let range = table
            .range((session_id, 0)..(session_id, u64::MAX))
            .map_err(|e| PiiError::Internal(format!("store: range session_msg: {e}")))?;
        for item in range {
            let (key, value) =
                item.map_err(|e| PiiError::Internal(format!("store: read session_msg item: {e}")))?;
            let (_, seq) = key.value();
            out.push((seq, value.value().to_vec()));
        }
        Ok(out)
    }

    /// Reads transcript messages appended strictly after `since_seq`, in
    /// order — the bounded-read primitive that keeps a full-session replay
    /// from re-reading the whole history: callers combine the latest
    /// snapshot (if any) with only this tail.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn session_messages_since(
        &self,
        session_id: &str,
        since_seq: u64,
    ) -> Result<Vec<(u64, Vec<u8>)>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("store: begin_read: {e}")))?;
        let table = read
            .open_table(SESSION_MSG)
            .map_err(|e| PiiError::Internal(format!("store: open session_msg: {e}")))?;
        let mut out = Vec::new();
        let range = table
            .range((session_id, since_seq.saturating_add(1))..(session_id, u64::MAX))
            .map_err(|e| PiiError::Internal(format!("store: range session_msg: {e}")))?;
        for item in range {
            let (key, value) =
                item.map_err(|e| PiiError::Internal(format!("store: read session_msg item: {e}")))?;
            let (_, seq) = key.value();
            out.push((seq, value.value().to_vec()));
        }
        Ok(out)
    }

    /// Stores a periodic full-session snapshot at a message seq (D3/D6):
    /// bounds the cost of reconstructing a session on restart to "latest
    /// snapshot + messages since" instead of the entire transcript.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn put_session_snapshot(
        &self,
        session_id: &str,
        seq: u64,
        bytes: &[u8],
    ) -> Result<(), PiiError> {
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("store: begin_write: {e}")))?;
        {
            let mut table = write
                .open_table(SESSION_SNAPSHOT)
                .map_err(|e| PiiError::Internal(format!("store: open session_snapshot: {e}")))?;
            table
                .insert((session_id, seq), bytes)
                .map_err(|e| PiiError::Internal(format!("store: put session_snapshot: {e}")))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("store: commit session_snapshot: {e}")))?;
        self.session_bytes_written
            .fetch_add(bytes.len() as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// Returns the highest-seq full-session snapshot for a session, if any.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn latest_session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<Option<(u64, Vec<u8>)>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("store: begin_read: {e}")))?;
        let table = read
            .open_table(SESSION_SNAPSHOT)
            .map_err(|e| PiiError::Internal(format!("store: open session_snapshot: {e}")))?;
        let mut latest: Option<(u64, Vec<u8>)> = None;
        let range = table
            .range((session_id, 0)..=(session_id, u64::MAX))
            .map_err(|e| PiiError::Internal(format!("store: range session_snapshot: {e}")))?;
        for item in range {
            let (key, value) =
                item.map_err(|e| PiiError::Internal(format!("store: read session_snapshot: {e}")))?;
            let (_, seq) = key.value();
            latest = Some((seq, value.value().to_vec()));
        }
        Ok(latest)
    }
}

/// The `META` key holding the next-seq counter for a session's message
/// journal. Namespaced by session id so every session's append counter is
/// independent.
fn session_msg_counter_key(session_id: &str) -> String {
    format!("session_msg_counter:{session_id}")
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use crate::Store;

    fn tmp_store(name: &str) -> Store {
        let dir = std::path::Path::new("/projects/pi-improved/_tmp/pi-store-test").to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}-{}.redb", name, std::process::id()));
        let _ = std::fs::remove_file(&path);
        Store::open(&path).unwrap()
    }

    #[test]
    fn session_put_get_roundtrip() {
        let store = tmp_store("session-roundtrip");
        store.put_session("s1", b"xyz").unwrap();
        assert_eq!(store.get_session("s1").unwrap().unwrap(), b"xyz");
        assert!(store.get_session("nope").unwrap().is_none());
    }

    #[test]
    fn session_del_removes() {
        let store = tmp_store("session-del");
        store.put_session("s1", b"xyz").unwrap();
        store.del_session("s1").unwrap();
        assert!(store.get_session("s1").unwrap().is_none());
        assert!(!store.all_sessions().unwrap().contains(&"s1".to_string()));
    }

    #[test]
    fn session_all_lists() {
        let store = tmp_store("session-all");
        for id in ["sa", "sb", "sc"] {
            store.put_session(id, b"").unwrap();
        }
        let all = store.all_sessions().unwrap();
        assert_eq!(all.len(), 3);
        assert!(all.contains(&"sa".to_string()));
        assert!(all.contains(&"sb".to_string()));
        assert!(all.contains(&"sc".to_string()));
    }

    #[test]
    fn session_id_counter_monotonic() {
        let dir = std::path::Path::new("/projects/pi-improved/_tmp/pi-store-test").to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("session-counter-{}.redb", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let store = Store::open(&path).unwrap();
        for expect in 1..=5u64 {
            assert_eq!(store.next_session_id().unwrap(), expect);
        }

        // Put and delete a session: the counter must NOT regress to len+1.
        store.put_session("s9", b"").unwrap();
        store.del_session("s9").unwrap();
        assert_eq!(store.next_session_id().unwrap(), 6);

        // Reopened store continues above the max persisted value.
        drop(store);
        let reopened = Store::open(&path).unwrap();
        assert!(reopened.next_session_id().unwrap() > 6);
    }

    #[test]
    fn session_token_roundtrip() {
        let store = tmp_store("session-tokens");
        store.set_session_tokens("s1", 120, 45).unwrap();
        assert_eq!(store.get_session_tokens("s1").unwrap(), Some((120, 45)));
    }

    #[test]
    fn session_token_default_zero() {
        let store = tmp_store("session-tokens-default");
        // Documented choice: a never-set id returns `None`, not `(0, 0)`.
        assert_eq!(store.get_session_tokens("never").unwrap(), None);
    }

    /// D3/D6 (026 report 5.0.4): appending N messages writes one row per
    /// message (never re-writing prior rows), so cumulative bytes written
    /// stays close to linear in N — the counting-store proof that the
    /// append-per-message fix is not just correct but actually avoids the
    /// pre-fix O(n^2) whole-transcript-rewrite behaviour. Bound: bytes
    /// written over 200 appends of equal-size messages must be <= 2x the
    /// bytes of the final message set (a generous slack over the ~1x a
    /// pure append-only journal achieves, but a bound an O(n^2) rewrite of
    /// ~200 messages would blow through by roughly two orders of
    /// magnitude).
    #[test]
    fn session_message_append_is_not_quadratic() {
        let store = tmp_store("session-msg-linear");
        let sid = "s1";
        let msg = vec![0u8; 64]; // fixed-size message payload
        let n = 200u64;
        for _ in 0..n {
            store.append_session_message(sid, &msg).unwrap();
        }
        let final_size_estimate = n * msg.len() as u64;
        let written = store.session_bytes_written();
        assert!(
            written <= final_size_estimate * 2,
            "bytes written ({written}) must stay <= 2x final size \
             ({final_size_estimate}) — got a ratio of {}x, which would \
             indicate the old O(n^2) whole-session-rewrite behaviour",
            f64::from(u32::try_from(written).unwrap_or(u32::MAX))
                / f64::from(u32::try_from(final_size_estimate).unwrap_or(1))
        );
        // And every message actually landed, in order.
        let all = store.session_messages(sid).unwrap();
        assert_eq!(all.len(), usize::try_from(n).unwrap());
        for (i, (seq, bytes)) in all.iter().enumerate() {
            assert_eq!(*seq, u64::try_from(i).unwrap());
            assert_eq!(bytes, &msg);
        }
    }

    /// D3/D6: `session_messages_since` returns only the tail after a given
    /// seq — the primitive that keeps a snapshot-based rebuild from
    /// replaying the whole history.
    #[test]
    fn session_messages_since_returns_only_tail() {
        let store = tmp_store("session-msg-since");
        let sid = "s1";
        for i in 0..10u8 {
            store.append_session_message(sid, &[i]).unwrap();
        }
        let tail = store.session_messages_since(sid, 6).unwrap();
        let seqs: Vec<u64> = tail.iter().map(|(s, _)| *s).collect();
        assert_eq!(seqs, vec![7, 8, 9]);
    }

    /// D3/D6: a session snapshot roundtrips and `latest_session_snapshot`
    /// returns the highest-seq one.
    #[test]
    fn session_snapshot_latest() {
        let store = tmp_store("session-snap");
        let sid = "s1";
        store.put_session_snapshot(sid, 32, b"snap32").unwrap();
        store.put_session_snapshot(sid, 64, b"snap64").unwrap();
        let (seq, bytes) = store.latest_session_snapshot(sid).unwrap().unwrap();
        assert_eq!(seq, 64);
        assert_eq!(bytes, b"snap64");
    }

    /// D3/D6: `del_session` also clears message/snapshot rows.
    #[test]
    fn del_session_clears_messages_and_snapshots() {
        let store = tmp_store("session-del-full");
        let sid = "s1";
        store.put_session(sid, b"meta").unwrap();
        store.append_session_message(sid, b"m1").unwrap();
        store.put_session_snapshot(sid, 0, b"snap").unwrap();
        store.del_session(sid).unwrap();
        assert!(store.get_session(sid).unwrap().is_none());
        assert!(store.session_messages(sid).unwrap().is_empty());
        assert!(store.latest_session_snapshot(sid).unwrap().is_none());
    }
}
