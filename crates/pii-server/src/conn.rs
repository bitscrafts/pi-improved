//! Connection loop (F2.6): hello gate, version skew = hard refusal (code
//! 100 → close), method-before-hello = code 101 → close. One writer task so
//! responses and notifications never interleave mid-line.

use std::sync::Arc;

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
    let mut lines = BufReader::new(r).lines();
    let mut helloed = false;
    while let Ok(Some(line)) = lines.next_line().await {
        match parse_frame(&line) {
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
                    let resp = dispatch::dispatch(&engine, &req, peer, &wtx);
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
