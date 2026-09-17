//! Deterministic, append-only audit log (spec 38).
//!
//! Lines are `FEED: <event> <detail>`; content is a pure function of the
//! request inputs and outcome — no timestamps, no hostnames, no
//! nondeterministic ids — so a duplicate input yields identical log lines.

use parking_lot::Mutex;
use pii_abi::PiiError;
use std::path::PathBuf;
use std::sync::Arc;

/// Where audit lines go. A file path for production; a shared in-memory
/// buffer for tests.
#[derive(Clone)]
pub enum AuditSink {
    /// Appends to a file (created on first write).
    File(PathBuf),
    /// Appends to a shared in-memory buffer.
    Memory(Arc<Mutex<Vec<String>>>),
}

impl AuditSink {
    fn append(&self, line: &str) -> Result<(), PiiError> {
        let record = format!("{line}\n");
        match self {
            Self::File(path) => {
                use std::io::Write as _;
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(|e| PiiError::Internal(format!("audit {}: {e}", path.display())))?;
                f.write_all(record.as_bytes())
                    .map_err(|e| PiiError::Internal(format!("audit write: {e}")))
            }
            Self::Memory(buf) => {
                buf.lock().push(String::from(line));
                Ok(())
            }
        }
    }
}

/// The fetch audit log.
#[derive(Clone)]
pub struct AuditLog {
    sink: AuditSink,
}

impl AuditLog {
    /// An audit log writing to a file.
    #[must_use]
    pub fn to_file(path: PathBuf) -> Self {
        Self {
            sink: AuditSink::File(path),
        }
    }

    /// An audit log kept in memory (tests).
    #[must_use]
    pub fn in_memory() -> (Self, Arc<Mutex<Vec<String>>>) {
        let buf = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                sink: AuditSink::Memory(Arc::clone(&buf)),
            },
            buf,
        )
    }

    /// Appends one deterministic audit line.
    ///
    /// # Errors
    /// [`PiiError::Internal`] when the sink cannot be written.
    pub fn append(&self, line: &str) -> Result<(), PiiError> {
        self.sink.append(line)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::AuditLog;

    /// Determinism: two identical appends produce identical bytes.
    #[test]
    fn audit_lines_are_deterministic() {
        let (log, buf) = AuditLog::in_memory();
        log.append(
            "FEED: GET http://x/ -> 200-OK status=200 bytes_in=3 headers=none method=GET body=none",
        )
        .unwrap();
        log.append(
            "FEED: GET http://x/ -> 200-OK status=200 bytes_in=3 headers=none method=GET body=none",
        )
        .unwrap();
        assert_eq!(buf.lock().len(), 2);
        let (first, second) = {
            let buf = buf.lock();
            (buf.first().cloned(), buf.get(1).cloned())
        };
        assert_eq!(first, second);
    }
}
