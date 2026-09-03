//! Shared limits (F2.8/F2.10).

/// Response body cap for the HTTP client (F2.8: 8 MiB).
pub const MAX_HTTP_BODY: usize = 8 * 1024 * 1024;
/// Server-side request body cap (F2.10: 1 MiB).
pub const MAX_SERVER_BODY: usize = 1024 * 1024;
/// Concurrent connections per listener (excess get 503, F2.10).
pub const MAX_SERVER_CONNS: u32 = 64;
