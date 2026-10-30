//! Time and sleeping, injected (spec 319 non-functional): the one real clock is here, so tests run in milliseconds.

/// Seconds since the lane started, and a sleep. A fake clock advances on `sleep` and never on its own.
pub(crate) trait Clock {
    fn now(&self) -> u64;
    fn sleep(&self, secs: u64);
}

/// The real clock: monotonic seconds since construction, and `std::thread::sleep`.
pub(crate) struct Real {
    start: std::time::Instant,
}

impl Real {
    // reason: the lane's own clock is the one place real time enters; the pure core only sees the trait.
    #[allow(clippy::disallowed_methods)]
    pub(crate) fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
        }
    }
}

impl Clock for Real {
    fn now(&self) -> u64 {
        self.start.elapsed().as_secs()
    }

    // reason: the one real sleep (see `Real`).
    #[allow(clippy::disallowed_methods)]
    fn sleep(&self, secs: u64) {
        std::thread::sleep(std::time::Duration::from_secs(secs));
    }
}
