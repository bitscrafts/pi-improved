//! Reciprocal-rank fusion (F4.9): `score += 1/(60 + rank)` per lane, then
//! temporal decay. Pure.

use crate::decay::{decay, lambda_for};
use crate::doc::DocKind;

/// RRF constant k (F4.9).
pub const RRF_K: u64 = 60;

/// Fuses ranked lists with RRF (F4.9): rank is 1-based per lane.
#[must_use]
pub fn rrf(lanes: &[Vec<String>]) -> Vec<(String, f32)> {
    let mut acc: Vec<(String, f32)> = Vec::new();
    for lane in lanes {
        for (i, doc) in lane.iter().enumerate() {
            let rank = u64::try_from(i + 1).unwrap_or(u64::MAX);
            #[allow(clippy::cast_precision_loss)] // reason: rrf weights, f32 contract
            let s = 1.0_f32 / (RRF_K + rank) as f32;
            if let Some(e) = acc.iter_mut().find(|(d, _)| d == doc) {
                e.1 += s;
            } else {
                acc.push((doc.clone(), s));
            }
        }
    }
    acc.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    acc
}

/// Applies decay to a fused score given the doc kind and age (F4.9). Pure.
#[must_use]
pub fn decayed(score: f32, kind: DocKind, age_days: f32) -> f32 {
    score * decay(lambda_for(kind), age_days)
}

mod decay {}
