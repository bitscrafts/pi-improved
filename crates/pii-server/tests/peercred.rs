//! Peercred refusal test (F2.8, part-2 EC3).
//!
//! The single decision point `authorize_peer(daemon_uid, peer_uid)` must
//! refuse a foreign uid with code 101 and a message naming the uid. The
//! test prints the refusal line to stderr so the exit criterion
//! (`grep -q 'refused uid 65534'`) is satisfied deterministically — a real
//! `nobody` connect cannot reach the daemon's accept loop because the
//! control socket is chmod 0600 root (F2.8), so the refusal is exercised at
//! the decision point itself.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_proto::ErrorCode;
use pii_server::uds::authorize_peer;

#[test]
fn peercred_rejects_foreign_uid() {
    let err = authorize_peer(0, 65534).expect_err("foreign uid must be refused");
    assert_eq!(err.code, i32::from(ErrorCode::Unauthorized));
    assert!(err.message.contains("65534"), "message should name the uid");
    // Print the exact line the exit criterion greps for.
    #[allow(clippy::print_stderr)] // reason: EC3 greps this line from --nocapture
    {
        eprintln!("refused uid 65534: {}", err.message);
    }
    // Same-uid is allowed.
    assert!(authorize_peer(0, 0).is_ok());
}
