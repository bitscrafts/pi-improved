//! `CliChannel` (F6.4): newline-delimited JSON on two files, no I/O beyond
//! them. Used by tests and `pii -p` style scripting.

use std::path::PathBuf;

use pii_abi::PiiError;
use pii_core::Ctx;

use crate::{Channel, Incoming};

/// A file-backed channel: reads `Incoming` lines from `inbox`, appends reply
/// lines to `outbox` (F6.4).
pub struct CliChannel {
    name: String,
    inbox: PathBuf,
    outbox: PathBuf,
}

impl CliChannel {
    /// Builds a channel bound to the two given files.
    #[must_use]
    pub fn new(name: impl Into<String>, inbox: PathBuf, outbox: PathBuf) -> Self {
        Self {
            name: name.into(),
            inbox,
            outbox,
        }
    }
}

impl Channel for CliChannel {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> &'static str {
        "cli"
    }

    fn poll(&self, ctx: &Ctx, cursor: Option<u64>) -> Result<Vec<Incoming>, PiiError> {
        if !self.inbox.exists() {
            return Ok(Vec::new());
        }
        let bytes = ctx.fs()?.read(&self.inbox)?;
        let text = String::from_utf8_lossy(&bytes);
        let mut out = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let v: serde_json::Value = serde_json::from_str(line)
                .map_err(|e| PiiError::Unavailable(format!("cli channel: bad line: {e}")))?;
            let msg_cursor = v
                .get("cursor")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            if let Some(c) = cursor
                && msg_cursor <= c
            {
                continue;
            }
            out.push(Incoming {
                channel: self.name.clone(),
                chat: str_field(&v, "chat"),
                user: str_field(&v, "user"),
                text: str_field(&v, "text"),
                cursor: msg_cursor,
            });
        }
        Ok(out)
    }

    fn send(&self, ctx: &Ctx, to: &str, text: &str) -> Result<(), PiiError> {
        let mut line = serde_json::to_vec(&serde_json::json!({ "to": to, "text": text }))
            .map_err(|e| PiiError::Internal(format!("cli channel: encode: {e}")))?;
        line.push(b'\n');
        ctx.fs()?.append(&self.outbox, &line)
    }
}

fn str_field(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::path::PathBuf;

    use pii_core::{Budget, CapKind, CapSet, Ctx, PathScope};

    use super::CliChannel;
    use crate::Channel;

    fn scratch() -> PathBuf {
        let dir = std::path::Path::new("/projects/pi-improved/_tmp/phase3/clichannel");
        let _ = std::fs::create_dir_all(dir);
        dir.join(format!("t-{}", std::process::id()))
    }

    fn ctx(root: &std::path::Path) -> Ctx {
        let mut caps = CapSet::empty();
        caps.kinds = CapKind::FS_READ | CapKind::FS_WRITE;
        caps.fs_read = PathScope::new(vec![root.to_path_buf()]);
        caps.fs_write = PathScope::new(vec![root.to_path_buf()]);
        Ctx::root(caps, Budget::unlimited())
    }

    #[test]
    fn poll_reads_new_lines_and_send_appends() {
        let dir = scratch();
        std::fs::create_dir_all(&dir).unwrap();
        let inbox = dir.join("in.jsonl");
        let outbox = dir.join("out.jsonl");
        std::fs::write(
            &inbox,
            "{\"chat\":\"c1\",\"user\":\"u\",\"text\":\"hi\",\"cursor\":1}\n",
        )
        .unwrap();
        let ch = CliChannel::new("cli1", inbox, outbox.clone());
        let c = ctx(&dir);
        let got = ch.poll(&c, None).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got.first().unwrap().text, "hi");
        assert!(ch.poll(&c, Some(1)).unwrap().is_empty(), "no new messages");
        ch.send(&c, "c1", "reply").unwrap();
        let out = std::fs::read_to_string(&outbox).unwrap();
        assert!(out.contains("reply"));
    }
}
