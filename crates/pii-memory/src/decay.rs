//! Temporal decay (F4.9): `score *= exp(-λ * age_days)`; λ per kind; persona
//! never decays. Uses `Ctx::now_ms()` for table-testability.

/// λ for atoms (F4.9).
pub const LAMBDA_ATOM: f32 = 0.02;
/// λ for scenes.
pub const LAMBDA_SCENE: f32 = 0.005;
/// λ for skills.
pub const LAMBDA_SKILL: f32 = 0.001;
/// λ for persona (0: never decays).
pub const LAMBDA_PERSONA: f32 = 0.0;

/// Decay factor for an age in days (pure).
#[must_use]
pub fn decay(lambda: f32, age_days: f32) -> f32 {
    if lambda <= 0.0 {
        return 1.0;
    }
    let product = -(f64::from(lambda) * f64::from(age_days));
    #[allow(clippy::cast_possible_truncation)] // reason: f32 contract for scores
    {
        product.exp() as f32
    }
}

/// λ for a (layer, kind) pair (F4.9).
#[must_use]
pub fn lambda_for(kind: crate::doc::DocKind) -> f32 {
    use crate::doc::DocKind;
    match kind {
        DocKind::Atom => LAMBDA_ATOM,
        DocKind::Scene => LAMBDA_SCENE,
        DocKind::Skill => LAMBDA_SKILL,
        DocKind::Persona => LAMBDA_PERSONA,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{LAMBDA_ATOM, decay, lambda_for};
    use crate::doc::DocKind;

    #[test]
    fn decay_orders_by_age() {
        let young = decay(LAMBDA_ATOM, 1.0);
        let old = decay(LAMBDA_ATOM, 90.0);
        assert!(young > old);
        #[allow(clippy::cast_possible_truncation)] // reason: test fixture
        let expected: f32 = (-(f64::from(LAMBDA_ATOM))).exp() as f32;
        assert!((young - expected).abs() < 1e-6);
        // Persona never decays.
        assert!(lambda_for(DocKind::Persona).abs() < f32::EPSILON);
        assert!((decay(0.0, 1.0e9) - 1.0).abs() < 1e-6);
    }
}
