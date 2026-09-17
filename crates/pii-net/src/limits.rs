//! Shared limits (F2.8/F2.10).

use std::sync::atomic::{AtomicUsize, Ordering};

/// Response body cap for the HTTP client (F2.8: 8 MiB).
pub const MAX_HTTP_BODY: usize = 8 * 1024 * 1024;
/// Body cap shared with the codec's chunked decoder.
pub const MAX_BODY: usize = crate::http_codec::MAX_BODY;
/// Server-side request body cap (F2.10: 1 MiB).
pub const MAX_SERVER_BODY: usize = 1024 * 1024;
/// Concurrent connections per listener (excess get 503, F2.10).
pub const MAX_SERVER_CONNS: u32 = 64;
/// Worker-pool size for the accept loop: at most this many connection-
/// serving threads exist per listener; further connections queue on a
/// bounded channel until a worker frees up. An (undocumented) test override
/// lets the pool be exercised at small N; 0 means use the default.
static ACCEPT_POOL_OVERRIDE: AtomicUsize = AtomicUsize::new(0);

/// Test-only pool-size override (hidden from docs; not part of the
/// service API).
#[doc(hidden)]
pub fn set_accept_pool_override(n: usize) {
    ACCEPT_POOL_OVERRIDE.store(n, Ordering::Relaxed);
}

/// Worker-pool size for the accept loop.
#[must_use]
pub fn accept_pool_size() -> usize {
    let o = ACCEPT_POOL_OVERRIDE.load(Ordering::Relaxed);
    if o > 0 { o } else { MAX_SERVER_CONNS as usize }
}
