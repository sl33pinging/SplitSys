//! GEMM and requantization, SPEC §1.2.

use crate::consts::{RQ_MULT_MAX, RQ_MULT_MIN, RQ_SHIFT_MAX, RQ_SHIFT_MIN};
use crate::mat::{MatI32, MatI8};

/// C = A · W with int32 accumulation. Naive triple loop.
///
/// The sum is formed in i64 and the low 32 bits are returned, which is what the
/// RTL presents at drain (SPEC §1.2). For K ≤ 256 the i64 sum always fits in i32.
pub fn gemm_i32(a: &MatI8, w: &MatI8) -> MatI32 {
    assert_eq!(a.cols(), w.rows(), "gemm_i32: inner dimensions differ");
    let k = a.cols();
    MatI32::from_fn(a.rows(), w.cols(), |i, j| {
        let mut acc: i64 = 0;
        for kk in 0..k {
            acc += i64::from(a.get(i, kk)) * i64::from(w.get(kk, j));
        }
        acc as i32 // low 32 bits, by definition
    })
}

/// Panics unless `mult` and `shift` are inside the SPEC §1.2 ranges.
pub fn check_rq_params(mult: u16, shift: u8) {
    assert!(
        (RQ_MULT_MIN..=RQ_MULT_MAX).contains(&mult),
        "requant: mult {mult} outside {RQ_MULT_MIN}..={RQ_MULT_MAX}"
    );
    assert!(
        (RQ_SHIFT_MIN..=RQ_SHIFT_MAX).contains(&shift),
        "requant: shift {shift} outside {RQ_SHIFT_MIN}..={RQ_SHIFT_MAX}"
    );
}

/// True if (mult, shift) is in range.
pub fn rq_params_valid(mult: u16, shift: u8) -> bool {
    (RQ_MULT_MIN..=RQ_MULT_MAX).contains(&mult) && (RQ_SHIFT_MIN..=RQ_SHIFT_MAX).contains(&shift)
}

/// SPEC §1.2, exactly:
/// `t = i64(acc)*i64(mult) + (1 << (shift-1)); y = clamp(t >>> shift, -128, 127)`.
/// `>>` on i64 is an arithmetic shift (floor). Panics on out-of-range parameters.
pub fn requant(acc: i32, mult: u16, shift: u8) -> i8 {
    check_rq_params(mult, shift);
    let t = i64::from(acc) * i64::from(mult) + (1i64 << (shift - 1));
    (t >> shift).clamp(-128, 127) as i8
}

/// The pre-clamp value `t >>> shift` (useful to classify saturation in vectors).
pub fn requant_unclamped(acc: i32, mult: u16, shift: u8) -> i64 {
    check_rq_params(mult, shift);
    let t = i64::from(acc) * i64::from(mult) + (1i64 << (shift - 1));
    t >> shift
}

/// True if `acc*mult / 2^shift` lies exactly halfway between two integers.
pub fn requant_is_tie(acc: i32, mult: u16, shift: u8) -> bool {
    check_rq_params(mult, shift);
    let p = i64::from(acc) * i64::from(mult);
    p.rem_euclid(1i64 << shift) == (1i64 << (shift - 1))
}

/// Requantize every element of an accumulator matrix with per-column params.
pub fn requant_mat(acc: &MatI32, params: &[(u16, u8)]) -> MatI8 {
    assert_eq!(
        params.len(),
        acc.cols(),
        "requant_mat: need one param per column"
    );
    MatI8::from_fn(acc.rows(), acc.cols(), |i, j| {
        let (m, s) = params[j];
        requant(acc.get(i, j), m, s)
    })
}

/// Quantized GEMM: `requant(gemm_i32(a, w))` with per-output-column (mult, shift).
pub fn gemm_q(a: &MatI8, w: &MatI8, params: &[(u16, u8)]) -> MatI8 {
    requant_mat(&gemm_i32(a, w), params)
}
