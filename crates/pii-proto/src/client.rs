//! Protocol client (F1.8), feature-gated on `client`.
//!
//! One reader task demultiplexes responses (by id) from push notifications
//! (`event.subscribe` → [`EventPush`]). Frames are newline-delimited; a line
//! over `max_frame_bytes` is a fatal `ParseError` and closes the connection.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc};

use crate::event::EventPush;
use crate::method::Method;
use crate::sys::{HelloParams, HelloResult, PeerInfo};
use crate::{Envelope, ErrorCode, Id, PROTOCOL_VERSION, Request, RpcError, parse_frame};

/// Client-side failure modes.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The server refused our protocol version (code 100).
    #[error("protocol mismatch: server speaks {got}")]
    ProtocolMismatch {
        /// What the server said it speaks.
        got: String,
    },
    /// The server answered with an RPC error.
    #[error("rpc error {code}: {message}")]
    Rpc {
        /// Numeric code.
        code: i32,
        /// Message.
        message: String,
    },
    /// I/O failure.
    #[error("io: {0}")]
    Io(String),
    /// The connection closed before an answer.
    #[error("connection closed")]
    Closed,
}

impl From<RpcError> for ClientError {
    fn from(e: RpcError) -> Self {
        if e.code == i32::from(ErrorCode::ProtocolMismatch) {
            let got = e
                .data
                .as_ref()
                .and_then(|d| d.get("got"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            Self::ProtocolMismatch { got }
        } else {
            Self::Rpc {
                code: e.code,
                message: e.message,
            }
        }
    }
}

type PendingMap =
    Arc<Mutex<BTreeMap<String, mpsc::UnboundedSender<Result<serde_json::Value, RpcError>>>>>;

/// A newline-JSON-RPC client over any async byte pair.
pub struct Client {
    next_id: AtomicU64,
    pending: PendingMap,
    writer: mpsc::UnboundedSender<String>,
    events_tx: tokio::sync::broadcast::Sender<EventPush>,
    /// Cap on inbound lines (fatal when exceeded).
    pub max_frame_bytes: u64,
    reader_h: tokio::task::JoinHandle<()>,
    writer_h: tokio::task::JoinHandle<()>,
}

impl Client {
    /// Performs `sys.hello` and fails hard on protocol skew (F1.8/F1.9).
    ///
    /// # Errors
    /// Returns [`ClientError::ProtocolMismatch`] when the server's
    /// `result.protocol != PROTOCOL_VERSION`, [`ClientError::Rpc`] for other
    /// refusals, [`ClientError::Io`]/[`ClientError::Closed`] on transport
    /// failure.
    pub async fn connect<R, W>(
        r: R,
        w: W,
        client_info: PeerInfo,
    ) -> Result<(Self, HelloResult), ClientError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (wtx, wrx) = mpsc::unbounded_channel::<String>();
        let writer_h = tokio::spawn(writer_task(w, wrx));
        let pending: PendingMap = Arc::new(Mutex::new(BTreeMap::new()));
        let (etx, _) = tokio::sync::broadcast::channel::<EventPush>(1024);
        let reader_h = tokio::spawn(reader_task(
            r,
            Arc::clone(&pending),
            etx.clone(),
            1u64 << 20,
        ));
        let mut client = Self {
            next_id: AtomicU64::new(0),
            pending,
            writer: wtx,
            events_tx: etx,
            max_frame_bytes: 1 << 20,
            reader_h,
            writer_h,
        };
        let params = serde_json::to_value(HelloParams {
            protocol: PROTOCOL_VERSION.to_owned(),
            client: client_info,
            encodings: vec!["json".into()],
            features: Vec::new(),
        })
        .map_err(|e| ClientError::Io(format!("hello serde: {e}")))?;
        let result = client.call(Method::SysHello, params).await?;
        let hello: HelloResult = serde_json::from_value(result)
            .map_err(|e| ClientError::Io(format!("hello result: {e}")))?;
        if hello_result_protocol(&hello) != PROTOCOL_VERSION {
            return Err(ClientError::ProtocolMismatch {
                got: hello_result_protocol(&hello).to_owned(),
            });
        }
        client.max_frame_bytes = hello.limits.max_frame_bytes;
        Ok((client, hello))
    }

    /// Calls a method and awaits its response.
    ///
    /// # Errors
    /// Returns [`RpcError`] from the server or transport failure.
    pub async fn call(
        &self,
        method: Method,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RpcError> {
        let n = self.next_id.fetch_add(1, Ordering::Relaxed);
        let id = Id::Num(n);
        let req = Request {
            jsonrpc: "2.0".into(),
            id: id.clone(),
            method,
            params,
        };
        let line = serde_json::to_string(&Envelope::Request(req))
            .map_err(|e| RpcError::new(ErrorCode::Internal, format!("ser: {e}")))?;
        let (tx, mut rx) = mpsc::unbounded_channel();
        self.pending.lock().await.insert(id_key(&id), tx);
        self.writer
            .send(line)
            .map_err(|_| RpcError::new(ErrorCode::Internal, "writer closed"))?;
        rx.recv()
            .await
            .unwrap_or_else(|| Err(RpcError::new(ErrorCode::Internal, "connection closed")))
    }

    /// Subscribes to push notifications. Multiple subscribers allowed.
    pub fn events(&self) -> tokio::sync::broadcast::Receiver<EventPush> {
        self.events_tx.subscribe()
    }

    /// Sends a raw line (internal helper for tests/notify).
    ///
    /// # Errors
    /// Fails when the writer task is gone.
    pub fn send_line(&self, line: String) -> Result<(), RpcError> {
        self.writer
            .send(line)
            .map_err(|_| RpcError::new(ErrorCode::Internal, "writer closed"))
    }

    /// Closes the connection (B5): aborts both halves' tasks so the split
    /// reader no longer parks on the socket and the server sees EOF.
    ///
    /// `tokio::io::split` only closes the underlying fd when BOTH halves
    /// drop; dropping a [`Client`] without calling this leaves the reader
    /// task parked forever and pins the server's per-connection state
    /// (the rss-gate leak). Call this on every one-shot path.
    pub fn shutdown(&self) {
        self.reader_h.abort();
        self.writer_h.abort();
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // Best-effort backstop: a client dropped without `shutdown()` must
        // still release the socket so the server connection is not pinned.
        self.reader_h.abort();
        self.writer_h.abort();
    }
}

// `events_tx` is shared between the reader task (push) and `events()`
// (subscribe); one broadcast channel serves all subscribers.

fn hello_result_protocol(h: &HelloResult) -> &str {
    &h.protocol
}

fn id_key(id: &Id) -> String {
    match id {
        Id::Num(n) => n.to_string(),
        Id::Str(s) => s.clone(),
    }
}

async fn writer_task<W: AsyncWrite + Unpin + Send + 'static>(
    mut w: W,
    mut rx: mpsc::UnboundedReceiver<String>,
) {
    while let Some(line) = rx.recv().await {
        if w.write_all(line.as_bytes()).await.is_err()
            || w.write_all(b"\n").await.is_err()
            || w.flush().await.is_err()
        {
            break;
        }
    }
}

async fn reader_task<R: AsyncRead + Unpin + Send + 'static>(
    r: R,
    pending: PendingMap,
    events_tx: tokio::sync::broadcast::Sender<EventPush>,
    max_frame_bytes: u64,
) {
    let mut lines = BufReader::new(r).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.len() as u64 > max_frame_bytes {
            break; // fatal ParseError: close the connection (F1.8)
        }
        match parse_frame(&line) {
            Ok(Envelope::Response(resp)) => {
                let removed = pending.lock().await.remove(&id_key(&resp.id));
                if let Some(tx) = removed {
                    let out = match (resp.result, resp.error) {
                        (Some(v), _) => Ok(v),
                        (None, Some(e)) => Err(e),
                        (None, None) => Err(RpcError::new(
                            ErrorCode::Internal,
                            "response without result or error",
                        )),
                    };
                    let _ = tx.send(out);
                }
            }
            Ok(Envelope::Notification(n)) if n.method == Method::EventSubscribe => {
                if let Ok(push) = serde_json::from_value::<EventPush>(n.params) {
                    let _ = events_tx.send(push);
                }
            }
            _ => {}
        }
    }
    // EOF: fail every pending call so callers do not hang.
    let mut map = pending.lock().await;
    let ids: Vec<String> = map.keys().cloned().collect();
    for id in ids {
        if let Some(tx) = map.remove(&id) {
            let _ = tx.send(Err(RpcError::new(ErrorCode::Internal, "connection closed")));
        }
    }
}
