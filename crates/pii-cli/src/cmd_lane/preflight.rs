//! The judge preflight's inputs (spec 319 R2, `S:234-255`): the throwaway repo's name and base, and the judge input
//! the daemon reads. The repo itself is made in `git.rs` and `mod.rs`; this file is the pure part.

/// The throwaway repo the judge preflight runs against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreflightRepo {
    /// The `preflight-<ts>-<rand>` directory name under the devbox-test dir.
    pub(crate) dir: String,
    /// `git rev-parse HEAD` of that repo.
    pub(crate) rev: String,
    /// The `date +%s` the directory was made with.
    pub(crate) ts: u64,
}

/// The one-line judge input: `{"spec":…,"base":…,"workdir":…}` (the shell's `printf`).
pub(crate) fn judge_input(rev: &str, workdir: &str) -> String {
    format!("{{\"spec\":\"specs/preflight.md\",\"base\":\"{rev}\",\"workdir\":\"{workdir}\"}}\n")
}

/// The preflight spec the throwaway repo commits: one criterion that makes a directory under `_tmp/`.
pub(crate) fn spec_text(prel: &str) -> String {
    format!("# Spec: preflight\n\n## Exit Criteria\n- [ ] `mkdir -p {prel} && test -d {prel}`\n")
}
