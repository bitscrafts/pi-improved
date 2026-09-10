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

    /// Builder: attaches deny prefixes on top of the existing allow set
    /// (spec 31 R1). `permits` already treats `deny` as winning over `allow`
    /// — this is the only public way to populate it without hand-building
    /// the struct, so a deny list is a capability, not a policy bolted on
    /// after the fact.
    #[must_use]
    pub fn with_deny(mut self, deny: Vec<PathBuf>) -> Self {
        self.deny = deny;
        self
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

/// Default protected-path deny list for a heal run's `fs_write` scope.
///
/// Relative to the run's `scope_root` (spec 31 R1). A heal worktree is a full
/// checkout, so without this a patch could delete the very assertions that
/// prove it worked. Shared by `run_caps` (the enforcement site) and `xtask
/// heal-collect` (the pre-write refusal check) so both stay in sync.
pub const PROTECTED_RELATIVE_PATHS: &[&str] = &[
    "specs",
    "xtask",
    "quality-gate.sh",
    "graphs",
    "clippy.toml",
    "rust-toolchain.toml",
    ".github",
];

/// A prefix allowlist over strings, with one matching semantics per type.
///
/// 001 §2.4 specified three distinct scope types because they encode three
/// different matching semantics — wildcard host+port, program basename,
/// namespace prefix. One `StringScope` meant one semantics. Split (B7).
macro_rules! string_scope {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, Default)]
        pub struct $name(Vec<String>);

        impl $name {
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
    };
}

string_scope!(
    HostScope,
    "A prefix allowlist over net host:port strings (wildcard host+port semantics)."
);
string_scope!(
    ArgvScope,
    "A prefix allowlist over process argv0 strings (program basename semantics)."
);
string_scope!(
    SecretScope,
    "A prefix allowlist over secret key strings (namespace prefix semantics)."
);

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
    pub net_hosts: HostScope,
    /// Allowed proc argv0 prefixes.
    pub proc_argv0: ArgvScope,
    /// Allowed secret key prefixes.
    pub secrets: SecretScope,
}

impl Default for CapSet {
    /// A capability set granting nothing.
    fn default() -> Self {
        Self {
            kinds: CapKind::empty(),
            fs_read: PathScope::default(),
            fs_write: PathScope::default(),
            net_hosts: HostScope::default(),
            proc_argv0: ArgvScope::default(),
            secrets: SecretScope::default(),
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

    /// Narrows only the kind bits, keeping every path/host scope intact
    /// (F4.3: node capability narrowing — scopes are inherited from the run
    /// context; only the kind bits attenuate).
    #[must_use]
    pub fn narrow_kinds(&self, kinds: CapKind) -> Self {
        let mut cs = Self::clone(self);
        cs.kinds &= kinds;
        cs
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

    /// R1 (spec 31): `with_deny` builds a deny prefix that wins even though
    /// an allow prefix covers the same path — the acceptance property a
    /// path-list-only check (filtering candidates before calling `permits`)
    /// would not exercise, since here the deny is enforced INSIDE `permits`.
    #[test]
    fn protected_deny_wins_even_when_allow_covers_it() {
        use super::PathScope;
        let s = PathScope::new(vec![Path::new("/wt").to_path_buf()])
            .with_deny(vec![Path::new("/wt/specs").to_path_buf()]);
        assert!(!s.permits(Path::new("/wt/specs/foo.md")));
        assert!(s.permits(Path::new("/wt/crates/x.rs")));
    }

    /// R4 (spec 31): the shared protected-path list is non-empty and every
    /// entry is a bare relative fragment (no leading `/`), so joining it onto
    /// a `scope_root` in `run_caps` produces a path INSIDE the worktree, not
    /// an absolute escape.
    #[test]
    fn protected_relative_paths_are_relative() {
        use super::PROTECTED_RELATIVE_PATHS;
        assert!(!PROTECTED_RELATIVE_PATHS.is_empty());
        for p in PROTECTED_RELATIVE_PATHS {
            assert!(!p.starts_with('/'), "{p} must be relative");
        }
    }

    #[test]
    fn parse_kind_roundtrip() {
        assert_eq!(CapKind::parse_kind("fs_read").unwrap(), CapKind::FS_READ);
        assert!(CapKind::parse_kind("bogus").is_err());
    }
}
