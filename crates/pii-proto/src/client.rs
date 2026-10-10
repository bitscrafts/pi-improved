//! Protocol client (F1.8), feature-gated on `client`.
//!
//! One reader task demultiplexes responses (by id) from push notifications
//! (`event.subscribe` → [`EventPush`]). Frames are newline-delimited; a line
//! over `max_frame_bytes` is a fatal `ParseError` and closes the connection.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use pii_observe::{Event, Level, NullObserver, Observer, names};
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
    /// The peer closed the connection before answering the hello.
    #[error("{endpoint}: connection closed before the hello was answered")]
    Closed {
        /// The endpoint that was dialled.
        endpoint: String,
    },
    /// The peer did not answer the hello within the handshake bound.
    #[error("{endpoint}: no hello answer within {bound_ms}ms")]
    Timeout {
        /// The endpoint that was dialled.
        endpoint: String,
        /// The bound that expired, in milliseconds.
        bound_ms: u64,
    },
}

/// Default bound on the whole handshake (spec 109 R1). Override per call with
/// [`ConnectOptions::bound`].
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The message every "the connection is gone" [`RpcError`] carries.
const CLOSED_MSG: &str = "connection closed";

/// How [`Client::connect_with`] reaches and reports on its peer.
pub struct ConnectOptions<'a> {
    /// Names the peer in errors and events (a socket path).
    pub endpoint: &'a str,
    /// Bound on the whole handshake.
    pub bound: Duration,
    /// Receives the transport-failure events ([`names::PROTO_HANDSHAKE_TIMEOUT`] and friends).
    pub observer: &'a dyn Observer,
}

impl<'a> ConnectOptions<'a> {
    /// The default bound, reporting to nobody.
    #[must_use]
    pub const fn new(endpoint: &'a str) -> Self {
        Self {
            endpoint,
            bound: DEFAULT_CONNECT_TIMEOUT,
            observer: &NullObserver,
        }
    }
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

fn closed_error() -> RpcError {
    RpcError::new(ErrorCode::Internal, CLOSED_MSG)
}

/// A newline-JSON-RPC client over any async byte pair.
pub struct Client {
    next_id: AtomicU64,
    pending: PendingMap,
    /// Set (under the `pending` lock) once the reader hit EOF: a call made
    /// after that point must fail at once, not wait for an answer that cannot come.
    closed: Arc<AtomicBool>,
    writer: mpsc::UnboundedSender<String>,
    events_tx: tokio::sync::broadcast::Sender<EventPush>,
    /// Cap on inbound lines (fatal when exceeded).
    pub max_frame_bytes: u64,
    reader_h: tokio::task::JoinHandle<()>,
    writer_h: tokio::task::JoinHandle<()>,
}

impl Client {
    /// [`Client::connect_with`] with the default bound, an unlabelled
    /// endpoint and no observer.
    ///
    /// # Errors
    /// As [`Client::connect_with`].
    pub async fn connect<R, W>(
        r: R,
        w: W,
        client_info: PeerInfo,
    ) -> Result<(Self, HelloResult), ClientError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        Self::connect_with(r, w, client_info, &ConnectOptions::new("unlabelled peer")).await
    }

    /// Performs `sys.hello` within `opts.bound` and fails hard on protocol
    /// skew (F1.8/F1.9). Every failure is also emitted to `opts.observer`.
    ///
    /// # Errors
    /// Returns [`ClientError::Timeout`] when the peer stays silent past the
    /// bound, [`ClientError::Closed`] when it hangs up unanswered,
    /// [`ClientError::ProtocolMismatch`] when the server's
    /// `result.protocol != PROTOCOL_VERSION`, [`ClientError::Rpc`] for other
    /// refusals, [`ClientError::Io`] on transport failure.
    pub async fn connect_with<R, W>(
        r: R,
        w: W,
        client_info: PeerInfo,
        opts: &ConnectOptions<'_>,
    ) -> Result<(Self, HelloResult), ClientError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let bounded = tokio::time::timeout(
            opts.bound,
            Self::handshake(r, w, client_info, opts.endpoint),
        );
        let out = bounded.await.unwrap_or_else(|_| {
            Err(ClientError::Timeout {
                endpoint: opts.endpoint.to_owned(),
                bound_ms: u64::try_from(opts.bound.as_millis()).unwrap_or(u64::MAX),
            })
        });
        if let Err(e) = &out {
            report(opts, e);
        }
        out
    }

    async fn handshake<R, W>(
        r: R,
        w: W,
        client_info: PeerInfo,
        endpoint: &str,
    ) -> Result<(Self, HelloResult), ClientError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (wtx, wrx) = mpsc::unbounded_channel::<String>();
        let writer_h = tokio::spawn(writer_task(w, wrx));
        let pending: PendingMap = Arc::new(Mutex::new(BTreeMap::new()));
        let closed = Arc::new(AtomicBool::new(false));
        let (etx, _) = tokio::sync::broadcast::channel::<EventPush>(1024);
        let reader_h = tokio::spawn(reader_task(
            r,
            Arc::clone(&pending),
            Arc::clone(&closed),
            etx.clone(),
            1u64 << 20,
        ));
        let mut client = Self {
            next_id: AtomicU64::new(0),
            pending,
            closed,
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
        let result = client.call(Method::SysHello, params).await.map_err(|e| {
            match ClientError::from(e) {
                ClientError::Rpc { message, .. } if message == CLOSED_MSG => ClientError::Closed {
                    endpoint: endpoint.to_owned(),
                },
                other => other,
            }
        })?;
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
        {
            let mut pending = self.pending.lock().await;
            if self.closed.load(Ordering::Acquire) {
                return Err(closed_error());
            }
            pending.insert(id_key(&id), tx);
        }
        self.writer
            .send(line)
            .map_err(|_| RpcError::new(ErrorCode::Internal, "writer closed"))?;
        rx.recv().await.unwrap_or_else(|| Err(closed_error()))
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

/// Emits the transport-failure event for `e` (spec 109 R7).
fn report(opts: &ConnectOptions<'_>, e: &ClientError) {
    let ev = match e {
        ClientError::Timeout { bound_ms, .. } => {
            Event::new("proto", Level::Error, names::PROTO_HANDSHAKE_TIMEOUT)
                .field("bound_ms", *bound_ms)
        }
        ClientError::Closed { .. } => {
            Event::new("proto", Level::Error, names::PROTO_HANDSHAKE_CLOSED)
        }
        ClientError::ProtocolMismatch { got } => {
            Event::new("proto", Level::Error, names::PROTO_HANDSHAKE_SKEW)
                .field("got", got.as_str())
                .field("want", PROTOCOL_VERSION)
        }
        ClientError::Rpc { .. } | ClientError::Io(_) => {
            Event::new("proto", Level::Error, names::PROTO_HANDSHAKE_FAILED)
                .field("error", e.to_string())
        }
    };
    opts.observer.event(&ev.field("endpoint", opts.endpoint));
}

async fn reader_task<R: AsyncRead + Unpin + Send + 'static>(
    r: R,
    pending: PendingMap,
    closed: Arc<AtomicBool>,
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
    closed.store(true, Ordering::Release);
    let ids: Vec<String> = map.keys().cloned().collect();
    for id in ids {
        if let Some(tx) = map.remove(&id) {
            let _ = tx.send(Err(closed_error()));
        }
    }
}
