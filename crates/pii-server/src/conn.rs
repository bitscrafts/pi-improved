//! Connection loop (F2.6): hello gate, version skew = hard refusal (code
//! 100 → close), method-before-hello = code 101 → close. One writer task so
//! responses and notifications never interleave mid-line.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use pii_proto::method::Method;
use pii_proto::sys::{HelloParams, HelloResult, Limits, PeerInfo};
use pii_proto::{
    Envelope, ErrorCode, Id, PROTOCOL_VERSION, Request, Response, RpcError, parse_frame,
    write_frame,
};

use crate::Engine;
use crate::dispatch;

/// B5 (opus review): a connection with no traffic for this long is dropped
/// so a client that vanishes without a clean close (crash, `kill -9`, a
/// dropped `Client` whose abort hasn't been scheduled yet) does not pin
/// this task + its `BufReader` + its writer task + its mpsc channel
/// forever. Chosen well above any single RPC's expected round-trip so it
/// never fires on a live, idle-but-connected client (e.g. `pii tui`
/// between commands) under any existing test.
const IDLE_TIMEOUT: Duration = Duration::from_mins(5);

/// A live subscription's cancel handle and task.
type SubEntry = (
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
);

/// One connection's live `event.subscribe` tasks (M1, opus review).
///
/// Cancelling a subscription (`event.unsubscribe`, or the connection's own
/// teardown) sends on the stored `oneshot` sender, which the subscriber
/// task races in its `select!` loop and exits on. This is what makes
/// `event.unsubscribe` a real cancellation instead of an ack-only no-op,
/// and what makes connection teardown release every subscriber task.
///
/// It would otherwise keep running until the process exits.
#[derive(Default)]
pub struct SubscriptionRegistry {
    inner: parking_lot::Mutex<HashMap<String, SubEntry>>,
}

impl SubscriptionRegistry {
    /// Number of live subscriptions on this connection (enforces
    /// `dispatch::MAX_SUBSCRIPTIONS_PER_CONN`).
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }

    /// True when no subscriptions are live.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Registers a new subscription's cancel handle and task.
    pub fn insert(
        &self,
        sub_id: String,
        cancel: tokio::sync::oneshot::Sender<()>,
        handle: tokio::task::JoinHandle<()>,
    ) {
        self.inner.lock().insert(sub_id, (cancel, handle));
    }

    /// Cancels one subscription by id. Returns `true` if it existed.
    pub fn cancel(&self, sub_id: &str) -> bool {
        let entry = self.inner.lock().remove(sub_id);
        match entry {
            Some((tx, _handle)) => {
                let _ = tx.send(());
                true
            }
            None => false,
        }
    }

    /// Cancels every live subscription (connection teardown).
    fn cancel_all(&self) {
        let entries: Vec<_> = self.inner.lock().drain().collect();
        for (_, (tx, _handle)) in entries {
            let _ = tx.send(());
        }
    }
}

/// How the connection arrived (F2.6/F2.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerIdentity {
    /// A UDS peer (peercred applies).
    Uds {
        /// Peer uid from `SO_PEERCRED`.
        uid: Option<u32>,
    },
    /// The in-process loopback transport.
    InProcess,
}

/// Serves one connection until EOF or refusal (F2.6).
pub async fn serve_connection<S>(engine: Arc<Engine>, io: S, peer: PeerIdentity)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (r, w) = tokio::io::split(io);
    let (wtx, mut wrx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut w = w;
        while let Some(line) = wrx.recv().await {
            if w.write_all(line.as_bytes()).await.is_err()
                || w.write_all(b"\n").await.is_err()
                || w.flush().await.is_err()
            {
                break;
            }
        }
    });
    let subs = SubscriptionRegistry::default();
    let mut lines = BufReader::new(r).lines();
    let mut helloed = false;
    loop {
        // B5 (opus review): bound how long a connection with no traffic can
        // pin this task. A live client always has *something* pending
        // within IDLE_TIMEOUT (a request or, for `pii tui`-style holders,
        // periodic protocol traffic); a vanished one (crash, `kill -9`,
        // an abort that hasn't unwound yet) now gets reaped instead of
        // parking the reader + writer task + mpsc channel forever.
        let Ok(Ok(Some(next))) = tokio::time::timeout(IDLE_TIMEOUT, lines.next_line()).await else {
            // EOF, read error, or idle timeout all mean the same thing
            // here: stop serving this connection.
            break;
        };
        match parse_frame(&next) {
            Ok(Envelope::Request(req)) => {
                if !helloed && req.method != Method::SysHello {
                    let _ = write_resp(
                        &wtx,
                        &Response::err(
                            req.id,
                            RpcError::new(
                                ErrorCode::Unauthorized,
                                "hello required before any call",
                            ),
                        ),
                    );
                    break; // 101 → close (F1.9)
                }
                if req.method == Method::SysHello {
                    match hello_params(&req).and_then(|p| hello_result(&p)) {
                        Ok(result) => {
                            helloed = true;
                            if write_resp(&wtx, &Response::ok(req.id, result)).is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            let _ = write_resp(&wtx, &Response::err(req.id, e));
                            break; // skew → close (F1.9)
                        }
                    }
                } else {
                    let resp = dispatch::dispatch(&engine, &req, peer, &wtx, &subs);
                    if write_resp(&wtx, &resp).is_err() {
                        break;
                    }
                }
            }
            Ok(Envelope::Notification(n)) => dispatch::notification(&engine, &n),
            Ok(Envelope::Response(_)) => {}
            Err(e) => {
                let _ = write_resp(&wtx, &Response::err(Id::Num(0), e));
            }
        }
    }
    // M1 (opus review): teardown must release every subscriber task this
    // connection ever created, not just the ones the client explicitly
    // unsubscribed — otherwise a crash/idle-timeout/EOF path still leaks.
    subs.cancel_all();
}

fn write_resp(tx: &mpsc::UnboundedSender<String>, resp: &Response) -> Result<(), ()> {
    write_frame(&Envelope::Response(resp.clone()))
        .map(|line| {
            let _ = tx.send(line);
        })
        .map_err(|_| ())
}

fn hello_params(req: &Request) -> Result<HelloParams, RpcError> {
    serde_json::from_value(req.params.clone())
        .map_err(|_| RpcError::new(ErrorCode::InvalidParams, "bad hello params"))
}

fn hello_result(params: &HelloParams) -> Result<serde_json::Value, RpcError> {
    if params.protocol != PROTOCOL_VERSION {
        return Err(RpcError::with_data(
            ErrorCode::ProtocolMismatch,
            "protocol mismatch",
            serde_json::json!({"expected": PROTOCOL_VERSION, "got": params.protocol}),
        ));
    }
    // Encoding negotiation is CUT to a refusal (spec F1.4/part-1 scope): the
    // server answers "json", and a client offering only non-json encodings is
    // refused. An empty list is treated as "no acceptable encoding".
    if !params.encodings.iter().any(|e| e == "json") {
        return Err(RpcError::with_data(
            ErrorCode::InvalidParams,
            "no acceptable encoding (server speaks json)",
            serde_json::json!({"offered": params.encodings}),
        ));
    }
    serde_json::to_value(HelloResult {
        protocol: PROTOCOL_VERSION.to_owned(),
        server: PeerInfo {
            name: String::from("pii"),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            build: String::from("debug"),
        },
        encoding: String::from("json"),
        capabilities: vec![
            String::from("sessions"),
            String::from("runs"),
            String::from("graphs"),
            String::from("events"),
        ],
        limits: Limits {
            max_frame_bytes: 1 << 20,
            max_subscriptions: 32,
        },
    })
    .map_err(|e| RpcError::new(ErrorCode::Internal, format!("hello ser: {e}")))
}
