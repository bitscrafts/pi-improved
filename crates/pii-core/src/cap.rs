//! Capability scopes and the complete capability set.
//!
//! Every capability is a monotone lattice: the only operation is
//! `intersect`, which never widens. There is intentionally no `union` here.
//!
//! The coarse [`CapKind`] bits live in `pii-abi` (the stable ABI surface);
//! this crate re-exports them and adds the path/string scopes.

use std::path::{Path, PathBuf};

use pii_abi::CapKind;

/// A canonicalised allow/deny set of path prefixes.
///
/// `permits` is deny-wins: a path in `deny` is never allowed even if an allow
/// prefix covers it. Empty `allow` permits nothing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PathScope {
    /// Prefixes that grant access.
    allow: Vec<PathBuf>,
    /// Prefixes that revoke access.
    deny: Vec<PathBuf>,
}

impl PathScope {
    /// A fresh scope permitting nothing.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// A scope allowing exactly the given canonical prefixes.
    #[must_use]
    pub fn new(allow: Vec<PathBuf>) -> Self {
        Self {
            allow,
            deny: Vec::new(),
        }
    }

    /// Builds from string prefixes.
    #[must_use]
    pub fn from_strings(allow: &[impl AsRef<Path>]) -> Self {
        Self {
            allow: allow.iter().map(|p| p.as_ref().to_path_buf()).collect(),
            deny: Vec::new(),
        }
    }

    /// True if `canonical` is inside an allow prefix and not inside a deny.
    #[must_use]
    pub fn permits(&self, canonical: &Path) -> bool {
        if self.deny.iter().any(|d| canonical.starts_with(d)) {
            return false;
        }
        self.allow.iter().any(|a| canonical.starts_with(a))
    }

    /// Both scopes must permit the path. Result allow = prefixes permitted by
    /// both; deny = union of denies.
    #[must_use]
    pub fn intersect(&self, other: &Self) -> Self {
        let mut allow = Vec::new();
        for p in self.allow.iter().chain(other.allow.iter()) {
            if self.permits(p) && other.permits(p) {
                allow.push(p.clone());
            }
        }
        // Deduplicate allow prefixes. PathBuf is Hash.
        let mut seen: Vec<PathBuf> = Vec::new();
        allow.retain(|p| {
            if seen.contains(p) {
                false
            } else {
                seen.push(p.clone());
                true
            }
        });
        let mut deny = self.deny.clone();
        for d in &other.deny {
            if !deny.contains(d) {
                deny.push(d.clone());
            }
        }
        Self { allow, deny }
    }
}

/// A prefix allowlist over strings (used for net hosts / proc argv0 in Phase 1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StringScope(Vec<String>);

impl StringScope {
    /// A scope allowing the given string prefixes.
    #[must_use]
    pub fn new(allowed: Vec<String>) -> Self {
        Self(allowed)
    }

    /// True if the value starts with any allowed prefix.
    #[must_use]
    pub fn permits(&self, value: &str) -> bool {
        self.0.iter().any(|p| value.starts_with(p))
    }

    /// Both scopes must permit the value.
    #[must_use]
    pub fn intersect(&self, other: &Self) -> Self {
        let mut out = Vec::new();
        for p in self.0.iter().chain(other.0.iter()) {
            if self.permits(p) && other.permits(p) && !out.contains(p) {
                out.push(p.clone());
            }
        }
        Self(out)
    }
}

/// The complete capability set a context holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapSet {
    /// Coarse capability bits.
    pub kinds: CapKind,
    /// Read path scope.
    pub fs_read: PathScope,
    /// Write path scope.
    pub fs_write: PathScope,
    /// Allowed net host prefixes.
    pub net_hosts: StringScope,
    /// Allowed proc argv0 prefixes.
    pub proc_argv0: StringScope,
    /// Allowed secret key prefixes.
    pub secrets: StringScope,
}

impl Default for CapSet {
    /// A capability set granting nothing.
    fn default() -> Self {
        Self {
            kinds: CapKind::empty(),
            fs_read: PathScope::default(),
            fs_write: PathScope::default(),
            net_hosts: StringScope::default(),
            proc_argv0: StringScope::default(),
            secrets: StringScope::default(),
        }
    }
}

impl CapSet {
    /// A capability set granting nothing.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Attenuation only: `self ∩ other`. Kinds are bitwise AND; every scope is
    /// intersected. Never widens.
    #[must_use]
    pub fn intersect(&self, other: &Self) -> Self {
        Self {
            kinds: self.kinds & other.kinds,
            fs_read: self.fs_read.intersect(&other.fs_read),
            fs_write: self.fs_write.intersect(&other.fs_write),
            net_hosts: self.net_hosts.intersect(&other.net_hosts),
            proc_argv0: self.proc_argv0.intersect(&other.proc_argv0),
            secrets: self.secrets.intersect(&other.secrets),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::path::Path;

    use super::{CapKind, CapSet};

    #[test]
    fn capset_intersect_never_widens() {
        let a = CapSet {
            kinds: CapKind::FS_READ,
            ..CapSet::default()
        };
        let b = CapSet {
            kinds: CapKind::FS_READ | CapKind::FS_WRITE | CapKind::NET,
            ..CapSet::default()
        };
        assert_eq!(a.intersect(&b).kinds, CapKind::FS_READ);
        assert_eq!(b.intersect(&a).kinds, CapKind::FS_READ);
    }

    #[test]
    fn pathscope_permits_deny_wins() {
        use super::PathScope;
        let s = PathScope::new(vec![Path::new("/a/b").to_path_buf()]);
        assert!(s.permits(Path::new("/a/b/c")));
        assert!(!s.permits(Path::new("/a")));
        assert!(!s.permits(Path::new("/x")));
    }

    #[test]
    fn parse_kind_roundtrip() {
        assert_eq!(CapKind::parse_kind("fs_read").unwrap(), CapKind::FS_READ);
        assert!(CapKind::parse_kind("bogus").is_err());
    }
}
