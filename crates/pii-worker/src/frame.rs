//! Worker frames (F3.2): newline-delimited JSON over the child's stdio.
//! Total parsing — a malformed frame is an `Err`, never a panic.

use pii_abi::PiiError;
use serde::{Deserialize, Serialize};

/// Max frame length in bytes (F3.2: 8 MiB; longer is fatal for the worker).
pub const MAX_FRAME: usize = 8 * 1024 * 1024;

/// Worker protocol version.
pub const WORKER_PROTOCOL: &str = "pii.worker.v1";

/// One wire frame (closed, tagged).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkerFrame {
    /// A supervisor → worker request.
    Req {
        /// Request id.
        id: u64,
        /// Operation name (`hello|invoke|info|shutdown`).
        op: String,
        /// Arguments.
        #[serde(default)]
        args: serde_json::Value,
    },
    /// A worker → supervisor response.
    Res {
        /// Request id.
        id: u64,
        /// Success payload.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ok: Option<serde_json::Value>,
        /// Failure payload.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        err: Option<WorkerErr>,
    },
    /// A worker log note (also used for stderr drain lines).
    Note {
        /// Log level.
        level: String,
        /// Message.
        msg: String,
    },
}

/// A worker-side failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerErr {
    /// Machine-readable code.
    pub code: String,
    /// Human-readable message.
    pub message: String,
}

/// The worker's `hello` payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// Worker name.
    pub name: String,
    /// Build tag.
    pub tag: String,
    /// OS pid.
    pub pid: u32,
    /// Protocol version (must be [`WORKER_PROTOCOL`]).
    pub protocol: String,
}

/// Parses one newline-stripped frame.
///
/// # Errors
/// [`PiiError::Invalid`] for undecodable JSON or an unknown `kind`.
pub fn parse_frame(line: &str) -> Result<WorkerFrame, PiiError> {
    serde_json::from_str(line).map_err(|e| PiiError::Invalid(format!("worker frame: {e}")))
}

/// Serialises one frame to a newline-terminated line.
///
/// # Errors
/// [`PiiError::Internal`] on serialisation failure.
pub fn write_frame(frame: &WorkerFrame) -> Result<Vec<u8>, PiiError> {
    let mut line = serde_json::to_vec(frame)
        .map_err(|e| PiiError::Internal(format!("worker frame ser: {e}")))?;
    if line.len() > MAX_FRAME {
        return Err(PiiError::Invalid(format!(
            "worker frame exceeds {MAX_FRAME} bytes"
        )));
    }
    line.push(b'\n');
    Ok(line)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{WorkerFrame, parse_frame, write_frame};

    #[test]
    fn frames_roundtrip() {
        let req = WorkerFrame::Req {
            id: 7,
            op: String::from("invoke"),
            args: serde_json::json!({"v": 1}),
        };
        let back = parse_frame(
            std::str::from_utf8(&write_frame(&req).unwrap())
                .unwrap()
                .trim_end_matches('\n'),
        )
        .unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn malformed_is_err_not_panic() {
        assert!(parse_frame("{not json").is_err());
        assert!(parse_frame(r#"{"kind":"bogus"}"#).is_err());
    }
}
