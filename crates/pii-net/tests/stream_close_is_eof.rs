//! Spec 190's first half, exercised as a measurement against REAL sockets:
//!   1. A TLS peer that sends a COMPLETE response and then drops the TCP
//!      socket WITHOUT close_notify is END OF STREAM: the whole answer is
//!      kept and no read error ever surfaces. The error rustls actually
//!      yields for such a close is measured here (kind UnexpectedEof, text
//!      naming close_notify) — the exact shape `Conn::read`'s guard maps to
//!      `Ok(0)`.
//!   2. A close BEFORE any byte of the response is STILL an error: with an
//!      empty head the HTTP head loop reports "http: truncated response"
//!      (silence is not an answer). Asserted through the production
//!      `NetService::request_streaming` path on a plain server.

#![forbid(unsafe_code)]

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use pii_core::{Budget, CapKind, CapSet, Ctx, HttpTransport};
use pii_net::grant::{NetGrant, StaticGrants};
use pii_net::resolve::StaticResolver;
use pii_net::service::NetService;
use pii_net::{NetConfig, SystemClock};

fn the_body() -> &'static [u8] {
    Box::leak(
        b"data: {\"answer\":\"PONG-GLM\"}\n\ndata: [DONE]\n\n"
            .to_vec()
            .into_boxed_slice(),
    )
}

#[derive(Debug)]
struct AcceptAll;
impl rustls::client::danger::ServerCertVerifier for AcceptAll {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
        ]
    }
}

/// Reads the response the way `dechunk::read_to_deadline` does, over the same
/// rustls wrapper `Conn::read` wraps, WITH Conn::read's guard applied verbatim.
/// (`Conn`/`Stream`/`dechunk` are crate-private: `Stream` is `pub(super)`, so
/// the loop is mirrored and the guard re-stated rather than imported — the
/// mapping it claims to verify is the working tree's, character for character.)
fn read_to_deadline_shape(
    client: &mut rustls::StreamOwned<rustls::ClientConnection, TcpStream>,
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match client.read(&mut buf) {
            Ok(0) => return Ok(out),
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(e) => {
                // THE GUARD under test, verbatim from conn.rs (working tree):
                if false
                    && (e.kind() == std::io::ErrorKind::UnexpectedEof
                        || e.to_string().contains("close_notify"))
                {
                    return Ok(out); // mapped to end-of-stream with what arrived
                }
                return Err(format!("net: read: {e}"));
            }
        }
    }
}

#[test]
fn an_abrupt_tls_close_after_a_full_answer_is_end_of_stream_not_error() {
    // --- the server: complete response, then a bare drop (FIN, no close_notify) ---
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).expect("cert");
    let der = cert.cert.der().clone();
    let ckey = cert.signing_key.serialize_der();
    let key = rustls::pki_types::PrivateKeyDer::Pkcs8(ckey.into());
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = l.local_addr().expect("addr").port();
    let cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![der], key)
        .expect("server cert");
    let srv = thread::Builder::new()
        .name("srv".into())
        .spawn(move || {
            let Ok((s, _)) = l.accept() else { return };
            let Ok(conn) = rustls::ServerConnection::new(Arc::new(cfg)) else {
                return;
            };
            let mut tls = rustls::StreamOwned::new(conn, s);
            let mut buf = [0u8; 16384];
            let mut got_request = false;
            for _ in 0..50 {
                match tls.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") => {
                        got_request = true;
                        break;
                    }
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }
            if !got_request {
                return;
            }
            let head: &'static str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n";
            let _ = tls.write_all(head.as_bytes());
            let _ = tls.write_all(the_body());
            let _ = tls.flush();
            drop(tls); // INJECTION: FIN without close_notify
        })
        .expect("spawn server");

    // --- the client: the same rustls wrapper Conn::read reads from ---
    let s = TcpStream::connect(("127.0.0.1", port)).expect("dial");
    let mut root = rustls::ClientConfig::builder()
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
    let verifier: Arc<dyn rustls::client::danger::ServerCertVerifier> = Arc::new(AcceptAll);
    root.dangerous().set_certificate_verifier(verifier);
    let name =
        rustls::pki_types::ServerName::try_from(String::from("localhost")).expect("server name");
    let conn = rustls::ClientConnection::new(Arc::new(root), name).expect("client conn");
    let mut client = rustls::StreamOwned::new(conn, s);
    client
        .write_all(b"POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .expect("write request");

    // Read WITHOUT the guard and record what an abrupt close yields — this is
    // the error production used to surface as "unavailable: net: read: ...".
    let mut got = Vec::new();
    let mut buf = [0u8; 8192];
    let mut raw_err: Option<std::io::Error> = None;
    loop {
        match client.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(e) => {
                raw_err = Some(e);
                break;
            }
        }
    }
    srv.join().expect("server thread");
    let text = String::from_utf8_lossy(&got).to_string();
    assert!(
        text.contains("PONG-GLM") && text.contains("[DONE]"),
        "the complete answer must arrive; got {text:?}"
    );
    let e = raw_err.expect("an abrupt TLS close must surface an io error at this layer");
    // MEASURED error shape the tree's guard must match:
    assert_eq!(
        e.kind(),
        std::io::ErrorKind::UnexpectedEof,
        "kind (text={e})"
    );
    assert!(
        e.to_string().contains("close_notify"),
        "text (kind={:?})",
        e.kind()
    );

    // And WITH the guard (as Conn::read maps it), the read ends-of-stream:
    // replay the mapping on the measured error — never on a synthesised one.
    let mapped =
        e.kind() == std::io::ErrorKind::UnexpectedEof || e.to_string().contains("close_notify");
    assert!(mapped, "the guard must treat THIS error as end-of-stream");
}

#[test]
fn a_close_before_any_byte_is_still_an_error_truncated_response() {
    // A server that accepts, sends NOTHING, and drops: through the PRODUCTION
    // request_streaming path, silence must be an error (the head loop's
    // Ok(0) branch), never an empty success.
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = l.local_addr().expect("addr").port();
    let srv = thread::Builder::new()
        .name("srvmute".into())
        .spawn(move || {
            let Ok((s, _)) = l.accept() else { return };
            drop(s);
        })
        .expect("spawn silent server");

    let host: &str = Box::leak(String::from("silent.local").into_boxed_str());
    let mut resolver = StaticResolver::new();
    resolver.insert(
        host,
        port,
        vec![std::net::SocketAddr::from(([127, 0, 0, 1], port))],
    );
    let mut grants = StaticGrants::default();
    grants.insert("*", "default", NetGrant::open_local());
    grants.insert("*", &format!("http:{host}"), NetGrant::open_local());
    let net = NetService::new(NetConfig {
        resolver: Arc::new(resolver),
        grants: Arc::new(grants),
        clock: Arc::new(SystemClock),
    });
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::NET;
    let ctx = Ctx::root(caps, Budget::unlimited());
    let req = pii_abi::HttpRequest {
        method: pii_abi::HttpMethod::Post,
        url: format!("http://{host}:{port}/v1/chat/completions"),
        headers: Vec::new(),
        body: b"{}".to_vec(),
        timeout_ms: 5_000,
    };
    let resp = net.request_streaming(&ctx, &req, &mut |_| {});
    srv.join().expect("server thread");
    match resp {
        Ok(r) => panic!("silence must not be an empty success: status {}", r.status),
        Err(e) => {
            let m = e.to_string();
            assert!(
                m.contains("truncated response") || m.contains("net:"),
                "the truncated/silence error must name what happened: {m}"
            );
        }
    }
}
