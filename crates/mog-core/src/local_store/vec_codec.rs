//! Vector encoding for the `vec0` int8 index (MEM-08e, SP-21).
//!
//! sqlite-vec 0.1.9 built with `SQLITE_VEC_ENABLE_NEON` returns a NULL (NaN) L2 distance when the
//! difference of two int8 components leaves the i8 range; the scalar build does not. So the
//! quantizer keeps each value in `[-INT8_MAX, INT8_MAX]` with `INT8_MAX = 63`: a difference is
//! then at most 126 and always fits. The exact re-rank runs on the f16 cache (SP-21), so the
//! smaller int8 step costs little recall. TODO(#269): re-measure recall with this scale.

/// The largest int8 magnitude the quantizer writes.
pub const INT8_MAX: i8 = 63;

/// Quantize an L2-normalised vector for `vec_int8(?)`. NaN becomes 0.
pub fn quantize_int8(v: &[f32]) -> Vec<u8> {
    let m = f32::from(INT8_MAX);
    v.iter()
        .map(|x| {
            let q = if x.is_nan() {
                0.0
            } else {
                (x * m).round().clamp(-m, m)
            };
            (q as i8) as u8
        })
        .collect()
}
