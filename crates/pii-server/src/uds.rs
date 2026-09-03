//! UDS transport (F2.8): socket dir resolution, stale-socket handling, and
//! the single `authorize_peer` decision point.

use std::path::PathBuf;

use pii_abi::PiiError;
use pii_proto::ErrorCode;
use pii_proto::RpcError;
use std::os::unix::fs::PermissionsExt;

/// Resolves the runtime dir (F2.8): `$PII_RUNTIME_DIR` →
/// `$XDG_RUNTIME_DIR/pii/<instance>` → `/run/user/<uid>/pii/<instance>`.
///
/// # Errors
/// [`PiiError::NotFound`] when no candidate is usable.
#[must_use]
pub fn runtime_dir(instance: &str) -> Option<PathBuf> {
    if let Some(d) = pii_core::env_var("PII_RUNTIME_DIR") {
        return Some(PathBuf::from(d));
    }
    if let Some(x) = pii_core::env_var("XDG_RUNTIME_DIR") {
        return Some(PathBuf::from(x).join("pii").join(instance));
    }
    let uid = rustix_uid();
    Some(PathBuf::from(format!("/run/user/{uid}/pii/{instance}")))
}

fn rustix_uid() -> u32 {
    // rustix is intentionally not a dependency of this build; `nobody`-style
    // uid discovery goes through the libc-free path: `id -u` is overkill, so
    // fall back to 0 when `/proc/self/status` cannot be read.
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("Uid:") {
                let first = rest.split_whitespace().next().unwrap_or("0");
                return first.parse().unwrap_or(0);
            }
        }
    }
    0
}

/// The single peercred decision point (F2.8), unit-testable.
///
/// # Errors
/// [`RpcError`] code 101 when the peer uid differs from the daemon uid.
pub fn authorize_peer(daemon_uid: u32, peer_uid: u32) -> Result<(), RpcError> {
    if daemon_uid == peer_uid {
        Ok(())
    } else {
        Err(RpcError::new(
            ErrorCode::Unauthorized,
            format!("refused uid {peer_uid}"),
        ))
    }
}

/// Prepares the control socket path: ensures the dir exists (0700) and
/// unlinks a stale socket only when no live daemon answers.
///
/// # Errors
/// [`PiiError::Internal`] on filesystem failure; [`PiiError::Busy`] when a
/// live daemon is already listening.
pub fn prepare_socket(dir: &std::path::Path) -> Result<PathBuf, PiiError> {
    std::fs::create_dir_all(dir)
        .map_err(|e| PiiError::Internal(format!("mkdir {}: {e}", dir.display())))?;
    // F10 (opus review): the control dir must be 0700 so the socket inside
    // it is not world-reachable through a permissive parent.
    #[allow(clippy::disallowed_methods)] // reason: dir mode 0700, not a Command site
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    let sock = dir.join("control.sock");
    if sock.exists() {
        match std::os::unix::net::UnixStream::connect(&sock) {
            Ok(_) => {
                return Err(PiiError::Internal(String::from(
                    "a live daemon is already listening on control.sock",
                )));
            }
            Err(_) => {
                // Stale socket: safe to remove (we created nothing prior).
                let _ = std::fs::remove_file(&sock);
            }
        }
    }
    Ok(sock)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use pii_proto::ErrorCode;

    #[test]
    fn peercred_rejects_foreign_uid() {
        let err = super::authorize_peer(0, 65534).unwrap_err();
        assert_eq!(err.code, i32::from(ErrorCode::Unauthorized));
        assert!(err.message.contains("65534"));
        assert!(super::authorize_peer(0, 0).is_ok());
    }
}
