//! The journal's `stop_reason` says why an agent run ended. The timeout fix ends a hung agent with exit code 124 and
//! the journal still said `end_turn`, which reads as "the agent finished its turn". A killed-by-timeout run must say
//! so. Controls: a clean exit and an ordinary failure stay `end_turn`, a signal stays `signalled`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_server::node_kinds::nodes::coder::stop_reason;

#[test]
fn the_timeout_exit_code_is_reported_as_a_timeout() {
    assert_eq!(stop_reason(124), "timeout");
}

#[test]
fn a_clean_exit_is_still_end_turn() {
    assert_eq!(stop_reason(0), "end_turn");
}

#[test]
fn an_ordinary_failure_is_still_end_turn() {
    assert_eq!(stop_reason(1), "end_turn");
    assert_eq!(stop_reason(3), "end_turn");
}

#[test]
fn a_signal_exit_is_still_signalled() {
    assert_eq!(stop_reason(137), "signalled");
}
