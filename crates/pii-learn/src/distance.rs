//! Spec 159: the operator distance, derived from recorded timestamps alone.
//!
//! Per incident in `incidents.jsonl`: **`H_pre`** (an operator act earlier than
//! the card), **L** (first evidence → opened) and **A** (operator acts). The
//! card is `card_at` when recorded, else the incident's `opened_at`. Operator
//! acts are the optional `operator_acts` array; an absent array is zero acts.
//! A missing or unparseable timestamp is named and counted, never imputed.
//! Read-only: nothing here writes the store.

use std::fmt::Write as _;
use std::path::Path;

use crate::incident::{INCIDENTS_FILE, parse_iso};

/// One incident's three numbers; `None` is NOT COMPUTABLE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Distance {
    /// The incident's fingerprint (or `?` when absent).
    pub fingerprint: String,
    /// `first_seen` as recorded, for naming it.
    pub first_seen: String,
    /// An operator act precedes the card.
    pub h_pre: Option<bool>,
    /// Seconds from first evidence to the incident being opened.
    pub l_secs: Option<i64>,
    /// Operator acts.
    pub acts: Option<u64>,
}

/// The measured window.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DistanceReport {
    /// Every incident in the window, in file order.
    pub incidents: Vec<Distance>,
}

fn ts(v: &serde_json::Value, key: &str) -> Option<i64> {
    parse_iso(v.get(key)?.as_str()?)
}

fn measure_one(v: &serde_json::Value) -> Distance {
    let text = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_owned();
    let opened = ts(v, "opened_at");
    let card = if v.get("card_at").is_some() {
        ts(v, "card_at")
    } else {
        opened
    };
    let acts: Option<Vec<i64>> = v.get("operator_acts").map_or_else(
        || Some(Vec::new()),
        |a| {
            a.as_array().and_then(|a| {
                a.iter()
                    .map(|x| parse_iso(x.as_str()?))
                    .collect::<Option<Vec<_>>>()
            })
        },
    );
    let h_pre = acts
        .as_ref()
        .zip(card)
        .map(|(a, c)| a.iter().any(|t| *t < c));
    let n = acts.as_ref().map(|a| a.len() as u64);
    Distance {
        fingerprint: v
            .get("fingerprint")
            .and_then(|x| x.as_str())
            .unwrap_or("?")
            .to_owned(),
        first_seen: text("first_seen"),
        h_pre,
        l_secs: ts(v, "first_seen").zip(opened).map(|(f, o)| o - f),
        acts: n,
    }
}

/// Reads `<root>/incidents.jsonl`, keeping incidents opened since `since`.
///
/// `since` is Unix seconds. An incident whose `opened_at` cannot be read cannot be
/// placed in a window, so it is kept and reported NOT COMPUTABLE.
#[must_use]
pub fn measure_distance(root: &Path, since: i64) -> DistanceReport {
    let text = std::fs::read_to_string(root.join(INCIDENTS_FILE)).unwrap_or_default();
    let incidents = text
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| ts(v, "opened_at").is_none_or(|o| o >= since))
        .map(|v| measure_one(&v))
        .collect();
    DistanceReport { incidents }
}

fn median(sorted: &[i64]) -> String {
    let n = sorted.len();
    let (Some(&a), Some(&b)) = (sorted.get((n.saturating_sub(1)) / 2), sorted.get(n / 2)) else {
        return "n/a".to_owned();
    };
    if (a + b) % 2 == 0 {
        format!("{}", i64::midpoint(a, b))
    } else {
        format!("{}.5", (a + b).div_euclid(2))
    }
}

fn dist(out: &mut String, name: &str, unit: &str, total: usize, mut vals: Vec<i64>) {
    vals.sort_unstable();
    let list: Vec<String> = vals.iter().map(i64::to_string).collect();
    let _ = writeln!(
        out,
        "{name}: computable {}/{total} median={}{unit} values=[{}]",
        vals.len(),
        median(&vals),
        list.join(",")
    );
}

/// The deterministic text of a report (no clock, no ordering by chance).
#[must_use]
pub fn render(r: &DistanceReport) -> String {
    let total = r.incidents.len();
    if total == 0 {
        return "no incidents in the window\n".to_owned();
    }
    let mut out = format!("incidents in the window: {total}\n");
    let hp: Vec<i64> = r
        .incidents
        .iter()
        .filter_map(|d| d.h_pre.map(i64::from))
        .collect();
    let zero = hp.iter().filter(|v| **v == 0).count();
    let _ = writeln!(
        out,
        "H_pre: computable {}/{total} no_act_before_card={zero} act_before_card={}",
        hp.len(),
        hp.len() - zero
    );
    dist(
        &mut out,
        "L (seconds, first evidence to opened)",
        "s",
        total,
        r.incidents.iter().filter_map(|d| d.l_secs).collect(),
    );
    dist(
        &mut out,
        "A (operator acts)",
        "",
        total,
        r.incidents
            .iter()
            .filter_map(|d| d.acts.and_then(|a| i64::try_from(a).ok()))
            .collect(),
    );
    for d in &r.incidents {
        let mut miss = Vec::new();
        if d.h_pre.is_none() {
            miss.push("H_pre");
        }
        if d.l_secs.is_none() {
            miss.push("L");
        }
        if d.acts.is_none() {
            miss.push("A");
        }
        if !miss.is_empty() {
            let _ = writeln!(
                out,
                "NOT COMPUTABLE {} first_seen={}: {}",
                d.fingerprint,
                d.first_seen,
                miss.join(",")
            );
        }
    }
    out
}
