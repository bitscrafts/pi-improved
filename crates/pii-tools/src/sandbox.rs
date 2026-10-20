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

/// The grant a profile puts in force, in words — the ONE text `pii health`,
/// the startup event and the runbook all quote, pinned by the behaviour tests
/// in `pii-cli/tests/sandbox_grant.rs`.
#[must_use]
pub fn grant_policy(p: SandboxProfileName) -> &'static str {
    match p {
        SandboxProfileName::Off => {
            "sandbox=off: NO filesystem confinement in force (explicit opt-out); \
             the daemon can read and write anything its user can"
        }
        SandboxProfileName::Workspace => {
            "sandbox=workspace (landlock): WRITES confined to the workspace, store dir, \
             runtime dir, $TMPDIR, the container runtime state dirs \
             (HOME/.local/share/containers, XDG_RUNTIME_DIR/containers, XDG_RUNTIME_DIR/libpod \
             -- podman's graph root and locks; spec 228) and /dev/shm (podman's rootless lock; \
             spec 229), plus /dev/{null,zero,urandom,tty}; \
             READS are NOT confined (read-only access to / is granted, so /etc, /home and \
             every other readable path stay readable)"
        }
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

/// The container runtime's state dirs, granted read-write beside the
/// workspace (spec 228 R1).
///
/// Podman keeps its graph root and bolt DB under `HOME/.local/share/containers`
/// and its runroot and locks under `XDG_RUNTIME_DIR/containers` and
/// `XDG_RUNTIME_DIR/libpod`; the `workspace` profile granted none of them, so a
/// sandboxed daemon could not even inspect a present image (measured on lnx
/// 2026-10-20: `bolt_state.db` and `alive.lck` EACCES). Each dir is created
/// best-effort: a dir that cannot exist cannot be granted, and podman then
/// fails loudly on its own. `HOME` falls back to `/root` (the daemon's
/// documented default).
#[must_use]
pub fn podman_state_dirs() -> Vec<PathBuf> {
    let home = env_var("HOME")
        .filter(|h| !h.is_empty())
        .map_or_else(|| PathBuf::from("/root"), PathBuf::from);
    let xdg = env_var("XDG_RUNTIME_DIR")
        .filter(|d| !d.is_empty())
        .map_or_else(|| home.join(".run"), PathBuf::from);
    let dirs = vec![
        home.join(".local/share/containers"),
        xdg.join("containers"),
        xdg.join("libpod"),
        // Podman's rootless lock file is /dev/shm/libpod_rootless_lock_<uid> (podman #16119).
        // The profile grants /dev READ-ONLY, so this tmpfs must be named explicitly; without it
        // podman's fallback base is empty and it fails at /libpod_rootless_lock_<uid>
        // (measured on lnx 2026-10-20, M0 run r000000000209).
        PathBuf::from("/dev/shm"),
    ];
    for d in &dirs {
        // reason: best-effort, same contract as writable_dirs: an uncreatable dir is simply
        // not granted and the runtime's own failure names it.
        let _ = std::fs::create_dir_all(d);
    }
    dirs
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
    // Device nodes are not directories: granting the full directory-rights
    // set (MakeDir/RemoveDir/...) on them makes the kernel downgrade the
    // whole ruleset to PartiallyEnforced. File-content rights only.
    let file_rw = AccessFs::ReadFile | AccessFs::WriteFile;
    let ruleset = Ruleset::default()
        .handle_access(all)
        .map_err(|e| PiiError::Internal(format!("sandbox: {e}")))?
        .create()
        .map_err(|e| PiiError::Internal(format!("sandbox: {e}")))?;
    let mut rules = ruleset;
    for path in ro_paths() {
        if let Ok(fd) = landlock::PathFd::new(&path) {
            rules = rules
                .add_rule(PathBeneath::new(fd, read))
                .map_err(|e| PiiError::Internal(format!("sandbox: {e}")))?;
        }
    }
    for (path, is_dir) in rw_paths(workspace, extra_rw) {
        if let Ok(fd) = landlock::PathFd::new(&path) {
            let access = if is_dir { all } else { file_rw };
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
    // The out-grant probe MUST fail now; a success means confinement is not real.
    let probe = probe_dir.join(format!(".pii-sandbox-probe-{}", std::process::id()));
    if std::fs::write(&probe, b"probe").is_ok() {
        let _ = std::fs::remove_file(&probe);
        return Err(unverifiable());
    }
    // R3 (spec 228) control beside it: a write INSIDE a granted dir MUST succeed. A grant
    // whose every rule is inert (wrong fd, downgraded ruleset) passes the out-grant check
    // while granting nothing; without this control the confinement could be empty by
    // construction and every later refusal a coincidence.
    let in_grant = extra_rw
        .first()
        .cloned()
        .unwrap_or_else(|| workspace.join(".pii-in-grant-probe-dir"));
    let _ = std::fs::create_dir_all(&in_grant);
    let in_probe = in_grant.join(format!(".pii-sandbox-probe-in-{}", std::process::id()));
    if std::fs::write(&in_probe, b"probe").is_err() {
        return Err(PiiError::Internal(format!(
            "sandbox: the in-grant probe could not write {} — the grant is inert",
            in_probe.display()
        )));
    }
    let _ = std::fs::remove_file(&in_probe);
    Ok(())
}

fn unverifiable() -> PiiError {
    PiiError::Internal(String::from("sandbox: enforcement could not be verified"))
}

fn ro_paths() -> Vec<PathBuf> {
    vec![PathBuf::from("/"), PathBuf::from("/dev")]
}

fn rw_paths(workspace: &Path, extra_rw: &[PathBuf]) -> Vec<(PathBuf, bool)> {
    let mut v: Vec<(PathBuf, bool)> = vec![(workspace.to_path_buf(), true)];
    v.extend(extra_rw.iter().cloned().map(|p| (p, true)));
    for dev in ["/dev/null", "/dev/zero", "/dev/urandom", "/dev/tty"] {
        let p = PathBuf::from(dev);
        if p.exists() {
            v.push((p, false));
        }
    }
    if let Some(tmp) = env_var("TMPDIR")
        .filter(|t| !t.is_empty())
        .map(PathBuf::from)
    {
        v.push((tmp, true));
    }
    v
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{SandboxProfileName, grant_policy, parse_profile, podman_state_dirs, profile_name};

    #[test]
    fn podman_state_dirs_are_the_three_runtime_state_locations() {
        // No set_var from a test (edition-2024 unsafe): read what the process carries.
        let dirs = podman_state_dirs();
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        let xdg = std::env::var("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from(home.clone()).join(".run"));
        assert_eq!(
            dirs,
            vec![
                std::path::PathBuf::from(&home).join(".local/share/containers"),
                xdg.join("containers"),
                xdg.join("libpod"),
                std::path::PathBuf::from("/dev/shm"),
            ],
            "the grant is exactly the documented dirs, nothing invented"
        );
        for d in &dirs {
            assert!(d.is_dir(), "created best-effort: {d:?}");
        }
        // Control: the helper must NOT grow arbitrary members.
        assert_eq!(dirs.len(), 4);
    }

    #[test]
    fn grant_policy_names_the_podman_state_dirs() {
        let ws = grant_policy(SandboxProfileName::Workspace);
        assert!(
            ws.contains("HOME/.local/share/containers")
                && ws.contains("XDG_RUNTIME_DIR/containers")
                && ws.contains("XDG_RUNTIME_DIR/libpod")
                && ws.contains("/dev/shm"),
            "{ws}"
        );
    }

    #[test]
    fn grant_policy_states_the_real_read_and_write_rule() {
        let ws = grant_policy(SandboxProfileName::Workspace);
        assert!(ws.contains("WRITES confined"), "{ws}");
        assert!(ws.contains("READS are NOT confined"), "{ws}");
        let off = grant_policy(SandboxProfileName::Off);
        assert!(off.contains("NO filesystem confinement"), "{off}");
        assert_ne!(ws, off);
    }

    #[test]
    fn profile_names_roundtrip() {
        assert_eq!(profile_name(SandboxProfileName::Off), "off");
        assert_eq!(profile_name(SandboxProfileName::Workspace), "workspace");
        assert!(parse_profile("workspace").is_ok());
        assert!(parse_profile("bogus").is_err());
    }
}
