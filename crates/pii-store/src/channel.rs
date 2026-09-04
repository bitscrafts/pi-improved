//! Channel cursor (F6.5): `channel_cursor: channel -> encoded {cursor,run}`.
//!
//! Advance writes cursor + run in ONE transaction so a crash cannot
//! duplicate or lose a message at the granularity of "run started".

use pii_abi::PiiError;
use redb::{Database, ReadableDatabase};
use serde::{Deserialize, Serialize};

use crate::codec::{from_bytes, to_bytes};
use crate::tables::CHANNEL_CURSOR;

/// Cursor state for one channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelCursor {
    /// Last processed message id/cursor.
    pub cursor: u64,
    /// Run started for the last processed message.
    pub run_id: String,
    /// How many API errors this channel has absorbed (F6.3).
    #[serde(default)]
    pub errors: u64,
}

/// The channel-cursor table.
pub struct ChannelStore {
    db: Database,
}

impl ChannelStore {
    /// Opens (creating if needed) the channel store on `db`.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn open(path: &std::path::Path) -> Result<Self, PiiError> {
        let db = Database::create(path)
            .map_err(|e| PiiError::Internal(format!("channel: open {}: {e}", path.display())))?;
        {
            let write = db
                .begin_write()
                .map_err(|e| PiiError::Internal(format!("channel: begin_write: {e}")))?;
            write
                .open_table(CHANNEL_CURSOR)
                .map_err(|e| PiiError::Internal(format!("channel: open table: {e}")))?;
            write
                .commit()
                .map_err(|e| PiiError::Internal(format!("channel: commit tables: {e}")))?;
        }
        Ok(Self { db })
    }

    /// Reads a channel's cursor (`None` before the first message).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn get(&self, channel: &str) -> Result<Option<ChannelCursor>, PiiError> {
        let read = self
            .db
            .begin_read()
            .map_err(|e| PiiError::Internal(format!("channel: begin_read: {e}")))?;
        let t = read
            .open_table(CHANNEL_CURSOR)
            .map_err(|e| PiiError::Internal(format!("channel: open table: {e}")))?;
        let Some(v) = t
            .get(channel)
            .map_err(|e| PiiError::Internal(format!("channel: get: {e}")))?
        else {
            return Ok(None);
        };
        Ok(Some(from_bytes(v.value())?))
    }

    /// Advances cursor + run in ONE transaction (F6.5).
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb or encode failure.
    pub fn advance(&self, channel: &str, cursor: u64, run_id: &str) -> Result<(), PiiError> {
        let cur = self.get(channel)?.unwrap_or(ChannelCursor {
            cursor: 0,
            run_id: String::new(),
            errors: 0,
        });
        let next = ChannelCursor {
            cursor,
            run_id: String::from(run_id),
            errors: cur.errors,
        };
        let bytes = to_bytes(&next)?;
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("channel: begin_write: {e}")))?;
        {
            let mut t = write
                .open_table(CHANNEL_CURSOR)
                .map_err(|e| PiiError::Internal(format!("channel: open table: {e}")))?;
            t.insert(channel, bytes.as_slice())
                .map_err(|e| PiiError::Internal(format!("channel: insert: {e}")))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("channel: commit advance: {e}")))
    }

    /// Bumps the channel's API-error counter (F6.3), preserving the cursor.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn record_error(&self, channel: &str) -> Result<u64, PiiError> {
        let cur = self.get(channel)?.unwrap_or(ChannelCursor {
            cursor: 0,
            run_id: String::new(),
            errors: 0,
        });
        let errors = cur.errors.saturating_add(1);
        let next = ChannelCursor { errors, ..cur };
        let bytes = to_bytes(&next)?;
        let write = self
            .db
            .begin_write()
            .map_err(|e| PiiError::Internal(format!("channel: begin_write: {e}")))?;
        {
            let mut t = write
                .open_table(CHANNEL_CURSOR)
                .map_err(|e| PiiError::Internal(format!("channel: open table: {e}")))?;
            t.insert(channel, bytes.as_slice())
                .map_err(|e| PiiError::Internal(format!("channel: insert: {e}")))?;
        }
        write
            .commit()
            .map_err(|e| PiiError::Internal(format!("channel: commit error: {e}")))?;
        Ok(errors)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::ChannelStore;

    #[test]
    fn cursor_advances_exactly_once() {
        let dir = std::path::Path::new("/projects/pi-improved/_tmp/phase3/channeltest");
        let _ = std::fs::create_dir_all(dir);
        let path = dir.join(format!("ch-{}.redb", std::process::id()));
        let s = ChannelStore::open(&path).unwrap();
        assert!(s.get("discord").unwrap().is_none());
        s.advance("discord", 41, "r1").unwrap();
        s.advance("discord", 42, "r2").unwrap();
        let c = s.get("discord").unwrap().unwrap();
        assert_eq!(c.cursor, 42);
        assert_eq!(c.run_id, "r2");
        assert_eq!(c.errors, 0);
        assert_eq!(s.record_error("discord").unwrap(), 1);
        let c = s.get("discord").unwrap().unwrap();
        assert_eq!(c.errors, 1, "errors counter survives an advance");
        assert_eq!(c.cursor, 42, "advance is not rolled back");
        let _ = std::fs::remove_file(&path);
    }
}
