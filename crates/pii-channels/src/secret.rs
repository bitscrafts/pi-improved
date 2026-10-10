//! Where a channel's credential comes from (spec 112 R6): an environment
//! variable, or a file that only its owner can read. Never a config value.

use pii_abi::PiiError;

/// Resolves a credential: the env var `env` when set and non-empty, else the
/// trimmed contents of `file`. `Ok(None)` when neither names a value.
///
/// # Errors
/// [`PiiError::Invalid`] when `file` is readable by group/other (not 0600) or
/// cannot be read — a loose secret file is refused, not used.
pub fn resolve(env: Option<&str>, file: Option<&str>) -> Result<Option<String>, PiiError> {
    if let Some(v) = env.and_then(pii_core::env_var).filter(|v| !v.is_empty()) {
        return Ok(Some(v));
    }
    let Some(path) = file else {
        return Ok(None);
    };
    check_mode(path)?;
    let raw = std::fs::read_to_string(path)
        .map_err(|e| PiiError::Invalid(format!("secret file '{path}': {e}")))?;
    let v = raw.trim();
    Ok((!v.is_empty()).then(|| v.to_owned()))
}

#[cfg(unix)]
fn check_mode(path: &str) -> Result<(), PiiError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)
        .map_err(|e| PiiError::Invalid(format!("secret file '{path}': {e}")))?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        return Err(PiiError::Invalid(format!(
            "secret file '{path}' has mode {:o}; it must be 0600",
            mode & 0o777
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_mode(_path: &str) -> Result<(), PiiError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::resolve;
    use std::os::unix::fs::PermissionsExt;

    fn scratch() -> tempfile::TempDir {
        let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../_tmp"));
        std::fs::create_dir_all(root).unwrap();
        tempfile::Builder::new()
            .prefix("secret-")
            .tempdir_in(root)
            .unwrap()
    }

    #[test]
    fn file_must_be_0600_and_is_trimmed() {
        let d = scratch();
        let p = d.path().join("tok");
        std::fs::write(&p, "abc\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        let path = p.to_str().unwrap();
        let err = resolve(None, Some(path)).unwrap_err();
        assert!(err.to_string().contains("0600"), "{err}");
        assert!(!err.to_string().contains("abc"));
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(resolve(None, Some(path)).unwrap().as_deref(), Some("abc"));
    }

    #[test]
    fn nothing_configured_is_none() {
        assert_eq!(resolve(None, None).unwrap(), None);
        assert_eq!(resolve(Some("PII_TEST_UNSET_VAR_112"), None).unwrap(), None);
    }
}
