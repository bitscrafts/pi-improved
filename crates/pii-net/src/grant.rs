//! Egress grants (F2.2): host scopes, budgets, and the grant store seam.

use std::collections::BTreeMap;
use std::sync::Arc;

/// One `host:port` pattern. Host part is `name`, `*.suffix`, or `name:*`
/// ports; port part is exact or `*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPattern {
    host: String,
    port: PortPart,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PortPart {
    Exact(u16),
    Any,
}

impl HostPattern {
    /// Parses `"host:port" | "*.suffix:port" | "host:*"`.
    ///
    /// # Errors
    /// [`pii_abi::PiiError::Invalid`] for a malformed pattern.
    pub fn parse(s: &str) -> Result<Self, pii_abi::PiiError> {
        let (host, port) = s
            .rsplit_once(':')
            .ok_or_else(|| invalid(s, "missing ':port'"))?;
        if host.is_empty() {
            return Err(invalid(s, "empty host"));
        }
        let port = if port == "*" {
            PortPart::Any
        } else {
            PortPart::Exact(
                port.parse()
                    .map_err(|_| invalid(s, "port must be u16 or '*'"))?,
            )
        };
        Ok(Self {
            host: host.to_ascii_lowercase(),
            port,
        })
    }

    /// True when `host:port` is covered. `*.suffix` matches subdomains at
    /// any depth but never the bare suffix.
    #[must_use]
    pub fn matches(&self, host: &str, port: u16) -> bool {
        let port_ok = match self.port {
            PortPart::Any => true,
            PortPart::Exact(p) => p == port,
        };
        if !port_ok {
            return false;
        }
        let h = host.to_ascii_lowercase();
        if self.host == "*" {
            return true;
        }
        self.host.strip_prefix("*.").map_or_else(
            || h == self.host,
            |suffix| h.len() > suffix.len() + 1 && h.ends_with(&format!(".{suffix}")),
        )
    }
}

fn invalid(s: &str, why: &str) -> pii_abi::PiiError {
    pii_abi::PiiError::Invalid(format!("host pattern '{s}': {why}"))
}

/// The allow set of `host:port` patterns a grant covers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HostScope(Vec<HostPattern>);

impl HostScope {
    /// Parses a list of patterns.
    ///
    /// # Errors
    /// [`pii_abi::PiiError::Invalid`] for any malformed pattern.
    pub fn parse_all(patterns: &[String]) -> Result<Self, pii_abi::PiiError> {
        Ok(Self(
            patterns
                .iter()
                .map(|s| HostPattern::parse(s))
                .collect::<Result<Vec<_>, _>>()?,
        ))
    }

    /// True when any pattern covers `host:port`.
    #[must_use]
    pub fn permits(&self, host: &str, port: u16) -> bool {
        self.0.iter().any(|p| p.matches(host, port))
    }
}

/// A spendable egress grant (F2.2). No `Default`: a zero grant is a
/// downgrade, never a convenience.
#[derive(Debug, Clone)]
pub struct NetGrant {
    /// Hosts this grant may reach.
    pub connect: HostScope,
    /// Whether loopback/private/link-local classes are allowed.
    pub allow_private: bool,
    /// Concurrent connection lease cap.
    pub max_conns: u32,
    /// Cumulative bytes out cap.
    pub max_bytes_out: u64,
    /// Cumulative bytes in cap.
    pub max_bytes_in: u64,
    /// Absolute expiry in clock milliseconds; `None` never expires.
    pub expires_at_ms: Option<u64>,
}

impl NetGrant {
    /// A permissive grant for tests and the loopback fake server.
    #[must_use]
    pub fn open_local() -> Self {
        Self {
            connect: HostScope::parse_all(&[String::from("*:*")])
                .unwrap_or_else(|_| HostScope::default()),
            allow_private: true,
            max_conns: u32::MAX,
            max_bytes_out: u64::MAX,
            max_bytes_in: u64::MAX,
            expires_at_ms: None,
        }
    }
}

/// Where grants come from, keyed by `(principal, key)`.
pub trait GrantStore: Send + Sync {
    /// The grant for a principal/key pair, if any.
    fn grant(&self, principal: &str, key: &str) -> Option<NetGrant>;
    /// Every `(principal, key, grant)` in the store.
    fn all(&self) -> Vec<(String, String, NetGrant)>;
}

/// An in-memory grant store.
#[derive(Debug, Clone, Default)]
pub struct StaticGrants(BTreeMap<(String, String), NetGrant>);

impl StaticGrants {
    /// Inserts or replaces a grant.
    pub fn insert(&mut self, principal: &str, key: &str, grant: NetGrant) {
        self.0
            .insert((String::from(principal), String::from(key)), grant);
    }
}

impl GrantStore for StaticGrants {
    fn grant(&self, principal: &str, key: &str) -> Option<NetGrant> {
        self.0
            .get(&(String::from(principal), String::from(key)))
            .cloned()
    }

    fn all(&self) -> Vec<(String, String, NetGrant)> {
        self.0
            .iter()
            .map(|((p, k), g)| (p.clone(), k.clone(), g.clone()))
            .collect()
    }
}

/// Shared, clonable grant store handle.
pub type Grants = Arc<dyn GrantStore>;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{HostPattern, HostScope};

    #[test]
    fn host_pattern_wildcard_depth() {
        let p = HostPattern::parse("*.example.com:443").unwrap();
        assert!(p.matches("a.example.com", 443));
        assert!(p.matches("a.b.example.com", 443));
        assert!(!p.matches("example.com", 443));
        assert!(!p.matches("evilexample.com", 443));
        assert!(!p.matches("a.example.com", 8443));
    }

    #[test]
    fn host_pattern_any_port() {
        let p = HostPattern::parse("host:*").unwrap();
        assert!(p.matches("host", 1));
        assert!(p.matches("HOST", 65535));
        assert!(!p.matches("other", 1));
    }

    #[test]
    fn scope_parse_rejects_malformed() {
        assert!(HostScope::parse_all(&["noport".into()]).is_err());
        assert!(HostScope::parse_all(&["h:notaport".into()]).is_err());
        assert!(HostScope::parse_all(&["h:80".into()]).is_ok());
    }
}
