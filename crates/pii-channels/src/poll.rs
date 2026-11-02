//! Spec 347: per-channel poll backoff and outage state.
//!
//! A channel whose poll keeps failing waits longer each time (1 s, doubling, capped at 5 min), a success
//! resets the wait, and the outage is reported as a state change: one `Degraded` and one `Recovered`, never
//! one per attempt. The daemon keeps one [`PollBackoff`] per channel and reads it from its own loop and its
//! own health answer.

use std::future::Future;
use std::time::Duration;

/// The first wait after a failure, and the wait while a channel is healthy.
pub const BASE_INTERVAL: Duration = Duration::from_secs(1);

/// The longest a failing channel waits between polls. Lowering it stalls a recovered channel; raising it
/// hides a recovery for longer than a quarter of an hour.
pub const CEILING: Duration = Duration::from_secs(300);

/// The attempt count shown in a message is capped here, so a long outage cannot grow a line without bound.
const COUNT_CAP: u64 = 1000;

/// What a recorded poll changed, for the caller that logs state changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// The first failure of an outage.
    Degraded,
    /// The first success after an outage.
    Recovered,
    /// Nothing changed state.
    None,
}

/// A state change worth reporting: the two ends of an outage.
///
/// [`Transition`] is the internal three-way answer (it also says "nothing changed"); this is what a
/// caller logs, so its match is exhaustive with no unreachable arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The first failure of an outage.
    Degraded,
    /// The first success after an outage.
    Recovered,
}

impl Transition {
    /// The reportable change, or `None` when nothing about the channel's state moved.
    #[must_use]
    pub fn change(self) -> Option<Change> {
        match self {
            Self::Degraded => Some(Change::Degraded),
            Self::Recovered => Some(Change::Recovered),
            Self::None => None,
        }
    }
}

/// Backoff and outage state for one channel. `now` is a wall-clock time since the Unix epoch.
#[derive(Debug, Clone, Default)]
pub struct PollBackoff {
    consecutive: u32,
    attempts: u64,
    degraded: bool,
    last_error: Option<String>,
    last_seen: Option<Duration>,
    resolved_at: Option<Duration>,
}

impl PollBackoff {
    /// A healthy channel with no failures recorded.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How long to wait before the next poll: the base after a success, then doubling with each consecutive
    /// failure (1 s, 2 s, 4 s, ...), never above [`CEILING`].
    #[must_use]
    pub fn interval(&self) -> Duration {
        let factor = 1u32 << self.consecutive.saturating_sub(1).min(20);
        BASE_INTERVAL.saturating_mul(factor).min(CEILING)
    }

    /// Records a failed poll at `now`. Returns `Degraded` only for the first failure of an outage.
    pub fn record_failure(&mut self, now: Duration, error: &str) -> Transition {
        if !self.degraded {
            // A new outage starts a fresh count and forgets the previous outage's resolution.
            self.attempts = 0;
            self.resolved_at = None;
        }
        self.consecutive = self.consecutive.saturating_add(1);
        self.attempts += 1;
        self.last_seen = Some(now);
        self.last_error = Some(error.to_owned());
        if self.degraded {
            Transition::None
        } else {
            self.degraded = true;
            Transition::Degraded
        }
    }

    /// Records a successful poll at `now`. Resets the wait; returns `Recovered` only if an outage ended.
    pub fn record_success(&mut self, now: Duration) -> Transition {
        self.consecutive = 0;
        if self.degraded {
            self.degraded = false;
            self.resolved_at = Some(now);
            Transition::Recovered
        } else {
            Transition::None
        }
    }

    /// The health detail of a channel that is failing now, or `None` while it is healthy:
    /// the last error, the attempt count (capped) and the time of the last attempt.
    #[must_use]
    pub fn outage(&self) -> Option<String> {
        if !self.degraded {
            return None;
        }
        let error = self.last_error.as_deref().unwrap_or("unknown error");
        let seen = self.last_seen.map_or(0, |t| t.as_secs());
        Some(format!(
            "{error} (count={}, last attempt {seen})",
            self.count_label()
        ))
    }

    /// The one line `pii health` carries for this channel while it is failing, or `None` while it is
    /// healthy: the channel, the last error, the attempt count and the last attempt. This is what makes an
    /// outage visible on a path that does not go through the channel that is down.
    #[must_use]
    pub fn health_line(&self, channel: &str) -> Option<String> {
        self.outage()
            .map(|why| format!("{channel}: degraded — {why}"))
    }

    /// The attempt count as shown in a message: exact below [`COUNT_CAP`], `N+` at it.
    #[must_use]
    pub fn count_label(&self) -> String {
        if self.attempts >= COUNT_CAP {
            format!("{COUNT_CAP}+")
        } else {
            self.attempts.to_string()
        }
    }

    /// The last poll error, for a log line.
    #[must_use]
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// How many consecutive failed attempts this outage has seen.
    #[must_use]
    pub fn failures(&self) -> u32 {
        self.consecutive
    }

    /// When the most recent failure happened.
    #[must_use]
    pub fn last_seen(&self) -> Option<Duration> {
        self.last_seen
    }

    /// When the most recent outage ended; `None` while it is still failing or it never failed.
    #[must_use]
    pub fn resolved_at(&self) -> Option<Duration> {
        self.resolved_at
    }
}

/// How long the loop sleeps after a poll (spec 194).
///
/// After a success (`err_text` `None`) it is the interval. After a failure it is the larger of the interval
/// and the provider's `retry_after`, else `interval * 2^min(n-1, 4)`; never above [`CEILING`].
#[must_use]
pub fn pause_ms(err_text: Option<&str>, consecutive: u32, interval_ms: u64) -> u64 {
    let ceiling = u64::try_from(CEILING.as_millis()).unwrap_or(u64::MAX);
    let Some(text) = err_text else {
        return interval_ms.min(ceiling);
    };
    let asked = crate::retry_after_ms(&pii_abi::PiiError::Unavailable(text.to_owned()));
    // Grow with consecutive failures EITHER WAY: a named pause is a floor, not the whole answer (spec 194).
    let factor = 1u64 << consecutive.saturating_sub(1).min(4);
    let base = asked.unwrap_or(interval_ms);
    interval_ms.max(base.saturating_mul(factor)).min(ceiling)
}

/// The poll loop of one channel: poll, wait, and wait longer while it keeps failing.
///
/// `sleep` and `report` are injected so the loop is testable without a clock or a daemon, and `poll` is a
/// closure returning a future so a blocking poll can be run on a blocking pool by the caller.
///
/// Two behaviours matter and both are measured by `tests/poll_backoff.rs`: the wait grows on consecutive
/// failures (base, doubling, capped at [`CEILING`]) and a success resets it; and an outage is reported
/// once when it starts and once when it ends, never once per attempt.
pub async fn run_loop<P, PF, S, SF, R, E>(
    base_ms: u64,
    backoff: &std::sync::Arc<parking_lot::Mutex<PollBackoff>>,
    poll: P,
    sleep: S,
    report: R,
) where
    P: Fn() -> PF,
    E: std::fmt::Display,
    PF: Future<Output = Result<(), E>>,
    S: Fn(u64) -> SF,
    SF: Future<Output = ()>,
    R: Fn(Change, &PollBackoff),
{
    loop {
        let outcome = poll().await;
        // Wall-clock by design: the outage stamps are read against the daemon's own log, which is wall-clock.
        #[allow(clippy::disallowed_methods)]
        // reason: outage stamps are wall-clock, like every daemon log line
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let (wait_ms, change) = {
            let mut state = backoff.lock();
            let (text, change) = match outcome {
                Ok(()) => (None, state.record_success(now).change()),
                Err(err) => {
                    let text = err.to_string();
                    (
                        Some(text.clone()),
                        state.record_failure(now, &text).change(),
                    )
                }
            };
            // A healthy channel keeps its configured cadence; a failing one waits the longer of the
            // provider's pause (spec 194) and its own backoff (spec 347).
            let wait = if text.is_some() {
                let backoff_ms = u64::try_from(state.interval().as_millis()).unwrap_or(u64::MAX);
                pause_ms(text.as_deref(), state.failures(), base_ms).max(backoff_ms)
            } else {
                base_ms
            };
            drop(state);
            (wait, change)
        };
        if let Some(change) = change {
            let state = backoff.lock();
            report(change, &state);
        }
        sleep(wait_ms).await;
    }
}
