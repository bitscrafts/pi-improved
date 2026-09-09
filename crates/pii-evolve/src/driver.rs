//! The boss's outer-loop skeleton (R5).
//!
//! Loads `state.md`, runs the deterministic self-improvement gates
//! (reachability-check and doc-drift-check, spec 22's `cargo xtask`
//! machinery), and on the first red gate records the failure and suspends
//! for a human — never self-approves. Patching and validation are a
//! later, graph-driven slice; this module only proves the dogfooding
//! skeleton (check -> record -> suspend) and always persists state, win or
//! lose.

use std::path::{Path, PathBuf};
use std::time::Duration;

use pii_core::{Budget, CapKind, CapSet, Ctx, PiiError, StringScope};

use crate::state::StateDoc;

/// Wall-clock budget for a single gate subprocess.
const GATE_TIMEOUT: Duration = Duration::from_mins(2);

/// One deterministic gate the boss runs before anything else.
#[derive(Debug, Clone)]
pub struct Gate {
    /// Human-readable gate name, recorded in `needs_attention` on failure.
    pub name: String,
    /// The argv run from [`GatePolicy::workspace_root`]; a non-zero exit
    /// means the gate is red.
    pub argv: Vec<String>,
}

/// The gates a cycle runs, in order, and the directory they run from.
#[derive(Debug, Clone)]
pub struct GatePolicy {
    /// The gates to run; the first red gate stops the cycle.
    pub gates: Vec<Gate>,
    /// The workspace root gates run from.
    pub workspace_root: PathBuf,
}

impl GatePolicy {
    /// The default policy: `reachability-check` then `doc-drift-check` via
    /// `cargo xtask`, spec 22's deterministic self-improvement gates.
    #[must_use]
    pub fn default_gates(workspace_root: PathBuf) -> Self {
        Self {
            gates: vec![
                Gate {
                    name: String::from("reachability-check"),
                    argv: vec![
                        String::from("cargo"),
                        String::from("xtask"),
                        String::from("reachability-check"),
                    ],
                },
                Gate {
                    name: String::from("doc-drift-check"),
                    argv: vec![
                        String::from("cargo"),
                        String::from("xtask"),
                        String::from("doc-drift-check"),
                    ],
                },
            ],
            workspace_root,
        }
    }
}

/// The result of one boss cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CycleOutcome {
    /// Every gate passed; the cycle completed cleanly.
    Ok,
    /// A gate was red; the boss suspended for human acknowledgement rather
    /// than self-approving. Carries the failing gate's name.
    PendingHuman(String),
    /// The cycle's budget for gate calls was consumed before/while running;
    /// the cycle stopped rather than spending unboundedly.
    BudgetExhausted,
}

/// Runs one boss cycle rooted at `state_dir`.
///
/// Loads `state.md`, runs every gate in `policy` in order, and on the
/// first red gate records `needs_attention` and suspends. Each gate call
/// is charged against `budget` (kind `ToolCalls`); if the budget is
/// exhausted the cycle returns [`CycleOutcome::BudgetExhausted`] instead of
/// spending further. `state.md` is always saved and the cycle counter always
/// bumps, whether the cycle went green, red, or stopped on budget.
///
/// # Errors
/// Returns [`PiiError`] on `state.md` I/O failure or a gate process
/// failure that is not itself a gate-red result (the gate binary could not
/// be spawned at all).
pub fn run_cycle(
    state_dir: &Path,
    policy: &GatePolicy,
    budget: Budget,
) -> Result<CycleOutcome, PiiError> {
    let mut state = StateDoc::load(state_dir)?;
    let ctx = gate_ctx(policy, budget);

    let cycle_count: u64 = state
        .properties
        .get("cycle_count")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    state
        .properties
        .insert(String::from("cycle_count"), (cycle_count + 1).to_string());

    for gate in &policy.gates {
        // Charge one gate call against the budget; a zero/exhausted budget
        // stops the cycle cleanly before it spends unboundedly.
        if ctx
            .budget()
            .charge(pii_core::BudgetKind::ToolCalls, 1)
            .is_err()
        {
            state.phase = String::from("budget_exhausted");
            state.last_step.clone_from(&gate.name);
            state.touch(&ctx);
            state.save(state_dir)?;
            return Ok(CycleOutcome::BudgetExhausted);
        }
        let output = ctx
            .proc()?
            .run(&gate.argv, &policy.workspace_root, GATE_TIMEOUT)?;
        if output.exit_code != 0 {
            state
                .properties
                .insert(String::from("needs_attention"), String::from("true"));
            state.properties.insert(
                String::from("failure_signature"),
                format!("{}:exit={}", gate.name, output.exit_code),
            );
            state.phase = String::from("pending_human");
            state.last_step.clone_from(&gate.name);
            state.touch(&ctx);
            state.save(state_dir)?;
            return Ok(CycleOutcome::PendingHuman(gate.name.clone()));
        }
    }

    state.properties.remove("needs_attention");
    state.properties.remove("failure_signature");
    state.phase = String::from("ok");
    state.touch(&ctx);
    state.save(state_dir)?;
    Ok(CycleOutcome::Ok)
}

/// Builds the confined `Ctx` gate subprocesses run under: `PROC` scoped to
/// exactly the argv0 binaries `policy`'s gates need, nothing wider, and the
/// given `budget` (so a cycle is bounded, not `unlimited`).
fn gate_ctx(policy: &GatePolicy, budget: Budget) -> Ctx {
    let argv0s: Vec<String> = policy
        .gates
        .iter()
        .filter_map(|g| g.argv.first().cloned())
        .collect();
    let caps = CapSet {
        kinds: CapKind::PROC,
        proc_argv0: StringScope::new(argv0s),
        ..CapSet::default()
    };
    Ctx::root(caps, budget)
}
