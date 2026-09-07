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

use crate::tables::{META, SESSION, SESSION_TOKENS};

const SESSION_ID_COUNTER_KEY: &str = "session_id_counter";

impl crate::Store {
    /// Stores a session blob by id (upsert).
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on a redb failure.
    pub fn put_session(&self, id: &str, bytes: &[u8]) -> Result<(), PiiError> {
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

    /// Deletes a session blob by id. Idempotent (no error if absent).
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
}
