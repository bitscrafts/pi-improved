//! Promotion and lifecycle (spec 125): the only place in `pii` where a learned
//! conclusion changes behaviour by itself, so every rule here exists to make
//! that safe — a one-entry whitelist, a re-read `confirmed` verdict, a ledger
//! record before anything is applied, stored rollback data, a TTL, a declared
//! metric with an automatic reversal, and a scope check on every write.
//!
//! The loop's only action today is [`PromotionKind::QuarantineGraph`]: the
//! daemon refuses to start one named graph. Adding a second kind needs its own
//! spec. Everything lives under `<dir>/learn/` (promotions, ledger, counters).

mod storage;
mod types;

use std::path::PathBuf;
use std::sync::Arc;

use pii_observe::{Event, Level, Observer, names};

use crate::scan::RunSummary;
use crate::store::{self, StoreError};
use crate::verdict::{Verdict, get_verdict};
use storage::{append_ledger, read_status, save, status_path};

pub use storage::{
    get_promotion, ledger_tail, list_promotions, measure, promotion_counts, quarantine_reason,
    write_scoped,
};
pub use types::{
    DEFAULT_TTL_SECS, LedgerEntry, Metric, Outcome, PromoteConfig, PromoteRequest, Promotion,
    PromotionKind, PromotionState, Refusal, Refused, Rollback, ledger_id,
};

const SUBSYSTEM: &str = "learn";

/// Applies, reverts, expires and evaluates promotions under one store dir.
pub struct Promoter {
    dir: PathBuf,
    cfg: PromoteConfig,
    obs: Arc<dyn Observer>,
}

impl Promoter {
    /// A promoter over the store directory `dir` (the one holding `learn/`).
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>, cfg: PromoteConfig, obs: Arc<dyn Observer>) -> Self {
        Self {
            dir: dir.into(),
            cfg,
            obs,
        }
    }

    fn refuse(&self, req: &PromoteRequest, reason: Refusal, message: String, now: u64) -> Refused {
        let cand = store::show(&self.dir, &req.candidate).ok();
        let verdict = get_verdict(&self.dir, &req.candidate).map(|v| v.verdict);
        let graph = PromotionKind::parse(&req.kind)
            .ok()
            .and_then(|k| cand.as_ref().and_then(|c| k.target(c)))
            .unwrap_or_default();
        let ledger = append_ledger(
            &self.dir,
            LedgerEntry {
                seq: 0,
                at_secs: now,
                event: "refused".into(),
                promotion: String::new(),
                kind: req.kind.clone(),
                graph: graph.clone(),
                candidate: req.candidate.clone(),
                verdict,
                evidence: cand.map(|c| c.evidence),
                actor: req.actor.clone(),
                from: None,
                to: None,
                metric: req.metric.clone(),
                before: None,
                after: None,
                decision: message.clone(),
            },
        )
        .ok()
        .map(ledger_id);
        let mut st = read_status(&self.dir);
        *st.refused_by_reason
            .entry(reason.as_str().to_owned())
            .or_default() += 1;
        if let Ok(b) = serde_json::to_vec_pretty(&st) {
            let _ = write_scoped(&self.dir, &status_path(&self.dir), &b);
        }
        self.obs.event(
            &Event::new(SUBSYSTEM, Level::Warn, names::LEARN_PROMOTION_REFUSED)
                .field("candidate", req.candidate.as_str())
                .field("kind", req.kind.as_str())
                .field("graph", graph.as_str())
                .field("reason", reason.as_str())
                .field("actor", req.actor.as_str()),
        );
        Refused {
            reason,
            message,
            ledger,
        }
    }

    /// Promotes a candidate: checks every rule, records the decision in the
    /// ledger, stores the rollback data, and only then makes it effective.
    ///
    /// # Errors
    /// [`Refused`] with a counted reason; nothing changes except the counter
    /// and the refusal's ledger record.
    // reason: the rules are a flat sequence of guards; splitting would only hide the order.
    #[allow(clippy::too_many_lines)]
    pub fn promote(&self, req: &PromoteRequest, now: u64) -> Result<Promotion, Refused> {
        let no = |r, m: String| Err(self.refuse(req, r, m, now));
        let kind = match PromotionKind::parse(&req.kind) {
            Ok(k) => k,
            Err(m) => return no(Refusal::UnknownKind, m),
        };
        let Ok(cand) = store::show(&self.dir, &req.candidate) else {
            return no(
                Refusal::NoCandidate,
                format!("no candidate {}", req.candidate),
            );
        };
        // R2: the verdict is re-read from the learn area, never taken from the caller.
        let verdict = match get_verdict(&self.dir, &cand.id) {
            None => {
                return no(
                    Refusal::NoVerdict,
                    format!(
                        "candidate {} has no verdict (run pii learn verify)",
                        cand.id
                    ),
                );
            }
            Some(v) => match v.verdict {
                Verdict::Confirmed => v.verdict,
                Verdict::Refuted => {
                    return no(
                        Refusal::Refuted,
                        format!("candidate {} was refuted", cand.id),
                    );
                }
                Verdict::Inconclusive => {
                    return no(
                        Refusal::Inconclusive,
                        format!("candidate {} is inconclusive", cand.id),
                    );
                }
            },
        };
        // R5: an effect that cannot be measured is a guess wearing a promotion's clothes.
        let Some(metric) = req.metric.as_deref().and_then(Metric::parse) else {
            return no(
                Refusal::Unmeasurable,
                "no known metric declared (use --metric graph_failed_runs)".into(),
            );
        };
        let window = req.window_secs.unwrap_or(0);
        if window == 0 {
            return no(
                Refusal::Unmeasurable,
                "no measurement window declared (use --window-secs N, N > 0)".into(),
            );
        }
        let ttl = req.ttl_secs.unwrap_or(self.cfg.ttl_secs);
        if ttl == 0 {
            return no(Refusal::BadTtl, "ttl must be greater than zero".into());
        }
        let Some(graph) = kind.target(&cand) else {
            return no(
                Refusal::NoGraph,
                format!("candidate {} names no graph to {}", cand.id, kind.as_str()),
            );
        };
        if let Some(active) = list_promotions(&self.dir)
            .into_iter()
            .find(|p| p.in_effect(now) && p.kind == kind && p.graph == graph)
        {
            return no(
                Refusal::AlreadyActive,
                format!("{} is already in effect on graph {graph}", active.id),
            );
        }
        let mut h = blake3::Hasher::new();
        for part in [cand.id.as_str(), kind.as_str(), graph.as_str()] {
            h.update(part.as_bytes());
            h.update(&[0]);
        }
        h.update(&now.to_le_bytes());
        let hex = h.finalize().to_hex();
        let id = format!("p-{}", hex.as_str().get(..16).unwrap_or(hex.as_str()));
        let p = Promotion {
            id,
            kind,
            graph: graph.clone(),
            candidate: cand.id.clone(),
            verdict,
            evidence: cand.evidence.clone(),
            actor: req.actor.clone(),
            state: PromotionState::Active,
            rollback: Some(kind.rollback(&graph, now)),
            applied_secs: now,
            ttl_secs: ttl,
            expires_secs: now.saturating_add(ttl),
            metric,
            window_secs: window,
            outcome: None,
            ended_secs: None,
            decision: format!(
                "applied: candidate {} confirmed; {} on {graph} for {ttl}s, measuring {} over {window}s",
                cand.id,
                kind.as_str(),
                metric.as_str()
            ),
        };
        if p.rollback.is_none() {
            return no(Refusal::NoRollback, "rollback data missing".into());
        }
        // R3: the ledger record exists BEFORE the promotion is effective.
        let seq = match append_ledger(&self.dir, Self::entry(&p, "applied", None, now)) {
            Ok(s) => s,
            Err(e) => return no(scope_or_store(&e), e.to_string()),
        };
        if let Err(e) = save(&self.dir, &p) {
            return no(scope_or_store(&e), e.to_string());
        }
        self.obs.event(
            &Event::new(SUBSYSTEM, Level::Info, names::LEARN_PROMOTION_APPLIED)
                .field("promotion", p.id.as_str())
                .field("kind", kind.as_str())
                .field("graph", graph.as_str())
                .field("metric", metric.as_str())
                .field("window_secs", window)
                .field("ttl_secs", ttl)
                .field("ledger", ledger_id(seq)),
        );
        Ok(p)
    }

    fn entry(p: &Promotion, event: &str, from: Option<PromotionState>, now: u64) -> LedgerEntry {
        LedgerEntry {
            seq: 0,
            at_secs: now,
            event: event.to_owned(),
            promotion: p.id.clone(),
            kind: p.kind.as_str().to_owned(),
            graph: p.graph.clone(),
            candidate: p.candidate.clone(),
            verdict: Some(p.verdict),
            evidence: Some(p.evidence.clone()),
            actor: p.actor.clone(),
            from,
            to: Some(p.state),
            metric: Some(p.metric.as_str().to_owned()),
            before: p.outcome.as_ref().map(|o| o.before),
            after: p.outcome.as_ref().map(|o| o.after),
            decision: p.decision.clone(),
        }
    }

    /// Moves `p` out of `active` into `to`: ledger first, then the record,
    /// then the event. Returns the ledger id.
    fn end(
        &self,
        p: &mut Promotion,
        to: PromotionState,
        decision: String,
        actor: Option<&str>,
        now: u64,
    ) -> Result<String, String> {
        let from = p.state;
        p.state = to;
        p.ended_secs = Some(now);
        p.decision = decision;
        if let Some(a) = actor {
            a.clone_into(&mut p.actor);
        }
        let (event, name) = match to {
            PromotionState::Expired => ("expired", names::LEARN_PROMOTION_EXPIRED),
            PromotionState::Reverted => ("reverted", names::LEARN_PROMOTION_REVERTED),
            PromotionState::AutoReverted => ("auto_reverted", names::LEARN_PROMOTION_REVERTED),
            PromotionState::Active => return Err("not an ending state".into()),
        };
        let seq = append_ledger(&self.dir, Self::entry(p, event, Some(from), now))
            .map_err(|e| e.to_string())?;
        save(&self.dir, p).map_err(|e| e.to_string())?;
        let mut ev = Event::new(SUBSYSTEM, Level::Info, name)
            .field("promotion", p.id.as_str())
            .field("kind", p.kind.as_str())
            .field("graph", p.graph.as_str())
            .field("metric", p.metric.as_str())
            .field("state", to.as_str())
            .field("ledger", ledger_id(seq));
        if let Some(o) = &p.outcome {
            ev = ev.field("before", o.before).field("after", o.after);
        }
        self.obs.event(&ev);
        Ok(ledger_id(seq))
    }

    /// Takes a promotion back by hand (R4): restores the stored previous state.
    ///
    /// # Errors
    /// A message when the id is unknown, the promotion is no longer active, its
    /// rollback data is missing, or the ledger cannot be written.
    pub fn revert(&self, id: &str, actor: &str, now: u64) -> Result<String, String> {
        let mut p = get_promotion(&self.dir, id).ok_or_else(|| format!("no promotion {id}"))?;
        if p.state != PromotionState::Active {
            return Err(format!("promotion {id} is already {}", p.state.as_str()));
        }
        let Some(rb) = p.rollback.clone() else {
            return Err(format!("promotion {id} has no rollback data"));
        };
        let decision = match rb.kind {
            PromotionKind::QuarantineGraph => format!(
                "reverted by {actor}: graph {} restored to quarantined={}",
                rb.graph, rb.was_quarantined
            ),
        };
        self.end(&mut p, PromotionState::Reverted, decision, Some(actor), now)
    }

    /// One lifecycle pass: lapses promotions past their TTL and evaluates those
    /// whose window has closed, reverting any that did not improve the metric.
    /// `load_runs` is called at most once, and only when an evaluation is due.
    /// Returns `(promotion id, new state)` for each one that changed.
    pub fn sweep(
        &self,
        now: u64,
        load_runs: impl FnOnce() -> Vec<RunSummary>,
    ) -> Vec<(String, PromotionState)> {
        let mut changed = Vec::new();
        let mut runs: Option<Vec<RunSummary>> = None;
        let mut load = Some(load_runs);
        for mut p in list_promotions(&self.dir) {
            if p.state != PromotionState::Active {
                continue;
            }
            let due = now >= p.applied_secs.saturating_add(p.window_secs);
            if p.outcome.is_none() && due {
                if runs.is_none() {
                    runs = load.take().map(|f| f());
                }
                let o = measure(&p, runs.as_deref().unwrap_or(&[]));
                let improved = o.improved;
                p.outcome = Some(o.clone());
                let verdict = format!(
                    "{} before={} after={}",
                    p.metric.as_str(),
                    o.before,
                    o.after
                );
                if improved {
                    p.decision = format!("kept: {verdict}");
                    if append_ledger(
                        &self.dir,
                        Self::entry(&p, "outcome_kept", Some(p.state), now),
                    )
                    .is_ok()
                    {
                        let _ = save(&self.dir, &p);
                    }
                } else if self
                    .end(
                        &mut p,
                        PromotionState::AutoReverted,
                        format!("reverted automatically: metric did not improve ({verdict})"),
                        Some("daemon"),
                        now,
                    )
                    .is_ok()
                {
                    changed.push((p.id.clone(), PromotionState::AutoReverted));
                    continue;
                }
            }
            let ttl = p.ttl_secs;
            if now >= p.expires_secs
                && p.state == PromotionState::Active
                && self
                    .end(
                        &mut p,
                        PromotionState::Expired,
                        format!("expired: ttl {ttl}s elapsed"),
                        Some("daemon"),
                        now,
                    )
                    .is_ok()
            {
                changed.push((p.id.clone(), PromotionState::Expired));
            }
        }
        changed
    }
}

fn scope_or_store(e: &StoreError) -> Refusal {
    if e.0.contains("outside its own store area") || e.0.contains("symlink") {
        Refusal::Scope
    } else {
        Refusal::Store
    }
}
