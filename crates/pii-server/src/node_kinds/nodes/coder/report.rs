//! What one coder run journals on `coder.exited` beyond the pid and exit code: the criteria that ran, the
//! failing output, and the routes the agent reported (observability, spec 185).

use super::super::agent_worker::CriterionRun;
use super::tree::TreeDigest;
use pii_observe::{Event, Level, names};

/// What one agent run produced, as the journal records it: the criteria, the failing output, the routes the
/// agent reported. Grouped so `emit_exited` keeps its argument count.
pub(in crate::node_kinds::nodes) struct RunReport<'a> {
    /// The workdir digest before and after the agent ran; `None` when none was taken.
    pub tree: Option<(&'a TreeDigest, &'a TreeDigest)>,
    pub runs: &'a [CriterionRun],
    pub failure_tail: &'a str,
    pub routes: &'a [(String, String)],
    /// The agent's captured stdout: its size and head go to the journal, so a run that reported no route shows
    /// what the agent actually printed.
    pub agent_output: &'a str,
    /// The agent's captured stderr: the reason it exited, when it said one. Size and head go to the journal.
    pub agent_stderr: &'a str,
}

impl RunReport<'_> {
    /// `"yes"`/`"no"` when a digest pair exists, `"unknown"` when none was taken: unknown is not "no".
    fn tree_changed(&self) -> String {
        match self.tree {
            Some((b, a)) if b.hash == a.hash => "no",
            Some(_) => "yes",
            None => "unknown",
        }
        .to_owned()
    }

    /// The `coder.exited` event for this run. The field names are the journal's contract; do not rename them.
    pub(in crate::node_kinds::nodes) fn exited_event(
        &self,
        pid: u32,
        rc: i32,
        dur_ms: i64,
        stop: &str,
    ) -> Event {
        let runs = self.runs;
        let passed = runs.iter().filter(|r| r.rc == 0).count();
        let failed = runs.iter().find(|r| r.rc != 0);
        Event::new("coder", Level::Info, names::CODER_EXITED)
            .field("pid", i64::from(pid))
            .field("rc", i64::from(rc))
            .field("dur_ms", dur_ms)
            .field("stop_reason", stop.to_owned())
            .field(
                "criteria_run",
                i64::try_from(runs.len()).unwrap_or(i64::MAX),
            )
            .field("criteria_passed", i64::try_from(passed).unwrap_or(i64::MAX))
            .field(
                "criteria_failed",
                i64::try_from(runs.len() - passed).unwrap_or(i64::MAX),
            )
            .field(
                "first_failure",
                failed.map(|r| r.command.clone()).unwrap_or_default(),
            )
            .field("failure_output", self.failure_tail.to_owned())
            // What the agent REPORTED it used (provider/model), beside the requested route in the graph. An
            // empty value means the run reported no route, which is unknown, not a match.
            .field("agent_output_bytes", self.agent_output.len().to_string())
            .field(
                "agent_output_head",
                self.agent_output.chars().take(200).collect::<String>(),
            )
            // The reason the agent exited, when it wrote one to stderr: the failure a silent stdout hid.
            .field("agent_stderr_bytes", self.agent_stderr.len().to_string())
            .field(
                "agent_stderr_head",
                self.agent_stderr.chars().take(300).collect::<String>(),
            )
            .field("tree_changed", self.tree_changed())
            .field(
                "tree_files_before",
                self.tree
                    .map_or(-1, |(b, _)| i64::try_from(b.files).unwrap_or(i64::MAX)),
            )
            .field(
                "tree_files_after",
                self.tree
                    .map_or(-1, |(_, a)| i64::try_from(a.files).unwrap_or(i64::MAX)),
            )
            .field(
                "tree_hash_before",
                self.tree.map(|(b, _)| b.hash.clone()).unwrap_or_default(),
            )
            .field(
                "tree_hash_after",
                self.tree.map(|(_, a)| a.hash.clone()).unwrap_or_default(),
            )
            .field(
                "reported_routes",
                self.routes
                    .iter()
                    .map(|(p, m)| format!("{p}/{m}"))
                    .collect::<Vec<_>>()
                    .join(","),
            )
    }
}
