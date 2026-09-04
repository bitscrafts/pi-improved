//! Quantisation (F4.6): i8 with f32 scale; similarity is an i32 dot product.

/// `q[i] = clamp(round(v[i] * 127), -127, 127)` plus the scale (F4.6).
///
/// The scale is the max |v[i]| so the quantised vector spans the full i8
/// range; dot products are scaled back at compare time.
#[must_use]
pub fn quantise(v: &[f32]) -> (Vec<i8>, f32) {
    let max = v
        .iter()
        .fold(0.0f32, |m, x| m.max(x.abs()))
        .clamp(1e-12, f32::MAX);
    let q: Vec<i8> = v
        .iter()
        .map(|x| {
            let s = (x / max) * 127.0;
            {
                #[allow(clippy::cast_possible_truncation)] // reason: clamped
                // to the i8 range by construction.
                let q = s.round().clamp(-127.0, 127.0) as i8;
                q
            }
        })
        .collect();
    (q, max)
}

/// i32 dot product of two quantised vectors (F4.6). Pure.
#[must_use]
pub fn dot_i8(a: &[i8], b: &[i8]) -> i32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| i32::from(*x) * i32::from(*y))
        .fold(0i32, i32::saturating_add)
}

/// Cosine of two quantised unit vectors, scaled back (F4.6). Pure.
#[must_use]
pub fn cosine_q(a: &[i8], sa: f32, b: &[i8], sb: f32) -> f32 {
    let n = dot_i8(a, b);
    let norm = 127.0f32 * 127.0f32;
    {
        #[allow(clippy::cast_precision_loss)] // reason: dot products over
        // i8 inputs fit f32's 23-bit mantissa for the dims used here.
        let nf = n as f32;
        (nf / norm) * sa * sb
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{cosine_q, dot_i8, quantise};

    fn unit2(x: f32, y: f32) -> (f32, f32) {
        let n = x.hypot(y);
        (x / n, y / n)
    }

    #[test]
    fn roundtrip_error_under_one_percent() {
        // Unit vectors at many angles: quantised cosine error < 1%.
        for step in 0..100 {
            let t = f64::from(step) * std::f64::consts::PI / 100.0;
            #[allow(clippy::cast_possible_truncation)] // reason: test fixture
            let (x, y) = unit2(t.cos() as f32, t.sin() as f32);
            #[allow(clippy::tuple_array_conversions)]
            // reason: literal pair, not a tuple conversion
            let q2: [f32; 2] = [x, y];
            let (qv, sc) = quantise(&q2);
            let c = cosine_q(&qv, sc, &qv, sc);
            assert!((c - 1.0).abs() < 0.01, "self cosine {c} at step {step}");
        }
        let a1: [f32; 2] = [1.0, 0.0];
        let (a, sa) = quantise(&a1);
        let b1: [f32; 2] = [0.0, 1.0];
        let (b, sb) = quantise(&b1);
        assert!(cosine_q(&a, sa, &b, sb).abs() < 0.01);
        assert_eq!(dot_i8(&[3, 4], &[2, 1]), 10);
    }
}
