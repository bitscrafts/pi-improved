//! Deterministic fault-injection seams for `Store`, used by integration tests in
//! other crates (they cannot reach `#[cfg(test)]` items, so these live in the
//! public surface but carry no API promise).
//!
//! Every setter flips one atomic that the corresponding `Store` method checks
//! before touching redb, so a test can make exactly one write or read fail
//! without a real I/O fault. They are the seams behind the D3/D6 persist-error
//! test, the spec-77 journal flush-loss test and the spec-78 resume-refusal
//! tests.
//!
//! Kept in their own module so `lib.rs` stays inside the workspace's effective-
//! line gate as hooks accumulate.

use std::sync::atomic::Ordering;

use crate::{PiiError, Store};

impl Store {
    /// **Test instrumentation only** — not a public API promise. When set,
    /// every subsequent [`Store::put_session`] / [`Store::append_session_message`]
    /// / [`Store::put_session_snapshot`] call fails immediately with a
    /// synthetic [`crate::PiiError::Internal`], without touching redb — the
    /// deterministic fault-injection hook D3/D6's "persist failure reaches
    /// the Observer" test uses.
    pub fn set_fail_session_writes_for_test(&self, fail: bool) {
        self.fail_session_writes_for_test
            .store(fail, Ordering::Relaxed);
    }

    /// **Test instrumentation only.** Makes the next `n`
    /// [`Store::append_events`] calls fail with a synthetic
    /// [`crate::PiiError::Internal`] before touching redb (journal flush-loss test).
    pub fn set_fail_append_events_for_test(&self, n: u64) {
        self.fail_append_events_for_test.store(n, Ordering::Relaxed);
    }

    /// **Test instrumentation only.** While set, [`Store::read_journal`]
    /// fails with a synthetic [`crate::PiiError::Internal`].
    pub fn set_fail_read_journal_for_test(&self, fail: bool) {
        self.fail_read_journal_for_test
            .store(fail, Ordering::Relaxed);
    }

    /// **Test instrumentation only.** While set, [`Store::get_run`] fails
    /// with a synthetic [`crate::PiiError::Internal`].
    pub fn set_fail_get_run_for_test(&self, fail: bool) {
        self.fail_get_run_for_test.store(fail, Ordering::Relaxed);
    }

    /// **Test instrumentation only.** While set, [`Store::get_graph`] fails
    /// with a synthetic [`crate::PiiError::Internal`].
    pub fn set_fail_get_graph_for_test(&self, fail: bool) {
        self.fail_get_graph_for_test.store(fail, Ordering::Relaxed);
    }

    /// The synthetic error [`Store::append_events`] returns while the counter is
    /// armed (each call decrements it).
    pub(crate) fn injected_append_events_error(&self) -> Option<PiiError> {
        if self
            .fail_append_events_for_test
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
            .is_ok()
        {
            return Some(PiiError::Internal(String::from(
                "store: injected append_events fault",
            )));
        }
        None
    }

    /// The synthetic error [`Store::get_run`] returns while armed.
    pub(crate) fn injected_get_run_error(&self) -> Option<PiiError> {
        if self.fail_get_run_for_test.load(Ordering::Relaxed) {
            return Some(PiiError::Internal(String::from(
                "store: injected get_run fault",
            )));
        }
        None
    }

    /// The synthetic error [`Store::get_graph`] returns while armed.
    pub(crate) fn injected_get_graph_error(&self) -> Option<PiiError> {
        if self.fail_get_graph_for_test.load(Ordering::Relaxed) {
            return Some(PiiError::Internal(String::from(
                "store: injected get_graph fault",
            )));
        }
        None
    }

    /// The synthetic error [`Store::read_journal`] returns while armed.
    pub(crate) fn injected_read_journal_error(&self) -> Option<PiiError> {
        if self.fail_read_journal_for_test.load(Ordering::Relaxed) {
            return Some(PiiError::Internal(String::from(
                "store: injected read_journal fault",
            )));
        }
        None
    }
}
