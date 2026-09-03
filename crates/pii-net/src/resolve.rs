//! Name resolution (F2.4). The ONLY `ToSocketAddrs` call site in the
//! workspace lives in [`SystemResolver`]; `NetService::connect` resolves
//! exactly once and then dials only the pinned set (§4.3.2).

use std::collections::BTreeMap;
use std::net::SocketAddr;

use pii_abi::PiiError;

/// A name resolver.
pub trait Resolver: Send + Sync {
    /// Resolves `host:port` to candidate socket addresses, in answer order.
    ///
    /// # Errors
    /// [`PiiError::Unavailable`] on resolution failure (transient);
    /// [`PiiError::Invalid`] for a malformed host.
    fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, PiiError>;
}

/// The system resolver: the single sanctioned name-resolution call site.
pub struct SystemResolver;

impl Resolver for SystemResolver {
    #[allow(clippy::disallowed_methods)] // reason: resolve.rs is one of the
    // three pii-net socket files (spec F2.13/F1.6); every other crate is
    // denied name resolution by `cargo xtask net-check`.
    fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, PiiError> {
        use std::net::ToSocketAddrs as _;
        let addrs: Vec<SocketAddr> = (host, port)
            .to_socket_addrs()
            .map_err(|e| PiiError::Unavailable(format!("resolve {host}:{port}: {e}")))?
            .collect();
        Ok(addrs)
    }
}

/// A table resolver for tests: deterministic, offline.
#[derive(Debug, Default, Clone)]
pub struct StaticResolver(BTreeMap<(String, u16), Vec<SocketAddr>>);

impl StaticResolver {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one answer (the full address set, in dial order).
    pub fn insert(&mut self, host: &str, port: u16, addrs: Vec<SocketAddr>) {
        self.0.insert((String::from(host), port), addrs);
    }
}

impl Resolver for StaticResolver {
    fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, PiiError> {
        self.0
            .get(&(String::from(host), port))
            .cloned()
            .ok_or_else(|| {
                PiiError::Unavailable(format!("static resolver: no entry for {host}:{port}"))
            })
    }
}
