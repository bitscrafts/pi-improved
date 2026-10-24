//! `pii graph edit <name> --file <path> [--force]`: lint, refuse armed runs, move the old file
//! aside, write atomically, re-lint the installed copy, and report the hashes.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pii_core::Clock as _;
use pii_graph::fragment::sha256_hex;
use pii_graph::{GraphSpec, NodeRegistry};

use super::explain::node_diff;
use super::{
    Opts, check, error, fragments_dir, installed_dir, is_hash, one_name, registry, repo_dir,
    store_path, suspended_runs, usage,
};
use crate::cli_util::fail;

/// How the installed copy compares with the repo's, naming the node ids that differ.
pub(super) fn drift_verdict(
    name: &str,
    repo: &Path,
    sha: &str,
    spec: &GraphSpec,
    reg: &NodeRegistry,
) -> String {
    let Ok(rep_src) = std::fs::read_to_string(repo.join(format!("{name}.toml"))) else {
        return String::from("installed-only (no copy in the repo)");
    };
    let rep_sha = sha256_hex(rep_src.as_bytes());
    if rep_sha == sha {
        return String::from("same (the repo copy has the same sha256)");
    }
    let rep = match check(
        &rep_src,
        &fragments_dir(repo, repo),
        &[repo.to_path_buf()],
        reg,
    ) {
        Ok(c) => c.spec,
        Err(e) => return format!("repo copy does not load: {e}"),
    };
    let (changed, only_i, only_r) = node_diff(spec, &rep);
    if changed.is_empty() && only_i.is_empty() && only_r.is_empty() {
        return format!("differs from the repo copy outside its nodes (repo sha256 {rep_sha})");
    }
    format!(
        "differs from the repo copy (repo sha256 {rep_sha}); changed nodes: [{}]; \
         only installed: [{}]; only repo: [{}]",
        changed.join(", "),
        only_i.join(", "),
        only_r.join(", ")
    )
}

/// `pii graph edit <name> --file <path> [--force]`: lint, refuse armed runs, move the old file
/// aside, write atomically, re-lint the installed copy, and report the hashes.
pub(super) fn run(o: &Opts) -> ExitCode {
    let name = match one_name(o) {
        Ok(n) => n,
        Err(code) => return code,
    };
    let Some(file) = o.file.clone() else {
        return usage();
    };
    let installed = match installed_dir(o) {
        Ok(d) => d,
        Err(code) => return code,
    };
    let repo = repo_dir(o);
    let target = installed.join(format!("{name}.toml"));
    let before = match std::fs::read(&target) {
        Ok(b) => b,
        Err(e) => return error(format!("{name} is not installed: {e}")),
    };
    let new = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(e) => return error(format!("read {}: {e}", file.display())),
    };
    let armed = match gate(o, &name, &file, &installed, &new) {
        Ok(armed) => armed,
        Err(code) => return code,
    };

    let before_sha = sha256_hex(&before);
    let after_sha = sha256_hex(new.as_bytes());
    if before_sha == after_sha {
        println!("unchanged {name}: sha256 {before_sha}");
        return ExitCode::SUCCESS;
    }

    // The previous file moves aside under an epoch name. An existing name is refused, never
    // overwritten.
    let epoch = pii_core::SystemClock.now_ms();
    let prev = installed.join(format!("{name}.toml.prev-{epoch}"));
    if prev.exists() {
        return error(format!("refused: {} already exists", prev.display()));
    }
    let tmp = installed.join(format!(".{name}.toml.tmp-{}", std::process::id()));
    if let Err(e) = install(&target, &prev, &tmp, new.as_bytes()) {
        return error(e);
    }
    if !armed.is_empty() {
        println!("forced past armed run(s): {}", armed.join(", "));
    }
    let reread_sha = match relint_installed(&target, &installed, &repo) {
        Ok(sha) => sha,
        Err(e) => {
            return error(format!(
                "installed copy fails lint after the edit: {e}; previous file kept at {}",
                prev.display()
            ));
        }
    };
    report_edit(&name, &before_sha, &reread_sha, &prev, &repo);
    ExitCode::SUCCESS
}

/// Steps (a) and (b) of an edit. The new file must lint as `pii graph lint` would lint it, and no
/// suspended run of this graph (nor one that cannot be named) may be running unless `--force`
/// is given. Returns the run ids that `--force` let through.
fn gate(
    o: &Opts,
    name: &str,
    file: &Path,
    installed: &Path,
    new: &str,
) -> Result<Vec<String>, ExitCode> {
    let lint_fragments = file
        .parent()
        .map_or_else(|| PathBuf::from("fragments"), |p| p.join("fragments"));
    if let Err(e) = check(
        new,
        &lint_fragments,
        &[file
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()],
        &registry(),
    ) {
        fail(
            "graph",
            format!("refused: {} fails lint: {e}", file.display()),
        );
        return Err(ExitCode::from(3));
    }
    let blocking: Vec<String> = match suspended_runs(&store_path(o, installed)) {
        Ok(runs) => runs
            .into_iter()
            .filter(|r| r.graph == name || is_hash(&r.graph))
            .map(|r| r.run)
            .collect(),
        Err(e) => {
            return Err(error(format!(
                "refused: cannot tell whether {name} has armed runs: {e}"
            )));
        }
    };
    if blocking.is_empty() || o.force {
        return Ok(blocking);
    }
    fail(
        "graph",
        format!(
            "refused: {name} has armed run(s) {}; pass --force to edit under them",
            blocking.join(", ")
        ),
    );
    Err(ExitCode::from(3))
}

/// Step (d): the new bytes go to `tmp` first, so a failure there leaves the installed copy alone.
/// Then the old file moves to `prev`, and `tmp` is renamed into `target`. A failed rename puts the
/// previous file back, so the installed name is never left empty.
fn install(target: &Path, prev: &Path, tmp: &Path, new: &[u8]) -> Result<(), String> {
    write_new(tmp, new).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(target, prev).map_err(|e| format!("move {} aside: {e}", target.display()))?;
    if let Err(e) = std::fs::rename(tmp, target) {
        let restored = std::fs::rename(prev, target).is_ok();
        return Err(format!(
            "install {}: {e}; previous restored: {restored}; new bytes are in {}",
            target.display(),
            tmp.display()
        ));
    }
    Ok(())
}

/// Re-reads the installed copy and lints it again. Returns its sha256 on success.
fn relint_installed(target: &Path, installed: &Path, repo: &Path) -> Result<String, String> {
    let src = std::fs::read_to_string(target).map_err(|e| format!("re-read: {e}"))?;
    check(
        &src,
        &fragments_dir(installed, repo),
        &[installed.to_path_buf()],
        &registry(),
    )?;
    Ok(sha256_hex(src.as_bytes()))
}

/// The edit's report: both hashes, the previous file, and the drift state against the repo.
fn report_edit(name: &str, before_sha: &str, after_sha: &str, prev: &Path, repo: &Path) {
    println!("edited {name}");
    println!("before-sha {before_sha}");
    println!("after-sha {after_sha}");
    println!("moved aside {}", prev.display());
    match std::fs::read(repo.join(format!("{name}.toml"))) {
        Ok(r) if sha256_hex(&r) == after_sha => println!("drift same (repo copy matches)"),
        Ok(_) => println!("drift DRIFT (repo copy differs)"),
        Err(_) => println!("drift installed-only (no copy in the repo)"),
    }
}

/// Creates `tmp` exclusively and writes `bytes` into it, flushed to disk.
fn write_new(tmp: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(tmp)?;
    f.write_all(bytes)?;
    f.sync_all()
}
