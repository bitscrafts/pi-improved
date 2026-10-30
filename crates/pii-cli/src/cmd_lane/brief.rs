//! The brief the agent is given, byte-identical to the script's heredoc (spec 317 R4).

const TEMPLATE: &str = "Implement spec {TAG} (the spec file is the CONTRACT; its path is named at the end of this prompt).
Follow it exactly: TDD (see the RED fail before the GREEN), minimal diffs.
Do NOT run the whole workspace gate: the loop runs quality-gate.sh after you and it is the arbiter. Before you finish run the cheap half:
    cargo fmt --all
    cargo clippy -p <each crate you touched> --all-targets -- -D warnings
and run only the tests your spec names (cargo test -p <crate> --test <name>). Keep the turn short.
HARD RULES: never create, edit or rewrite anything under specs/ (read-only for implementers). Do NOT commit and do not run git commit/push.
Destructive commands are banned: never rm -rf, never git checkout/restore ., git reset --hard or a bare git stash in this shared tree,
and never touch /dev, /proc, /sys or /root. Scratch you create goes under {SCRATCH} and only that may be removed, with a guarded script.
Work in the foreground; no background processes. If you run out of budget, write a COMPLETE handoff (done/remaining/next-start) to
{LOGD}/HANDOFF-{TAG}.md and stop honestly; never compress the work to fake completion.
Final line: SPEC-DONE {N} criteria=<passed>/<total>, with every criterion command and its exit code listed.
";

/// The brief text. `scratch` is the lane's scratch phrase; `logd` is the agent-side `_tmp/claude-batchC`.
pub(crate) fn text(tag: &str, n: &str, logd: &str, scratch: &str) -> String {
    TEMPLATE
        .replace("{TAG}", tag)
        .replace("{N}", n)
        .replace("{LOGD}", logd)
        .replace("{SCRATCH}", scratch)
}

/// The scratch phrase the brief names: the spec lane's is inside the repository.
pub(crate) fn scratch(spec_lane: bool) -> &'static str {
    if spec_lane {
        "_tmp/ in the repository"
    } else {
        "/projects/_tmp/"
    }
}

/// The spec lane's documentation step.
pub(crate) fn docs(n: &str) -> String {
    format!(
        "Documentation step. Create the file _tmp/docs-notes/NOTES-{n}.md (make the directory first) with a short note on what the spec changed. Do not modify any other file, do not modify anything under specs/, and do not commit anything."
    )
}

/// The brief with every trailing newline removed (`$(cat …)` in the shell).
pub(crate) fn strip_trailing_newlines(brief: &[u8]) -> &[u8] {
    brief
        .iter()
        .rposition(|&b| b != b'\n')
        .and_then(|i| brief.get(..=i))
        .unwrap_or_default()
}
