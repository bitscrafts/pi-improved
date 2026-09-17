//! UDS transport (F2.8): socket dir handling, the single `authorize_peer`
//! decision point, and the testkit bind/connect.
//!
//! Client-facing helpers (`runtime_dir`, `client_connect`) live in `pii-uds`
//! (B2, 011 A.11) so client crates (`pii-tui`, `pii-web`) do not pull the
//! daemon tree into their dependency graph.

use pii_abi::PiiError;
use pii_proto::ErrorCode;
use pii_proto::RpcError;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

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

/// Testkit (net-check allowlist lives in this file): binds a 0600 UDS socket
/// and returns it. Only `#[cfg(test)]` integration tests use this; the gate
/// scans tests too, so the bind must live here.
///
/// # Errors
/// [`PiiError::Internal`] on bind/permission failure.
#[allow(clippy::disallowed_methods)] // reason: sanctioned UDS bind site (F2.13)
pub fn test_bind_socket(sock_path: &std::path::Path) -> Result<tokio::net::UnixListener, PiiError> {
    let listener = tokio::net::UnixListener::bind(sock_path)
        .map_err(|e| PiiError::Internal(format!("bind {}: {e}", sock_path.display())))?;
    std::fs::set_permissions(sock_path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| PiiError::Internal(format!("chmod {}: {e}", sock_path.display())))?;
    Ok(listener)
}

/// Testkit: connects a client to a test socket (same crate file as the
/// bind, so the gate's single allowlist entry covers both).
///
/// # Errors
/// [`PiiError::Internal`] on connect failure.
pub async fn test_connect_socket(
    sock_path: &std::path::Path,
) -> Result<tokio::net::UnixStream, PiiError> {
    tokio::net::UnixStream::connect(sock_path)
        .await
        .map_err(|e| PiiError::Internal(format!("connect {}: {e}", sock_path.display())))
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
