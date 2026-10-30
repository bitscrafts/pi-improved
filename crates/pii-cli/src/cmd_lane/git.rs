//! The git and working-tree facts the lane reads, gathered once through `Ctx::proc()` (spec 317).
//! This is the thin IO side: `plan()` receives a `RepoSnapshot` and does no IO of its own.

use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx, ProcOutput};
use std::path::{Path, PathBuf};
use std::time::Duration;

const GIT_TIMEOUT: Duration = Duration::from_secs(60);

/// One graph's state against `HEAD` and on disk.
#[derive(Debug, Default)]
pub(crate) struct Graph {
    /// `git diff --quiet HEAD -- <graph>` failed or reported a change (any git failure counts).
    pub(crate) differs: bool,
    /// Untracked and not ignored.
    pub(crate) untracked: bool,
    /// The working-tree bytes; None when the path is not a regular file.
    pub(crate) bytes: Option<Vec<u8>>,
}

/// Everything `plan()` reads from the repository.
#[derive(Debug, Default)]
pub(crate) struct RepoSnapshot {
    /// Where files are read and written.
    pub(crate) root: PathBuf,
    /// The path the devbox sees (the brief's `$LOGD`, the input's `workdir`).
    pub(crate) agent_root: PathBuf,
    /// `git status --porcelain`, raw.
    pub(crate) status: Vec<u8>,
    /// `git rev-parse HEAD`, None on failure.
    pub(crate) head: Option<String>,
    pub(crate) spec: Graph,
    pub(crate) docs: Graph,
    pub(crate) preflight: Graph,
    /// Entries of `specs/`: name and whether it is a regular file.
    pub(crate) specs: Vec<(String, bool)>,
    /// The extra brief, read when one was named; Err carries the `cat:` line.
    pub(crate) extra: Option<Result<Vec<u8>, String>>,
}

fn ctx() -> Ctx {
    let caps = CapSet {
        kinds: CapKind::PROC,
        proc_argv0: ArgvScope::new(vec![String::from("git")]),
        ..CapSet::default()
    };
    Ctx::root(caps, Budget::unlimited())
}

fn git(ctx: &Ctx, cwd: &Path, args: &[&str]) -> Option<ProcOutput> {
    let full: Vec<String> = std::iter::once("git")
        .chain(args.iter().copied())
        .map(String::from)
        .collect();
    ctx.proc().ok()?.run(&full, cwd, GIT_TIMEOUT).ok()
}

/// `git rev-parse --show-toplevel` from `cwd`.
pub(crate) fn toplevel(cwd: &Path) -> Option<PathBuf> {
    let out = git(&ctx(), cwd, &["rev-parse", "--show-toplevel"])?;
    (out.exit_code == 0).then(|| {
        PathBuf::from(
            String::from_utf8_lossy(&out.stdout)
                .trim_end_matches('\n')
                .to_owned(),
        )
    })
}

/// The error text of an I/O failure as the shell's tools print it (no `(os error N)`).
fn io_text(e: &std::io::Error) -> String {
    let s = e.to_string();
    s.split(" (os error").next().unwrap_or_default().to_owned()
}

/// Gathers the snapshot. `extra` is the extra-brief path as given (relative to `root`).
pub(crate) fn gather(root: &Path, agent_root: &Path, extra: Option<&str>) -> RepoSnapshot {
    let ctx = ctx();
    let graph = |name: &str| {
        let rel = format!("graphs/{name}.toml");
        let differs = git(&ctx, root, &["diff", "--quiet", "HEAD", "--", &rel])
            .is_none_or(|o| o.exit_code != 0);
        let untracked = git(
            &ctx,
            root,
            &["ls-files", "--others", "--exclude-standard", "--", &rel],
        )
        .is_some_and(|o| !o.stdout.is_empty());
        Graph {
            differs,
            untracked,
            bytes: std::fs::read(root.join(&rel)).ok(),
        }
    };
    let head = git(&ctx, root, &["rev-parse", "HEAD"])
        .filter(|o| o.exit_code == 0)
        .map(|o| String::from_utf8_lossy(&o.stdout).trim_end().to_owned());
    let specs = std::fs::read_dir(root.join("specs"))
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| {
                    let name = e.file_name().into_string().ok()?;
                    Some((name, e.path().is_file()))
                })
                .collect()
        })
        .unwrap_or_default();
    let extra = extra
        .filter(|p| !p.is_empty())
        .map(|p| std::fs::read(root.join(p)).map_err(|e| format!("cat: {p}: {}", io_text(&e))));
    RepoSnapshot {
        root: root.to_path_buf(),
        agent_root: agent_root.to_path_buf(),
        status: git(&ctx, root, &["status", "--porcelain"])
            .map(|o| o.stdout)
            .unwrap_or_default(),
        head,
        spec: graph("spec"),
        docs: graph("docs"),
        preflight: graph("preflight"),
        specs,
        extra,
    }
}

/// One git call from `root` (spec 319: the run half's preflight repo and `git status` reads).
pub(crate) fn run(root: &Path, args: &[&str]) -> Option<ProcOutput> {
    git(&ctx(), root, args).filter(|o| o.exit_code == 0)
}

/// `git status --porcelain`, raw, optionally limited to `paths` (after `--`).
pub(crate) fn porcelain(root: &Path, paths: &[&str]) -> Vec<u8> {
    let mut args = vec!["status", "--porcelain"];
    if !paths.is_empty() {
        args.push("--");
        args.extend_from_slice(paths);
    }
    git(&ctx(), root, &args)
        .map(|o| o.stdout)
        .unwrap_or_default()
}
