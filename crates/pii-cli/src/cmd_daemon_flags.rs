//! `pii daemon` command-line parsing: the help text, the flag set and its parser.
#![allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form

use std::path::PathBuf;

use pii_tools::sandbox::SandboxProfileName;

use crate::daemon_log::{LogFlags, parse_switch};

/// `pii daemon --help` text: every real flag, then the config/env surface.
pub(crate) const HELP: &str = "\
pii daemon - run the control daemon in the foreground

USAGE: pii daemon [FLAGS]

FLAGS:
  --store <path>             redb store file (default: <runtime dir>/store.redb); its directory owns the
                             durable stores (channel, timer) when this flag is given
  --config <path>            config TOML (default: $PII_CONFIG, else ~/.pii/pii.toml)
  --workspace <dir>          workspace root the sandbox and tools are scoped to (default: cwd)
  --sandbox <off|workspace>  sandbox profile (default: workspace; exit 4 if it cannot be enforced;
                             'off' is the explicit opt-out and runs UNCONFINED)
  --log-file <path>          append daemon events as JSON lines to <path> (default: off)
                             (denials always go to <store dir>/audit.log, rotated like the log)
  --log-rotate-bytes <n>     rotate the log file past <n> bytes (default: 10485760)
  --log-keep-files <n>       rotated log files to keep besides the live one (default: 5)
  --observer <on|off>        'off' detaches the observer fan-out; MEASUREMENT ONLY (default on)
  --audit <on|off>           'off' stops the audit feed; MEASUREMENT ONLY (default on)
  --foreground               accepted for supervisors; the daemon never forks
  --help                     print this help and exit 0

The runtime directory comes from $PII_RUNTIME_DIR, else [core].runtime_dir.
Each --log-* flag overrides the matching [core] key (log_file, log_rotate_bytes,
log_keep_files). Run trajectories are controlled by [core].trajectory_dir or
$PII_TRAJECTORY_DIR (default: off). See examples/pii.production.toml.
";

/// The parsed `pii daemon` command line.
#[derive(Debug)]
pub(crate) struct DaemonFlags {
    pub(crate) workspace: PathBuf,
    pub(crate) sandbox: SandboxProfileName,
    pub(crate) store: Option<PathBuf>,
    pub(crate) config: Option<PathBuf>,
    pub(crate) log: LogFlags,
    pub(crate) observer: Option<bool>,
    pub(crate) audit: Option<bool>,
}

/// Why parsing stopped without producing [`DaemonFlags`].
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Parse {
    Help,
    Usage(String),
}

/// A value-taking flag's value, or a usage error naming the flag.
fn flag_value(
    args: &mut dyn Iterator<Item = std::ffi::OsString>,
    flag: &str,
) -> Result<String, Parse> {
    args.next()
        .and_then(|a| a.into_string().ok())
        .ok_or_else(|| Parse::Usage(format!("{flag} needs a value")))
}

/// A measurement switch's `on|off` value (spec 116).
fn switch_value(
    args: &mut dyn Iterator<Item = std::ffi::OsString>,
    flag: &str,
) -> Result<Option<bool>, Parse> {
    parse_switch(flag, &flag_value(args, flag)?)
        .map(Some)
        .map_err(Parse::Usage)
}

/// Parses a numeric flag value.
fn flag_number<T: std::str::FromStr>(
    args: &mut dyn Iterator<Item = std::ffi::OsString>,
    flag: &str,
) -> Result<T, Parse> {
    let v = flag_value(args, flag)?;
    v.parse()
        .map_err(|_| Parse::Usage(format!("{flag} needs a non-negative integer, got '{v}'")))
}

pub(crate) fn parse_flags(
    args: &mut dyn Iterator<Item = std::ffi::OsString>,
) -> Result<DaemonFlags, Parse> {
    let mut f = DaemonFlags {
        workspace: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        sandbox: SandboxProfileName::Workspace,
        store: None,
        config: None,
        log: LogFlags::default(),
        observer: None,
        audit: None,
    };
    while let Some(flag) = args.next() {
        let flag = flag.to_str().unwrap_or_default().to_owned();
        match flag.as_str() {
            "--help" | "-h" => return Err(Parse::Help),
            "--workspace" => f.workspace = PathBuf::from(flag_value(args, "--workspace")?),
            "--store" => f.store = Some(PathBuf::from(flag_value(args, "--store")?)),
            "--config" => f.config = Some(PathBuf::from(flag_value(args, "--config")?)),
            "--sandbox" => {
                let v = flag_value(args, "--sandbox")?;
                f.sandbox = pii_tools::sandbox::parse_profile(&v)
                    .map_err(|e| Parse::Usage(e.to_string()))?;
            }
            "--observer" => f.observer = switch_value(args, "--observer")?,
            "--audit" => f.audit = switch_value(args, "--audit")?,
            "--log-file" => f.log.file = Some(PathBuf::from(flag_value(args, "--log-file")?)),
            "--log-rotate-bytes" => {
                f.log.rotate_bytes = Some(flag_number(args, "--log-rotate-bytes")?);
            }
            "--log-keep-files" => f.log.keep_files = Some(flag_number(args, "--log-keep-files")?),
            // P4.5 (spec 18 R1): explicit no-op — the daemon is already
            // foreground. The flag documents the supervisor contract and
            // future-proofs a daemonize path; it does not change behavior.
            "--foreground" => {}
            other => return Err(Parse::Usage(format!("unknown flag '{other}'"))),
        }
    }
    Ok(f)
}
