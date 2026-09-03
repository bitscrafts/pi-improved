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
        Ok(s) => {
            #[allow(clippy::cast_sign_loss)]
            let code = s.code().unwrap_or(1) as u32 & 0xff;
            ExitCode::from(u8::try_from(code).unwrap_or(1))
        }
        Err(e) => {
            eprintln!("error: spawn {prog}: {e}");
            ExitCode::from(2)
        }
    }
}
