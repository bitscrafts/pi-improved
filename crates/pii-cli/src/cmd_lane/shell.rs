//! The lane's two thin shells (spec 317 part 1, spec 319 part 2): `pii lane plan` and `pii lane run` read the
//! repository and the environment, call `plan()` and `run::run()`, and write what they return. No decisions here.
use super::args::Args;
use super::clock;
use super::devbox;
use super::env::{self, LaneEnv};
use super::git;
use super::now_secs;
use super::plan;
use super::preflight::{self, PreflightRepo};
use super::run;
use super::transport;
use super::{Devbox, DryDevbox, LanePlan};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const LANE_USAGE: &str = "usage: pii lane plan <N> [--resume <sid>] [<extra-brief>] [--dry-run] [--root <dir>] [--agent-root <dir>] [--emit]";

/// The lane's two shells, `pii lane plan …` (no start) and `pii lane run …` (the start and the wait).
pub(crate) fn cmd_lane(args: &mut std::env::ArgsOs) -> ExitCode {
    let mut it = args.map(|a| a.to_string_lossy().into_owned());
    match it.next().as_deref() {
        Some("plan") => lane(&mut it, false),
        Some("run") => lane(&mut it, true),
        _ => {
            let _ = writeln!(std::io::stderr(), "{LANE_USAGE}");
            ExitCode::from(2)
        }
    }
}

fn exit_code(code: i32) -> ExitCode {
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

/// `_tmp/devbox-test/preflight-<ts>-<rand>`: the throwaway repo the judge preflight runs against (spec 319 R2).
fn preflight_repo(root: &Path) -> Result<PreflightRepo, String> {
    const FAIL: &str = "ABORT: could not make the preflight repo: nothing started";
    let ts = now_secs();
    let rand = format!(
        "{:06x}",
        ((u64::from(std::process::id()) * 2_654_435_761) ^ ts) & 0xff_ffff
    );
    let dir = format!("preflight-{ts}-{rand}");
    let repo = root.join("_tmp/devbox-test").join(&dir).join("repo");
    fs::create_dir_all(repo.join("specs")).map_err(|_| FAIL.to_owned())?;
    let prel = format!("_tmp/preflight-{ts}");
    fs::write(repo.join("specs/preflight.md"), preflight::spec_text(&prel))
        .map_err(|_| FAIL.to_owned())?;
    let rev = git::run(&repo, &["init", "-q"])
        .and_then(|_| git::run(&repo, &["add", "specs"]))
        .and_then(|_| {
            git::run(
                &repo,
                &[
                    "-c",
                    "user.name=pii-preflight",
                    "-c",
                    "user.email=preflight@localhost",
                    "commit",
                    "-qm",
                    "base",
                ],
            )
        })
        .and_then(|_| git::run(&repo, &["rev-parse", "HEAD"]))
        .map(|o| String::from_utf8_lossy(&o.stdout).trim_end().to_owned())
        .ok_or_else(|| FAIL.to_owned())?;
    Ok(PreflightRepo { dir, rev, ts })
}

/// The shared lane body: `start` false is the plan (dry or refused), true runs the lane end to end.
fn lane(it: &mut impl Iterator<Item = String>, start: bool) -> ExitCode {
    let (mut root_arg, mut agent_arg, mut emit) = (None, None, false);
    let mut words = Vec::new();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--root" => root_arg = it.next(),
            "--agent-root" => agent_arg = it.next(),
            "--emit" => emit = true,
            _ => words.push(a),
        }
    }
    let Ok(cwd) = std::env::current_dir() else {
        return ExitCode::from(2);
    };
    let Some(root) = root_arg.map(PathBuf::from).or_else(|| git::toplevel(&cwd)) else {
        let _ = writeln!(std::io::stderr(), "pii lane: not inside a git repository");
        return ExitCode::from(2);
    };
    if root.components().any(|c| c.as_os_str() == "_tmp") {
        let _ = writeln!(
            std::io::stderr(),
            "pii lane: {} is inside _tmp/: refused",
            root.display()
        );
        return ExitCode::from(2);
    }
    let agent_root = agent_arg.map_or_else(|| root.clone(), PathBuf::from);
    let parsed = Args::parse(&words);
    let real = start && !parsed.dry;
    for dir in ["_tmp/claude-batchC", "_tmp/devbox-test/graphs"] {
        let _ = fs::create_dir_all(root.join(dir));
    }
    let env = LaneEnv::read();
    let snap = git::gather(&root, &agent_root, parsed.brief.as_deref());
    let ssh = transport::Ssh::new(
        env.ssh.clone().unwrap_or_else(|| "ssh".to_owned()),
        env.devbox_host
            .clone()
            .unwrap_or_else(|| env::DEFAULT_HOST.to_owned()),
    );
    let container = env
        .container
        .clone()
        .unwrap_or_else(|| "pii-devbox".to_owned());
    let cfg = root.join("_tmp/devbox-test/claude").display().to_string();
    let dev = devbox::SshDevbox {
        t: &ssh,
        container: &container,
        cfg: &cfg,
    };
    let devbox: &dyn Devbox = if real { &dev } else { &DryDevbox };
    let lp = plan(&env, &parsed, &snap, devbox, now_secs());
    // A plan that would start is refused before anything is written (`pii lane run` is the start).
    if !real && !parsed.dry && lp.exit == 0 {
        let _ = writeln!(
            std::io::stderr(),
            "pii lane plan: a run starts with `pii lane run`: nothing started"
        );
        return ExitCode::from(2);
    }
    for (path, bytes) in &lp.files {
        if let Err(e) = fs::write(path, bytes) {
            let _ = writeln!(
                std::io::stderr(),
                "pii lane: cannot write {}: {e}",
                path.display()
            );
            return ExitCode::from(2);
        }
    }
    let _ = std::io::stdout().write_all(&lp.stdout);
    let _ = std::io::stderr().write_all(&lp.stderr);
    if emit {
        let fields = [
            format!("TAG={}", lp.tag),
            format!("GRAPH_NAME={}", lp.graph_name),
            format!("LOG={}", lp.log.display()),
            format!(
                "INPUT={}",
                lp.input
                    .as_deref()
                    .map_or_else(String::new, |i| i.display().to_string())
            ),
            format!(
                "DEADLINE_S={}",
                lp.deadline_s.map_or_else(String::new, |d| d.to_string())
            ),
            format!("SID={}", lp.sid.clone().unwrap_or_default()),
        ];
        for f in fields {
            let _ = writeln!(std::io::stdout(), "{f}");
        }
    }
    if !real || lp.exit != 0 {
        return exit_code(lp.exit);
    }
    run_lane(&lp, &env, &root, &agent_root, &ssh)
}

/// The real host effects of the run half, then the files and log appends `run::run` returns.
fn run_lane(
    lp: &LanePlan,
    env: &LaneEnv,
    root: &Path,
    agent_root: &Path,
    ssh: &transport::Ssh,
) -> ExitCode {
    let clock = clock::Real::new();
    let daemon_sh = env
        .daemon_sh
        .clone()
        .map_or_else(|| root.join("modules/devbox/daemon.sh"), PathBuf::from);
    let daemon_argv = [
        "bash".to_owned(),
        daemon_sh.display().to_string(),
        "up".to_owned(),
    ];
    let daemon = || transport::local(&daemon_argv, root);
    let specs_status = || git::porcelain(root, &["specs"]);
    let status = || git::porcelain(root, &[]);
    let preflight_repo = || preflight_repo(root);
    let graphs = root.join("_tmp/devbox-test/graphs");
    let graph_copies = ["spec", "docs", "preflight"]
        .iter()
        .filter_map(|g| {
            let file = format!("{g}.toml");
            let bytes = fs::read(root.join("graphs").join(&file)).ok()?;
            Some((graphs.join(&file), bytes))
        })
        .collect();
    let io = run::Io {
        t: ssh,
        clock: &clock,
        daemon: &daemon,
        specs_status: &specs_status,
        status: &status,
        preflight_repo: &preflight_repo,
        graph_copies,
    };
    let paths = run::Paths {
        c: root.join("_tmp/devbox-test").display().to_string(),
        agent_root: agent_root.display().to_string(),
        logd: lp.log.parent().map_or_else(PathBuf::new, Path::to_path_buf),
        scratch: root.join("_tmp").display().to_string(),
    };
    let node_timeout = env
        .node_timeout_ms
        .as_deref()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3_000_000);
    let rep = run::run(lp, env, &paths, &io, node_timeout);
    for (path, bytes) in &rep.files {
        if let Err(e) = fs::write(path, bytes) {
            let _ = writeln!(
                std::io::stderr(),
                "pii lane: cannot write {}: {e}",
                path.display()
            );
            return ExitCode::from(2);
        }
    }
    if !rep.log_appends.is_empty() {
        let appended = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&lp.log)
            .and_then(|mut f| f.write_all(&rep.log_appends));
        if let Err(e) = appended {
            let _ = writeln!(
                std::io::stderr(),
                "pii lane: cannot append {}: {e}",
                lp.log.display()
            );
        }
    }
    let _ = std::io::stdout().write_all(&rep.stdout);
    let _ = std::io::stderr().write_all(&rep.stderr);
    exit_code(rep.exit)
}
