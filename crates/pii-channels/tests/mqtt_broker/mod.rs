//! A minimal in-process MQTT 3.1.1 broker for tests (spec 117 R5): loopback
//! only, one thread per connection, QoS 0/1, no sessions, no retained
//! delivery. It records what clients sent — that record is "the broker's view".
//!
//! Cost of hand-rolling it instead of `rumqttd` (measured 2026-10-10: ~370
//! lines of dependency tree, `config`, a second TLS stack): it proves the
//! client against *this* reading of the protocol, not against a second
//! independent implementation. `docs` name that gap.

// reason: a test double: panics are failures, indexing parses fixed-shape packets
// it just read, and it is the one loopback socket owner (allowlisted in net-check).
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    clippy::redundant_pub_crate,
    clippy::doc_markdown,
    clippy::no_effect_underscore_binding
)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;

/// One PUBLISH the broker received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Seen {
    pub(crate) topic: String,
    pub(crate) payload: Vec<u8>,
    pub(crate) qos: u8,
    pub(crate) retain: bool,
}

/// One CONNECT the broker received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Connect {
    pub(crate) client_id: String,
    pub(crate) username: Option<String>,
    pub(crate) password: Option<String>,
}

#[derive(Default)]
struct State {
    published: Mutex<Vec<Seen>>,
    connects: Mutex<Vec<Connect>>,
    subscribed: Mutex<Vec<String>>,
    clients: Mutex<Vec<TcpStream>>,
    stop: AtomicBool,
    accepted: AtomicU64,
}

/// The running broker. Dropping it stops the accept loop and closes clients.
pub(crate) struct Broker {
    pub(crate) port: u16,
    state: Arc<State>,
}

impl Broker {
    /// Binds `127.0.0.1:0`; `reject_auth` answers every CONNECT with
    /// "bad user name or password" (CONNACK 4).
    pub(crate) fn start(reject_auth: bool) -> Self {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        Self::on(l, reject_auth)
    }

    /// A loopback port nothing listens on. Taken from a range below the
    /// kernel's ephemeral range (so a parallel test's `bind(0)` can never be
    /// handed it), probed for being free.
    pub(crate) fn free_port() -> u16 {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let base = 12_000 + u64::from(std::process::id() % 400) * 20;
        loop {
            let port = u16::try_from(base + NEXT.fetch_add(1, Ordering::SeqCst) % 20).unwrap();
            if TcpListener::bind(("127.0.0.1", port)).is_ok() {
                return port;
            }
        }
    }

    /// Serves on `port` (restart a broker where a dead one was expected).
    pub(crate) fn start_on(port: u16, reject_auth: bool) -> Self {
        Self::on(TcpListener::bind(("127.0.0.1", port)).unwrap(), reject_auth)
    }

    fn on(l: TcpListener, reject_auth: bool) -> Self {
        let port = l.local_addr().unwrap().port();
        l.set_nonblocking(true).unwrap();
        let state = Arc::new(State::default());
        let st = Arc::clone(&state);
        std::thread::spawn(move || {
            while !st.stop.load(Ordering::SeqCst) {
                match l.accept() {
                    Ok((s, _)) => {
                        st.accepted.fetch_add(1, Ordering::SeqCst);
                        s.set_nonblocking(false).unwrap();
                        st.clients.lock().push(s.try_clone().unwrap());
                        let st2 = Arc::clone(&st);
                        std::thread::spawn(move || serve(s, &st2, reject_auth));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Self { port, state }
    }

    pub(crate) fn published(&self) -> Vec<Seen> {
        self.state.published.lock().clone()
    }

    pub(crate) fn connects(&self) -> Vec<Connect> {
        self.state.connects.lock().clone()
    }

    pub(crate) fn subscribed(&self) -> Vec<String> {
        self.state.subscribed.lock().clone()
    }

    pub(crate) fn accepted(&self) -> u64 {
        self.state.accepted.load(Ordering::SeqCst)
    }

    /// Pushes a QoS 0 PUBLISH of raw bytes to every connected client.
    pub(crate) fn push(&self, topic: &str, payload: &[u8]) {
        let pkt = publish_packet(topic, payload);
        for c in self.state.clients.lock().iter_mut() {
            let _ = c.write_all(&pkt);
        }
    }

    /// Stops accepting and drops every client connection.
    pub(crate) fn stop(&self) {
        self.state.stop.store(true, Ordering::SeqCst);
        for c in self.state.clients.lock().drain(..) {
            let _ = c.shutdown(std::net::Shutdown::Both);
        }
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn publish_packet(topic: &str, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&u16::try_from(topic.len()).unwrap().to_be_bytes());
    body.extend_from_slice(topic.as_bytes());
    body.extend_from_slice(payload);
    let mut out = vec![0x30];
    out.extend(remaining_length(body.len()));
    out.extend(body);
    out
}

fn remaining_length(mut n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut b = u8::try_from(n % 128).unwrap();
        n /= 128;
        if n > 0 {
            b |= 0x80;
        }
        out.push(b);
        if n == 0 {
            return out;
        }
    }
}

fn read_packet(s: &mut TcpStream) -> Option<(u8, Vec<u8>)> {
    let mut first = [0u8; 1];
    s.read_exact(&mut first).ok()?;
    let (mut len, mut mult) = (0usize, 1usize);
    loop {
        let mut b = [0u8; 1];
        s.read_exact(&mut b).ok()?;
        len += usize::from(b[0] & 0x7f) * mult;
        if b[0] & 0x80 == 0 {
            break;
        }
        mult *= 128;
    }
    let mut body = vec![0u8; len];
    s.read_exact(&mut body).ok()?;
    Some((first[0], body))
}

fn take_str(b: &[u8], at: &mut usize) -> String {
    let n = usize::from(u16::from_be_bytes([b[*at], b[*at + 1]]));
    let s = String::from_utf8_lossy(&b[*at + 2..*at + 2 + n]).into_owned();
    *at += 2 + n;
    s
}

fn serve(mut s: TcpStream, st: &State, reject_auth: bool) {
    while let Some((first, body)) = read_packet(&mut s) {
        match first >> 4 {
            1 => {
                let mut at = 0;
                let _proto = take_str(&body, &mut at);
                let _level = body[at];
                let flags = body[at + 1];
                at += 4; // level, flags, keepalive
                let client_id = take_str(&body, &mut at);
                if flags & 0x04 != 0 {
                    let _will_topic = take_str(&body, &mut at);
                    let _will_msg = take_str(&body, &mut at);
                }
                let username = (flags & 0x80 != 0).then(|| take_str(&body, &mut at));
                let password = (flags & 0x40 != 0).then(|| take_str(&body, &mut at));
                st.connects.lock().push(Connect {
                    client_id,
                    username,
                    password,
                });
                let code = if reject_auth { 4 } else { 0 };
                let _ = s.write_all(&[0x20, 2, 0, code]);
                if reject_auth {
                    return;
                }
            }
            3 => {
                let qos = (first >> 1) & 3;
                let mut at = 0;
                let topic = take_str(&body, &mut at);
                let pid = (qos > 0).then(|| {
                    let p = [body[at], body[at + 1]];
                    at += 2;
                    p
                });
                st.published.lock().push(Seen {
                    topic,
                    payload: body[at..].to_vec(),
                    qos,
                    retain: first & 1 == 1,
                });
                if let Some(p) = pid {
                    let _ = s.write_all(&[0x40, 2, p[0], p[1]]);
                }
            }
            8 => {
                let pid = [body[0], body[1]];
                let mut at = 2;
                let mut granted = Vec::new();
                while at < body.len() {
                    let f = take_str(&body, &mut at);
                    granted.push(body[at].min(1));
                    at += 1;
                    st.subscribed.lock().push(f);
                }
                let mut out = vec![0x90];
                out.extend(remaining_length(2 + granted.len()));
                out.extend(pid);
                out.extend(granted);
                let _ = s.write_all(&out);
            }
            12 => {
                let _ = s.write_all(&[0xD0, 0]);
            }
            14 => return,
            _ => {}
        }
    }
}
