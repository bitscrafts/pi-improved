//! The pure egress decision (F2.3): classify, refuse mixed answers, block
//! dangerous classes, pin the deadline. No I/O — fully table-testable.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// A dial target (F2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Host name or literal IP.
    pub host: String,
    /// Port.
    pub port: u16,
}

/// The pinned outcome of authorize: the resolved address set plus the one
/// absolute deadline that spans all remaining stages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pinned {
    /// Dial order is exactly this order; never re-resolved.
    pub addrs: Vec<SocketAddr>,
    /// Absolute deadline in clock milliseconds.
    pub deadline_ms: u64,
}

/// Why egress was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetDenied {
    /// The grant's expiry has passed.
    Expired,
    /// `host:port` is not covered by the grant's `HostScope`.
    NotAllowed(String),
    /// Resolution produced nothing.
    NoAddress,
    /// Public and non-public addresses in one answer — an attack.
    MixedAnswer,
    /// Every address is in a refused class.
    BlockedClass(String),
}

impl std::fmt::Display for NetDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Expired => write!(f, "grant expired"),
            Self::NotAllowed(t) => write!(f, "target '{t}' not in grant scope"),
            Self::NoAddress => write!(f, "no address resolved"),
            Self::MixedAnswer => write!(f, "mixed public/private answer refused"),
            Self::BlockedClass(c) => write!(f, "address class '{c}' refused"),
        }
    }
}

impl NetDenied {
    /// Converts to the wire error shape (`PiiError::Denied("net: …")`).
    #[must_use]
    pub fn to_pii(&self) -> pii_abi::PiiError {
        pii_abi::PiiError::Denied(format!("net: {self}"))
    }
}

/// Address classes (F2.3 rule 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddrClass {
    /// Globally routable.
    Public,
    /// `127/8`, `::1`.
    Loopback,
    /// RFC1918, `fc00::/7`, IPv4-mapped private.
    Private,
    /// `169.254/16`, `fe80::/10`.
    LinkLocal,
    /// `169.254.169.254`, `fd00:ec2::254`.
    CloudMetadata,
    /// Multicast ranges.
    Multicast,
    /// `0.0.0.0`, `::`.
    Unspecified,
    /// `64:ff9b::/96` embedding a private/loopback/link-local IPv4.
    Nat64MappedPrivate,
}

impl AddrClass {
    /// Classifies one address; IPv4-mapped IPv6 is classified by its
    /// embedded IPv4.
    #[must_use]
    pub fn of(addr: &IpAddr) -> Self {
        match addr {
            IpAddr::V4(v4) => Self::v4(*v4),
            IpAddr::V6(v6) => {
                if let Some(embedded) = v6.to_ipv4_mapped() {
                    return Self::v4(embedded);
                }
                let segs = v6.segments();
                if *v6 == Ipv6Addr::LOCALHOST {
                    Self::Loopback
                } else if *v6 == Ipv6Addr::UNSPECIFIED {
                    Self::Unspecified
                } else if (segs[0] & 0xFF00) == 0xFF00 {
                    Self::Multicast
                } else if *v6 == Ipv6Addr::new(0xFD00, 0xEC2, 0, 0, 0, 0, 0, 0x254) {
                    Self::CloudMetadata
                } else if (segs[0] & 0xFE00) == 0xFC00 {
                    Self::Private
                } else if (segs[0] & 0xFFC0) == 0xFE80 {
                    Self::LinkLocal
                } else if segs[0] == 0x64
                    && segs[1] == 0xFF9B
                    && segs[2] == 0
                    && segs[3] == 0
                    && segs[4] == 0
                    && segs[5] == 0
                {
                    // 64:ff9b::/96 — the embedded IPv4 is the last 32 bits.
                    let v4 = Ipv4Addr::new(
                        u8::try_from(segs[6] >> 8).unwrap_or(0),
                        u8::try_from(segs[6] & 0xFF).unwrap_or(0),
                        u8::try_from(segs[7] >> 8).unwrap_or(0),
                        u8::try_from(segs[7] & 0xFF).unwrap_or(0),
                    );
                    match Self::v4(v4) {
                        Self::Public => Self::Public,
                        _ => Self::Nat64MappedPrivate,
                    }
                } else {
                    Self::Public
                }
            }
        }
    }

    fn v4(v4: Ipv4Addr) -> Self {
        let o = v4.octets();
        if v4.is_loopback() {
            Self::Loopback
        } else if v4.is_broadcast() || o[0] == 0 {
            Self::Unspecified
        } else if v4.is_multicast() {
            Self::Multicast
        } else if v4.is_link_local() {
            if v4 == Ipv4Addr::new(169, 254, 169, 254) {
                Self::CloudMetadata
            } else {
                Self::LinkLocal
            }
        } else if o[0] == 10
            || (o[0] == 172 && (16..=31).contains(&o[1]))
            || (o[0] == 192 && o[1] == 168)
        {
            Self::Private
        } else {
            Self::Public
        }
    }
}

/// The authorize decision (F2.3). Order matters and is specified.
///
/// # Errors
/// [`NetDenied`] converted to `PiiError::Denied` for every refusal.
pub fn authorize(
    target: &Target,
    resolved: &[IpAddr],
    grant: &crate::grant::NetGrant,
    now_ms: u64,
    timeout_ms: u64,
) -> Result<Pinned, pii_abi::PiiError> {
    authorize_d(target, resolved, grant, now_ms, timeout_ms).map_err(|d| d.to_pii())
}

/// Same decision, returning the typed [`NetDenied`] for tests.
///
/// # Errors
/// [`NetDenied`] when the grant is expired, the host is not permitted, the
/// address set is empty, a class is blocked, or the answer is mixed.
pub fn authorize_d(
    target: &Target,
    resolved: &[IpAddr],
    grant: &crate::grant::NetGrant,
    now_ms: u64,
    timeout_ms: u64,
) -> Result<Pinned, NetDenied> {
    if let Some(exp) = grant.expires_at_ms
        && now_ms >= exp
    {
        return Err(NetDenied::Expired);
    }
    if !grant.connect.permits(&target.host, target.port) {
        return Err(NetDenied::NotAllowed(format!(
            "{}:{}",
            target.host, target.port
        )));
    }
    if resolved.is_empty() {
        return Err(NetDenied::NoAddress);
    }
    let classes: Vec<AddrClass> = resolved.iter().map(AddrClass::of).collect();
    let has_public = classes.contains(&AddrClass::Public);
    let has_other = classes.iter().any(|c| *c != AddrClass::Public);
    if has_public && has_other {
        return Err(NetDenied::MixedAnswer);
    }
    for c in &classes {
        match c {
            AddrClass::CloudMetadata
            | AddrClass::Multicast
            | AddrClass::Unspecified
            | AddrClass::Nat64MappedPrivate => {
                return Err(NetDenied::BlockedClass(format!("{c:?}")));
            }
            AddrClass::Loopback | AddrClass::Private | AddrClass::LinkLocal => {
                if !grant.allow_private {
                    return Err(NetDenied::BlockedClass(format!("{c:?}")));
                }
            }
            AddrClass::Public => {}
        }
    }
    Ok(Pinned {
        addrs: resolved
            .iter()
            .map(|ip| SocketAddr::new(*ip, target.port))
            .collect(),
        deadline_ms: now_ms.saturating_add(timeout_ms),
    })
}
