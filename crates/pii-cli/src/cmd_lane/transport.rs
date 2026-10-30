//! The devbox transport (spec 319 R1): one trait, the real ssh implementation, and the local runner the daemon
//! call uses. Every process goes through `Ctx::proc()`.
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use std::path::Path;
use std::time::Duration;

/// Longest a single ssh or local call may run. The shell has no limit; this only stops a hung connection.
const CALL_TIMEOUT: Duration = Duration::from_secs(3600);

/// What a call returned: the exit code and both streams, raw.
#[derive(Debug, Default, Clone)]
pub(crate) struct Out {
    pub(crate) code: i32,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

/// One remote command. `argv_after_host` is the single string ssh hands to the remote shell.
pub(crate) trait Transport {
    fn exec(&self, argv_after_host: &str) -> Out;
}

/// The real transport: `[ssh] -o BatchMode=yes -o ConnectTimeout=15 <host> <one string>`.
pub(crate) struct Ssh {
    bin: String,
    host: String,
    ctx: Ctx,
}

impl Ssh {
    pub(crate) fn new(bin: String, host: String) -> Self {
        let caps = CapSet {
            kinds: CapKind::PROC,
            proc_argv0: ArgvScope::new(vec![bin.clone()]),
            ..CapSet::default()
        };
        Self {
            bin,
            host,
            ctx: Ctx::root(caps, Budget::unlimited()),
        }
    }
}

impl Transport for Ssh {
    fn exec(&self, argv_after_host: &str) -> Out {
        let argv = [
            self.bin.as_str(),
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=15",
            self.host.as_str(),
            argv_after_host,
        ]
        .map(str::to_owned);
        run_proc(&self.ctx, &argv)
    }
}

/// Runs a local command with stderr merged after stdout (the shell's `2>&1` for the daemon call).
pub(crate) fn local(argv: &[String], cwd: &Path) -> Out {
    let caps = CapSet {
        kinds: CapKind::PROC,
        proc_argv0: ArgvScope::new(vec![argv.first().cloned().unwrap_or_default()]),
        ..CapSet::default()
    };
    let ctx = Ctx::root(caps, Budget::unlimited());
    match ctx.proc().and_then(|p| p.run(argv, cwd, CALL_TIMEOUT)) {
        Ok(o) => Out {
            code: o.exit_code,
            stdout: [o.stdout, o.stderr].concat(),
            stderr: Vec::new(),
        },
        Err(e) => Out {
            code: 127,
            stdout: format!("{e}\n").into_bytes(),
            stderr: Vec::new(),
        },
    }
}

fn run_proc(ctx: &Ctx, argv: &[String]) -> Out {
    match ctx
        .proc()
        .and_then(|p| p.run(argv, Path::new("."), CALL_TIMEOUT))
    {
        Ok(o) => Out {
            code: o.exit_code,
            stdout: o.stdout,
            stderr: o.stderr,
        },
        Err(e) => Out {
            code: 255,
            stdout: Vec::new(),
            stderr: format!("{e}\n").into_bytes(),
        },
    }
}
