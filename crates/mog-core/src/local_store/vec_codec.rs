//! Vector encoding for the `vec0` int8 index (MEM-08e, SP-21).
//!
//! sqlite-vec 0.1.9 built with `SQLITE_VEC_ENABLE_NEON` computes the int8 L2 in
//! `l2_sqr_int8_neon`: it widens to i16, subtracts, then squares with `vmulq_s16(diff, diff)`.
//! That product wraps when |diff| >= 182, and the distance comes back NULL; the scalar build does
//! not. So the quantizer keeps each value in `[-INT8_MAX, INT8_MAX]` with `INT8_MAX = 90`: a
//! difference is then at most 180, which is exact (tested). The exact re-rank runs on the f16
//! cache (SP-21). TODO(#269): SP-21 measured recall at scale 127; re-measure it at scale 90.

/// The largest int8 magnitude the quantizer writes.
pub const INT8_MAX: i8 = 90;

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
