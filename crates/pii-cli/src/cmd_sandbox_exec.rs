//! `pii sandbox-exec --profile <p> --workspace <d> -- <argv...>` (F5.5,
//! F3.10). Applies Landlock confinement process-wide, verifies enforcement
//! fail-closed, then `exec`s the child — no `unsafe`, so a plain spawned
//! child (not `pre_exec`) inherits the confinement via `execve`.

use std::path::PathBuf;
use std::process::ExitCode;

/// `pii sandbox-exec --profile <off|workspace> --workspace <dir> -- <argv…>`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_sandbox_exec(args: &mut std::env::ArgsOs) -> ExitCode {
    let mut profile = None;
    let mut workspace = None;
    let mut child_argv: Vec<String> = Vec::new();
    while let Some(a) = args.next() {
        let a = a.to_str().unwrap_or_default().to_owned();
        match a.as_str() {
            "--profile" => profile = args.next().and_then(|v| v.into_string().ok()),
            "--workspace" => workspace = args.next().and_then(|v| v.into_string().ok()),
            "--" => {
                child_argv = args.filter_map(|v| v.into_string().ok()).collect();
                break;
            }
            other => {
                eprintln!("error: unknown flag '{other}'");
                return ExitCode::from(2);
            }
        }
    }
    let (Some(profile), Some(workspace)) = (profile, workspace) else {
        eprintln!("usage: pii sandbox-exec --profile <p> --workspace <d> -- <argv…>");
        return ExitCode::from(2);
    };
    let Some((prog, rest)) = child_argv.split_first() else {
        eprintln!("usage: pii sandbox-exec --profile <p> --workspace <d> -- <argv…>");
        return ExitCode::from(2);
    };
    let profile = match pii_tools::sandbox::parse_profile(&profile) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let workspace = PathBuf::from(workspace);

    if matches!(profile, pii_tools::sandbox::SandboxProfileName::Workspace)
        && let Err(e) =
            pii_tools::sandbox::apply_workspace(&workspace, &[], std::path::Path::new("/etc"))
    {
        eprintln!("{e}");
        return ExitCode::from(4);
    }

    #[allow(clippy::disallowed_methods)] // reason: exec inherits confinement
    // applied process-wide above; this is the re-exec'd child, the single
    // sanctioned direct spawn site outside ProcCap::run (spec F5.5/F3.10).
    let status = std::process::Command::new(prog)
        .args(rest)
        .current_dir(&workspace)
        .status();
    match status {
        Ok(s) => ExitCode::from(coerce_child_exit(s.code())),
        Err(e) => {
            eprintln!("error: spawn {prog}: {e}");
            ExitCode::from(2)
        }
    }
}

/// Coerces a child's raw exit code to the parent's exit code: masked to the
/// low 8 bits, with `1` as the fallback when the child was killed by a
/// signal (`ExitStatus::code()` returns `None`). Spec 74 R4 documents the
/// reality here: a wait status only encodes the low 8 bits of a child's
/// exit code, so codes >= 256 are not representable — the mask-to-u8
/// coercion IS the documented behaviour, and the 128..=255 band must
/// survive it verbatim.
fn coerce_child_exit(code: Option<i32>) -> u8 {
    #[allow(clippy::cast_sign_loss)] // raw wait status is a positive small int
    code.map_or(1, |c| (c as u32 & 0xff) as u8)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;

    use super::coerce_child_exit;

    #[test]
    fn sandbox_exec_exit_code_coercion() {
        // Successful exit passes through, masked to 8 bits.
        let ok = ExitStatus::from_raw(0);
        assert_eq!(coerce_child_exit(ok.code()), 0);
        // Exit code 42 -> 42.
        let code = ExitStatus::from_raw(42 << 8);
        assert_eq!(coerce_child_exit(code.code()), 42);
        // Killed by SIGKILL (signal 9): code() is None, falls back to 1.
        let killed = ExitStatus::from_raw(9);
        assert!(killed.code().is_none());
        assert_eq!(coerce_child_exit(killed.code()), 1);
        // Killed by SIGINT (signal 2): same fallback.
        let sigint = ExitStatus::from_raw(2);
        assert!(sigint.code().is_none());
        assert_eq!(coerce_child_exit(sigint.code()), 1);
        // An out-of-range exit code (e.g. 300) is masked to its low 8 bits.
        let masked = ExitStatus::from_raw((0x012c & 0xff) << 8);
        assert_eq!(coerce_child_exit(masked.code()), 44); // 300 & 0xff
    }

    /// Spec 74 R4: the high band 128..=255 must survive the coercion
    /// verbatim — the case the table above lacked. A 7-bit collapse (e.g.
    /// masking with 0x7f) turns 255 into 127 silently; this pins that shut.
    /// Codes >= 256 are not representable (a wait status encodes 8 bits of
    /// exit code), so the documented u8 mask is what is asserted — not an
    /// impossible identity.
    #[test]
    fn sandbox_exec_exit_code_255_is_reported_verbatim() {
        let code = ExitStatus::from_raw(255 << 8);
        assert_eq!(code.code(), Some(255));
        assert_eq!(coerce_child_exit(code.code()), 255);
    }
}
