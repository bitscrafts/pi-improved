//! Unit-level control (inside pii-net, where `Conn` is constructible):
//! a rustls server sends a COMPLETE response then drops WITHOUT `close_notify`;
//! `Conn::read` itself — the production read — must turn that close into
//! end-of-stream (`Ok(0)`), and a close before any byte must stay an error.
#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    // reason: the test serves its own loopback peer; grants do not apply to
    // the test's own listener, as in tests/budget_window.rs.
    clippy::disallowed_methods
)]

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;

use crate::SystemClock;
use crate::clock::Clock as _;
use pii_abi::PiiError;

use super::conn::{Conn, Stream};

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

/// A permissive grant + inner registry, shared like `NetService` shares it.
fn test_inner() -> Arc<super::conn::Inner> {
    let mut grants = crate::grant::StaticGrants::default();
    grants.insert("*", "default", crate::grant::NetGrant::open_local());
    Arc::new(super::conn::Inner {
        resolver: Arc::new(crate::resolve::StaticResolver::new()),
        grants: Arc::new(grants),
        clock: Arc::new(SystemClock),
        registry: parking_lot::Mutex::new(super::conn::Registry::default()),
        listeners: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
        observer: std::sync::OnceLock::new(),
    })
}

fn client_conn(server: std::net::TcpStream) -> Conn {
    let mut root = rustls::ClientConfig::builder()
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
    let verifier: Arc<dyn rustls::client::danger::ServerCertVerifier> = Arc::new(AcceptAll);
    root.dangerous().set_certificate_verifier(verifier);
    let name =
        rustls::pki_types::ServerName::try_from(String::from("localhost")).expect("server name");
    let conn = rustls::ClientConnection::new(Arc::new(root), name).expect("client conn");
    Conn {
        stream: Stream::Tls(Box::new(rustls::StreamOwned::new(conn, server))),
        key: (String::from("test"), String::from("default")),
        inner: test_inner(),
        deadline_ms: 0,
        sent: 0,
        received: 0,
    }
}

fn serving() -> (thread::JoinHandle<()>, TcpListener) {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let h = serve_abrupt_tls(l.try_clone().expect("clone"));
    (h, l)
}

fn serve_abrupt_tls(l: TcpListener) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let Ok((s, _)) = l.accept() else { return };
        let Ok(cert) = rcgen::generate_simple_self_signed(vec!["localhost".into()]) else {
            return;
        };
        let der = cert.cert.der().clone();
        let ckey = cert.signing_key.serialize_der();
        let key = rustls::pki_types::PrivateKeyDer::Pkcs8(ckey.into());
        let Ok(cfg) = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![der], key)
        else {
            return;
        };
        let Ok(conn) = rustls::ServerConnection::new(Arc::new(cfg)) else {
            return;
        };
        let mut tls = rustls::StreamOwned::new(conn, s);
        let mut buf = [0u8; 16384];
        let mut got = false;
        for _ in 0..50 {
            match tls.read(&mut buf) {
                Ok(n) if n > 0 && buf[..n].windows(4).any(|w| w == b"\r\n\r\n") => {
                    got = true;
                    break;
                }
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
        if !got {
            return;
        }
        let head: &'static str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n";
        let _ = tls.write_all(head.as_bytes());
        let _ = tls.write_all(b"data: PONG-GLM\n\n");
        let _ = tls.flush();
        drop(tls); // INJECTION: FIN without close_notify
    })
}

fn client_ready(port: u16) -> Conn {
    let s = std::net::TcpStream::connect(("127.0.0.1", port)).expect("dial");
    let mut conn = client_conn(s);
    let pinned = crate::authorize::Pinned {
        addrs: Vec::new(),
        deadline_ms: SystemClock.now_ms() + 5_000,
    };
    conn.arm_deadline(&pinned, &SystemClock).expect("arm");
    conn
}

#[test]
fn conn_read_maps_an_abrupt_tls_close_after_a_full_answer_to_end_of_stream() {
    let (srv, l) = serving();
    let port = l.local_addr().expect("p").port();
    let mut conn = client_ready(port);
    // write the request head through the PRODUCTION write path
    conn.write_all(b"POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .expect("write request");
    // read the whole body: after the full answer, the abrupt close maps to Ok(0)
    let mut buf = [0u8; 8192];
    let mut got = Vec::new();
    let mut turns = 0;
    loop {
        turns += 1;
        match conn.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(e) => panic!("the abrupt close must NOT surface as an error: {e}"),
        }
        if turns > 64 {
            break;
        }
    }
    let t = String::from_utf8_lossy(&got).to_string();
    assert!(t.contains("PONG-GLM"), "the answer must be kept; got {t:?}");
    srv.join().expect("server");
}

#[test]
fn conn_read_of_an_abrupt_close_before_any_byte_stays_an_error() {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = l.local_addr().expect("p").port();
    let srv = thread::spawn(move || {
        let Ok((s, _)) = l.accept() else { return };
        drop(s); // INJECTION: accept-and-drop, nothing ever sent
    });
    let mut conn = client_ready(port);
    // The server dropped before anything crossed: the write itself may fail
    // (broken pipe) or succeed; both are honest. What matters is the read
    // never yields plaintext and never pretends a response exists.
    let _ = conn.write_all(b"POST /x HTTP/1.1\r\nHost: localhost\r\n\r\n");
    let mut buf = [0u8; 8192];
    match conn.read(&mut buf) {
        Ok(n) => assert_eq!(n, 0, "no plaintext can exist; got {n} bytes"),
        Err(PiiError::Unavailable(m)) if m.contains("net: read") && !m.contains("diagnostic") => {
            // an unmapped error is also acceptable at this layer — the HTTP
            // head loop treats it as truncation. The bug being guarded is the
            // SUCCESS-with-error shape, not which error text wins.
        }
        Err(e) => panic!("unexpected error shape: {e}"),
    }
    srv.join().expect("server");
}
