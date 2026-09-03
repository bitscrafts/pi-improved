//! Sandbox profiles (F3.9/F3.10): process-wide Landlock confinement applied
//! in the re-exec'd `pii sandbox-exec` child, with a fail-closed
//! self-verification probe.

use std::path::{Path, PathBuf};

use landlock::{
    Access, AccessFs, PathBeneath, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus,
};
use pii_abi::PiiError;

use pii_core::env_var;

/// Which confinement profile to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SandboxProfileName {
    /// No confinement (explicitly chosen, never the fallback).
    #[default]
    Off,
    /// Read `/`; read-write the workspace, store dir, runtime dir, `$TMPDIR`,
    /// and the openable device files.
    Workspace,
}

/// The wire string for a profile.
#[must_use]
pub fn profile_name(p: SandboxProfileName) -> &'static str {
    match p {
        SandboxProfileName::Off => "off",
        SandboxProfileName::Workspace => "workspace",
    }
}

/// Parses a profile name from the CLI.
///
/// # Errors
/// [`PiiError::Invalid`] for an unknown name.
pub fn parse_profile(s: &str) -> Result<SandboxProfileName, PiiError> {
    match s {
        "off" => Ok(SandboxProfileName::Off),
        "workspace" => Ok(SandboxProfileName::Workspace),
        other => Err(PiiError::Invalid(format!(
            "unknown sandbox profile '{other}' (off|workspace)"
        ))),
    }
}

/// Applies the `workspace` profile process-wide and verifies enforcement
/// (F3.10). Fail-closed: any doubt aborts.
///
/// # Errors
/// [`PiiError::Denied`] when enforcement cannot be positively verified —
/// the caller must exit 4 before serving anything.
pub fn apply_workspace(
    workspace: &Path,
    extra_rw: &[PathBuf],
    probe_dir: &Path,
) -> Result<(), PiiError> {
    if env_var("PII_SANDBOX_FORCE_UNSUPPORTED").as_deref() == Some("1") {
        return Err(unverifiable());
    }

    let abi = landlock::ABI::V2;
    let all = AccessFs::from_all(abi);
    let read = AccessFs::from_read(abi);
    let ruleset = Ruleset::default()
        .handle_access(all)
        .map_err(|e| PiiError::Internal(format!("sandbox: {e}")))?
        .create()
        .map_err(|e| PiiError::Internal(format!("sandbox: {e}")))?;
    let mut rules = ruleset;
    for (path, access) in ro_paths()
        .into_iter()
        .map(|p| (p, read))
        .chain(rw_paths(workspace, extra_rw).into_iter().map(|p| (p, all)))
    {
        if let Ok(fd) = landlock::PathFd::new(&path) {
            rules = rules
                .add_rule(PathBeneath::new(fd, access))
                .map_err(|e| PiiError::Internal(format!("sandbox: {e}")))?;
        }
    }
    let status = rules
        .restrict_self()
        .map_err(|e| PiiError::Internal(format!("sandbox: {e}")))?;
    if status.ruleset != RulesetStatus::FullyEnforced || !status.no_new_privs {
        return Err(unverifiable());
    }
    // The probe MUST fail now; a success means confinement is not real.
    let probe = probe_dir.join(format!(".pii-sandbox-probe-{}", std::process::id()));
    if std::fs::write(&probe, b"probe").is_ok() {
        let _ = std::fs::remove_file(&probe);
        return Err(unverifiable());
    }
    Ok(())
}

fn unverifiable() -> PiiError {
    PiiError::Internal(String::from("sandbox: enforcement could not be verified"))
}

fn ro_paths() -> Vec<PathBuf> {
    vec![PathBuf::from("/"), PathBuf::from("/dev")]
}

fn rw_paths(workspace: &Path, extra_rw: &[PathBuf]) -> Vec<PathBuf> {
    let mut v = vec![workspace.to_path_buf()];
    v.extend(extra_rw.iter().cloned());
    for dev in ["/dev/null", "/dev/zero", "/dev/urandom", "/dev/tty"] {
        let p = PathBuf::from(dev);
        if p.exists() {
            v.push(p);
        }
    }
    if let Some(tmp) = env_var("TMPDIR").map(PathBuf::from) {
        v.push(tmp);
    }
    v
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{SandboxProfileName, parse_profile, profile_name};

    #[test]
    fn profile_names_roundtrip() {
        assert_eq!(profile_name(SandboxProfileName::Off), "off");
        assert_eq!(profile_name(SandboxProfileName::Workspace), "workspace");
        assert!(parse_profile("workspace").is_ok());
        assert!(parse_profile("bogus").is_err());
    }
}
