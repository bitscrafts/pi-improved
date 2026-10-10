//! The four deterministic, offline extractors (spec 123 R2). Each claim is
//! built from numbers and closed-vocabulary tokens that are also in the
//! candidate's evidence, so a reader with only the evidence can check it.

use std::collections::BTreeMap;

use crate::candidate::{Candidate, Evidence, Kind, Refusal};
use crate::config::LearnConfig;
use crate::scan::RunSummary;

/// Candidates plus the refusals that were counted instead of emitted.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Extraction {
    /// Emitted candidates, sorted by id.
    pub candidates: Vec<Candidate>,
    /// Refused candidates by reason.
    pub refused: BTreeMap<Refusal, u64>,
}

impl Extraction {
    fn push(&mut self, c: Result<Candidate, Refusal>) {
        match c {
            Ok(c) => self.candidates.push(c),
            Err(r) => *self.refused.entry(r).or_default() += 1,
        }
    }
}

/// Runs every extractor over `runs`. Output is sorted and deduplicated by id:
/// the same input gives byte-identical output (R5).
#[must_use]
pub fn extract_all(runs: &[RunSummary], cfg: &LearnConfig) -> Extraction {
    let mut sorted: Vec<&RunSummary> = runs.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.run.cmp(&b.run)));
    let mut out = Extraction::default();
    repeated_failure(&sorted, cfg, &mut out);
    stuck_loop(&sorted, cfg, &mut out);
    limit_hit(&sorted, cfg, &mut out);
    cost_outlier(&sorted, cfg, &mut out);
    out.candidates.sort_by(|a, b| a.id.cmp(&b.id));
    out.candidates.dedup_by(|a, b| a.id == b.id);
    out
}

/// How a claim names where a failure happened (`-` is the run terminal).
fn place(node: &str) -> String {
    if node == "-" {
        String::from("the run terminal")
    } else {
        format!("node {node}")
    }
}

fn evidence(runs: &[&RunSummary], counts: &[(&str, u64)]) -> Evidence {
    Evidence {
        runs: runs.iter().map(|r| r.run.clone()).collect(),
        paths: runs.iter().map(|r| r.path.clone()).collect(),
        counts: counts.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
    }
}

/// (a) The same `(graph, node, error class)` failed in at least `min_runs`
/// distinct runs. Three is the smallest count that is not a coincidence pair.
fn repeated_failure(runs: &[&RunSummary], cfg: &LearnConfig, out: &mut Extraction) {
    let mut groups: BTreeMap<(&str, &str, &str), (Vec<&RunSummary>, u64)> = BTreeMap::new();
    for r in runs {
        for (node, class) in &r.failures {
            let g = groups.entry((&r.graph, node, class)).or_default();
            if g.0.last().is_none_or(|l| l.run != r.run) {
                g.0.push(r);
            }
            g.1 += 1;
        }
    }
    for ((graph, node, class), (rs, failures)) in groups {
        if (rs.len() as u64) < u64::from(cfg.min_runs) {
            continue;
        }
        let n = rs.len() as u64;
        let claim = format!(
            "{} in graph {graph} failed with class {class} in {n} runs ({failures} failures, threshold {})",
            place(node),
            cfg.min_runs
        );
        let ev = evidence(
            &rs,
            &[
                ("runs", n),
                ("failures", failures),
                ("threshold", u64::from(cfg.min_runs)),
            ],
        );
        out.push(Candidate::new(
            Kind::RepeatedFailure,
            claim,
            format!("repeated_failure:{graph}:{node}:{class}"),
            ev,
        ));
    }
}

/// (b) A graph with at least `min_runs` runs, every one of which failed at the
/// same `(node, class)`: not a flake, a loop that cannot get out.
fn stuck_loop(runs: &[&RunSummary], cfg: &LearnConfig, out: &mut Extraction) {
    let mut by_graph: BTreeMap<&str, Vec<&RunSummary>> = BTreeMap::new();
    for r in runs {
        by_graph.entry(&r.graph).or_default().push(r);
    }
    for (graph, rs) in by_graph {
        let sig = |r: &RunSummary| r.failures.last().cloned();
        let first = rs.first().and_then(|r| sig(r));
        let same = rs.iter().all(|r| r.verdict == "failure" && sig(r) == first);
        let (true, Some((node, class))) = (same, first) else {
            continue;
        };
        if (rs.len() as u64) < u64::from(cfg.min_runs) {
            continue;
        }
        let n = rs.len() as u64;
        let claim = format!(
            "graph {graph} failed at {} with class {class} in all {n} runs and never succeeded (threshold {})",
            place(&node),
            cfg.min_runs
        );
        let ev = evidence(
            &rs,
            &[
                ("runs", n),
                ("failed_runs", n),
                ("threshold", u64::from(cfg.min_runs)),
            ],
        );
        out.push(Candidate::new(
            Kind::StuckLoop,
            claim,
            format!("stuck_loop:{graph}:{node}:{class}"),
            ev,
        ));
    }
}

/// (c) A registered limit/refusal event (memory cap, epoch timeout, fuel,
/// refusal, truncation) seen in at least `min_runs` distinct runs.
fn limit_hit(runs: &[&RunSummary], cfg: &LearnConfig, out: &mut Extraction) {
    let mut groups: BTreeMap<&str, (Vec<&RunSummary>, u64)> = BTreeMap::new();
    for r in runs {
        for (key, n) in &r.limits {
            let g = groups.entry(key).or_default();
            g.0.push(r);
            g.1 += n;
        }
    }
    for (key, (rs, hits)) in groups {
        if (rs.len() as u64) < u64::from(cfg.min_runs) {
            continue;
        }
        let n = rs.len() as u64;
        let claim = format!(
            "limit {key} was hit {hits} times across {n} runs (threshold {})",
            cfg.min_runs
        );
        let ev = evidence(
            &rs,
            &[
                ("runs", n),
                ("hits", hits),
                ("threshold", u64::from(cfg.min_runs)),
            ],
        );
        out.push(Candidate::new(
            Kind::LimitHit,
            claim,
            format!("limit_hit:{key}"),
            ev,
        ));
    }
}

type Metric = (&'static str, fn(&RunSummary) -> u64, u64);

/// Lower median of an already-sorted slice.
fn median(sorted: &[u64]) -> u64 {
    sorted
        .get(sorted.len().saturating_sub(1) / 2)
        .copied()
        .unwrap_or(0)
}

/// (d) Within a graph of at least `outlier_min_runs` runs, a run whose summed
/// node duration (or file bytes) is at least `outlier_factor` times the lower
/// median AND above an absolute floor, so a ~0 median cannot flag everything.
fn cost_outlier(runs: &[&RunSummary], cfg: &LearnConfig, out: &mut Extraction) {
    let mut by_graph: BTreeMap<&str, Vec<&RunSummary>> = BTreeMap::new();
    for r in runs {
        by_graph.entry(&r.graph).or_default().push(r);
    }
    let metrics: [Metric; 2] = [
        ("dur_ms", |r| r.dur_ms, cfg.outlier_min_ms),
        ("bytes", |r| r.bytes, cfg.outlier_min_bytes),
    ];
    for (graph, rs) in by_graph {
        if (rs.len() as u64) < u64::from(cfg.outlier_min_runs) {
            continue;
        }
        for (name, get, floor) in metrics {
            let mut vals: Vec<u64> = rs.iter().map(|r| get(r)).collect();
            vals.sort_unstable();
            let med = median(&vals);
            for r in &rs {
                let v = get(r);
                if v < floor || v < med.saturating_mul(u64::from(cfg.outlier_factor)) {
                    continue;
                }
                let mut cited: Vec<&RunSummary> = vec![*r];
                cited.extend(rs.iter().filter(|o| o.run != r.run));
                let claim = format!(
                    "run {} in graph {graph} had {name} {v}, at least {}x the median {med} of {} runs",
                    r.run,
                    cfg.outlier_factor,
                    rs.len()
                );
                let ev = evidence(
                    &cited,
                    &[
                        ("value", v),
                        ("median", med),
                        ("group_runs", rs.len() as u64),
                        ("factor", u64::from(cfg.outlier_factor)),
                    ],
                );
                out.push(Candidate::new(
                    Kind::CostOutlier,
                    claim,
                    format!("cost_outlier:{graph}:{name}:{}", r.run),
                    ev,
                ));
            }
        }
    }
}
