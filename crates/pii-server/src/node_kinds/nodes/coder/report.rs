//! What one coder run journals on `coder.exited` beyond the pid and exit code: the criteria that ran, the
//! failing output, and the routes the agent reported (observability, spec 185).

use super::super::agent_claude::run_facts;
use super::super::agent_quota::quota_signal;
use super::super::agent_route::{failovers, route_drift};
use super::super::agent_worker::CriterionRun;
use super::tree::TreeDigest;
use pii_observe::{Event, Level, names};

/// What one agent run produced, as the journal records it: the criteria, the failing output, the routes the
/// agent reported. Grouped so `emit_exited` keeps its argument count.
pub struct RunReport<'a> {
    /// The workdir digest before and after the agent ran; `None` when none was taken.
    pub tree: Option<(&'a TreeDigest, &'a TreeDigest)>,
    /// The `(provider, model)` the graph asked for; `None` when none was requested, which cannot drift.
    pub requested: Option<&'a (String, String)>,
    /// The criteria that ran, in order.
    pub runs: &'a [CriterionRun],
    /// Whether every criterion passed before the agent ran; `None` when no pre-run was taken (spec 239).
    pub pre_passed: Option<bool>,
    /// The tail of the first failing criterion's output.
    pub failure_tail: &'a str,
    /// The `(provider, model)` routes the agent reported.
    pub routes: &'a [(String, String)],
    /// The agent's captured stdout: its size and head go to the journal, so a run that reported no route shows
    /// what the agent actually printed.
    pub agent_output: &'a str,
    /// The agent's captured stderr: the reason it exited, when it said one. Size and head go to the journal.
    pub agent_stderr: &'a str,
}

impl RunReport<'_> {
    /// `"yes"`/`"no"` when a digest pair exists, `"unknown"` when none was taken: unknown is not "no".
    pub fn tree_changed(&self) -> String {
        match self.tree {
            Some((b, a)) if b.hash == a.hash => "no",
            Some(_) => "yes",
            None => "unknown",
        }
        .to_owned()
    }

    /// The `coder.exited` event for this run. The field names are the journal's contract; do not rename them.
    pub fn exited_event(&self, pid: u32, rc: i32, dur_ms: i64, stop: &str) -> Event {
        let base = Event::new("coder", Level::Info, names::CODER_EXITED)
            .field("pid", i64::from(pid))
            .field("rc", i64::from(rc))
            .field("dur_ms", dur_ms)
            .field("stop_reason", stop.to_owned());
        self.quota_fields(self.route_fields(
            self.claude_fields(self.tree_fields(self.output_fields(self.criteria_fields(base)))),
        ))
    }

    /// Whether the agent's output or stderr carries the CLI's spend-limit words, and the reset time it printed (spec
    /// 237). Observability only: no verdict reads it.
    fn quota_fields(&self, event: Event) -> Event {
        let signal = quota_signal(self.agent_output, self.agent_stderr);
        event
            .field(
                "quota_exhausted",
                if signal.is_some() { "yes" } else { "no" },
            )
            .field(
                "quota_reset",
                signal.map(|s| s.reset_text).unwrap_or_default(),
            )
    }

    /// The criteria that ran, and the first failure with its output, and whether they passed before the agent ran.
    fn criteria_fields(&self, event: Event) -> Event {
        let runs = self.runs;
        let passed = runs.iter().filter(|r| r.rc == 0).count();
        let failed = runs.iter().find(|r| r.rc != 0);
        let pre = match self.pre_passed {
            Some(true) => "yes",
            Some(false) => "no",
            None => "skipped",
        };
        event
            .field("pre_criteria_passed", pre)
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
    }

    /// What the agent printed, and the reason it exited when it wrote one to stderr: the failure a silent stdout hid.
    fn output_fields(&self, event: Event) -> Event {
        event
            .field("agent_output_bytes", self.agent_output.len().to_string())
            .field(
                "agent_output_head",
                self.agent_output.chars().take(200).collect::<String>(),
            )
            .field("agent_stderr_bytes", self.agent_stderr.len().to_string())
            .field(
                "agent_stderr_head",
                self.agent_stderr.chars().take(300).collect::<String>(),
            )
    }

    /// Whether the agent changed the workdir, with the digests it is judged from.
    fn tree_fields(&self, event: Event) -> Event {
        let count = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
        event
            .field("tree_changed", self.tree_changed())
            .field(
                "tree_files_before",
                self.tree.map_or(-1, |(b, _)| count(b.files)),
            )
            .field(
                "tree_files_after",
                self.tree.map_or(-1, |(_, a)| count(a.files)),
            )
            .field(
                "tree_hash_before",
                self.tree.map(|(b, _)| b.hash.clone()).unwrap_or_default(),
            )
            .field(
                "tree_hash_after",
                self.tree.map(|(_, a)| a.hash.clone()).unwrap_or_default(),
            )
    }

    /// What a claude run said about itself. Empty or -1 for any agent that does not print claude's stream-json:
    /// unknown, not zero.
    fn claude_fields(&self, event: Event) -> Event {
        let claude = run_facts(self.agent_output);
        event
            .field("claude_session_id", claude.session_id.unwrap_or_default())
            .field(
                "claude_turns",
                claude
                    .turns
                    .map_or(-1, |t| i64::try_from(t).unwrap_or(i64::MAX)),
            )
            .field(
                "claude_cost_usd",
                claude
                    .cost_usd
                    .map_or_else(String::new, |c| format!("{c:.6}")),
            )
            .field(
                "claude_is_error",
                claude
                    .is_error
                    .map_or("unknown", |e| if e { "true" } else { "false" })
                    .to_owned(),
            )
            // The output was redacted before this report was built (graph_node.rs), so the text carries no secret.
            .field(
                "claude_result",
                claude
                    .result_text
                    .map(|t| t.chars().take(300).collect::<String>())
                    .unwrap_or_default(),
            )
            .field(
                "claude_api_error_status",
                claude
                    .api_error_status
                    .map_or_else(String::new, |s| s.to_string()),
            )
    }

    /// The routes the agent reported beside the one the graph asked for. An empty `reported_routes` is unknown, not a
    /// match; `route_drift` and `failovers` are empty when none was seen. A swap mid-turn spends another provider's
    /// quota, so it must not read as a pass.
    fn route_fields(&self, event: Event) -> Event {
        let drift = self
            .requested
            .map_or_else(Vec::new, |r| route_drift(r, self.routes));
        event
            .field("route_drift", drift.join(","))
            .field(
                "failovers",
                failovers(self.agent_output)
                    .iter()
                    .map(|f| format!("{}>{}:{}", f.from, f.to, f.class))
                    .collect::<Vec<_>>()
                    .join(","),
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
