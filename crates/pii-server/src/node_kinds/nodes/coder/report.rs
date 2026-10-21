//! What one coder run journals on `coder.exited` beyond the pid and exit code: the criteria that ran, the
//! failing output, and the routes the agent reported (observability, spec 185).

use super::super::agent_worker::CriterionRun;

/// What one agent run produced, as the journal records it: the criteria, the failing output, the routes the
/// agent reported. Grouped so `emit_exited` keeps its argument count.
pub(in crate::node_kinds::nodes) struct RunReport<'a> {
    pub runs: &'a [CriterionRun],
    pub failure_tail: &'a str,
    pub routes: &'a [(String, String)],
}
