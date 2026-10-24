//! `splitsys_golden`: the bit-exact reference for SplitSys (SPEC §1.2, §1.3, §3.5,
//! §3.8, §3.9). Pure functions, no I/O, no unsafe. If the RTL disagrees with this
//! crate, the RTL is wrong unless the spec says otherwise.

pub mod checksum;
pub mod consts;
pub mod descriptor;
pub mod job;
pub mod mat;
pub mod numerics;
pub mod pack;
pub mod packet;
pub mod rng;

pub use checksum::{crc8, fletcher32};
pub use descriptor::{DescError, Descriptor};
pub use job::{mask_to_ranges, obuf_written_mask, Bases, Job};
pub use mat::{Mat, MatI32, MatI8};
pub use numerics::{gemm_i32, gemm_q, requant, requant_is_tie, requant_mat, rq_params_valid};
pub use pack::{
    pack_a_blocks8, pack_c_i32, pack_c_i8_blocks16, pack_rqparams, pack_w_blocks16,
    unpack_a_blocks8, unpack_c_i32, unpack_c_i8_blocks16, unpack_rqparams, unpack_w_blocks16,
};

/// Pick a legal (mult, shift) so that `acc * mult / 2^shift ≈ acc * target / max_abs`,
/// i.e. an accumulator of magnitude `max_abs` maps to about `target`. Integer-only and
/// deterministic. Uses the largest shift that keeps `mult` in range (best precision).
pub fn choose_rq_param(max_abs: u64, target: u64) -> (u16, u8) {
    use consts::{RQ_MULT_MAX, RQ_MULT_MIN, RQ_SHIFT_MAX, RQ_SHIFT_MIN};
    let max_abs = max_abs.max(1);
    let mut best = (RQ_MULT_MIN, RQ_SHIFT_MIN);
    for shift in RQ_SHIFT_MIN..=RQ_SHIFT_MAX {
        // mult = round(target * 2^shift / max_abs)
        let num = u128::from(target) << shift;
        let mult = (num + u128::from(max_abs) / 2) / u128::from(max_abs);
        if mult > u128::from(RQ_MULT_MAX) {
            break;
        }
        if mult >= u128::from(RQ_MULT_MIN) {
            best = (mult as u16, shift);
        }
    }
    best
}
