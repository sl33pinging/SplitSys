//! A complete GEMM job: operands, memory images, descriptor, expected output.
//!
//! The mode (FULL / STRIP) changes only the RTL schedule. The memory images, the
//! expected OBUF image, and the set of compared bytes are mode-independent.
//!
//! SPEC §1.3 / D-006: OBUF bytes that are not an in-bounds `C[m, n]` (padding rows
//! `M..M_pad-1`, padding columns `N..16*ceil(N/16)-1`) are unspecified after a run.
//! `compare` marks exactly the in-bounds bytes; only those are checked.

use crate::consts::{ABUF_WORD_BYTES, OBUF_WORD_BYTES, RQ_WORD_BYTES, WBUF_WORD_BYTES};
use crate::descriptor::{DescError, Descriptor};
use crate::mat::{MatI32, MatI8};
use crate::numerics::{gemm_i32, requant_mat};
use crate::pack::{
    c_i32_offset, c_i8_offset, c_words, pack_a_blocks8, pack_c_i32, pack_c_i8_blocks16,
    pack_rqparams, pack_w_blocks16,
};

/// Word bases for the four regions (word indices; `rq` is an RQPARAM entry index).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bases {
    pub a: u32,
    pub w: u32,
    pub out: u32,
    pub rq: u16,
}

/// Everything a testbench or the host needs to run one GEMM and check it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub desc: Descriptor,
    pub a: MatI8,
    pub w: MatI8,
    /// Per-column (mult, shift); empty for int32 bypass jobs.
    pub rq: Vec<(u16, u8)>,
    /// int32 accumulators (always computed).
    pub acc: MatI32,
    /// Requantized output (None for bypass jobs).
    pub c: Option<MatI8>,
    /// ABUF image, to be written at byte address `a_base * 8`.
    pub a_image: Vec<u8>,
    /// WBUF image, at byte address `w_base * 16`.
    pub w_image: Vec<u8>,
    /// RQPARAM image (exactly N entries), at byte address `rq_base * 4`; empty for bypass.
    pub rq_image: Vec<u8>,
    /// Expected OBUF contents of the C extent, from byte address `out_base * 16`.
    /// Out-of-bounds bytes are shown as 0 but are unspecified (see `compare`).
    pub obuf_image: Vec<u8>,
    /// In-bounds bytes of `obuf_image` (the only bytes a check may compare).
    pub compare: Vec<bool>,
}

impl Job {
    /// Build and validate a job. `rq = None` means int32 bypass.
    pub fn new(
        mode: u8,
        a: MatI8,
        w: MatI8,
        rq: Option<Vec<(u16, u8)>>,
        bases: Bases,
    ) -> Result<Self, DescError> {
        assert_eq!(a.cols(), w.rows(), "Job::new: A cols != W rows");
        let (m, k, n) = (a.rows(), a.cols(), w.cols());
        let dim = |field: &'static str, v: usize| {
            u16::try_from(v).map_err(|_| DescError::DimOutOfRange {
                field,
                value: u16::MAX,
            })
        };
        let desc = Descriptor {
            mode,
            flags: Descriptor::flags_for(rq.is_some()),
            m: dim("M", m)?,
            k: dim("K", k)?,
            n: dim("N", n)?,
            a_base: bases.a,
            w_base: bases.w,
            out_base: bases.out,
            rq_base: bases.rq,
            reserved: 0,
        };
        desc.validate()?;
        if let Some(p) = &rq {
            assert_eq!(p.len(), n, "Job::new: need one requant param per column");
        }
        let acc = gemm_i32(&a, &w);
        let (c, obuf_image, rq_image) = match &rq {
            Some(p) => {
                let c = requant_mat(&acc, p);
                let img = pack_c_i8_blocks16(&c);
                (Some(c), img, pack_rqparams(p))
            }
            None => (None, pack_c_i32(&acc), Vec::new()),
        };
        let compare = in_bounds_mask(&desc);
        Ok(Self {
            a_image: pack_a_blocks8(&a),
            w_image: pack_w_blocks16(&w),
            desc,
            a,
            w,
            rq: rq.unwrap_or_default(),
            acc,
            c,
            rq_image,
            obuf_image,
            compare,
        })
    }

    /// The same job in another mode (only `desc.mode` changes).
    pub fn with_mode(&self, mode: u8) -> Self {
        let mut j = self.clone();
        j.desc.mode = mode;
        j.desc.validate().expect("with_mode: invalid mode");
        j
    }

    pub fn a_byte_addr(&self) -> usize {
        self.desc.a_base as usize * ABUF_WORD_BYTES
    }
    pub fn w_byte_addr(&self) -> usize {
        self.desc.w_base as usize * WBUF_WORD_BYTES
    }
    pub fn out_byte_addr(&self) -> usize {
        self.desc.out_base as usize * OBUF_WORD_BYTES
    }
    pub fn rq_byte_addr(&self) -> usize {
        usize::from(self.desc.rq_base) * RQ_WORD_BYTES
    }

    /// `compare` as merged `(offset, len)` byte ranges relative to `out_byte_addr()`.
    pub fn compare_ranges(&self) -> Vec<(usize, usize)> {
        mask_to_ranges(&self.compare)
    }

    /// True if every byte of the C extent is a defined (in-bounds) output, so a
    /// checksum over the whole extent is well defined (self-test requirement, D-001).
    pub fn extent_fully_defined(&self) -> bool {
        self.compare.iter().all(|&b| b)
    }

    /// Compare a read-back OBUF region (from `out_byte_addr()`) with the expected
    /// image on in-bounds bytes only. Returns the first mismatching byte offset.
    pub fn check_obuf(&self, readback: &[u8]) -> Result<(), usize> {
        for (i, (&want, &cmp)) in self.obuf_image.iter().zip(&self.compare).enumerate() {
            if cmp && readback.get(i) != Some(&want) {
                return Err(i);
            }
        }
        Ok(())
    }
}

/// Merge a byte mask into `(offset, len)` ranges.
pub fn mask_to_ranges(mask: &[bool]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (i, &b) in mask.iter().enumerate() {
        if !b {
            continue;
        }
        match out.last_mut() {
            Some((s, l)) if *s + *l == i => *l += 1,
            _ => out.push((i, 1)),
        }
    }
    out
}

/// In-bounds bytes of the C extent (relative to `out_base`): the bytes of every
/// logical `C[m, n]`, m < M, n < N, in the int8 or int32 layout chosen by `flags`.
pub fn in_bounds_mask(desc: &Descriptor) -> Vec<bool> {
    let (m, _k, n) = desc.dims();
    let int32 = desc.out_int32();
    let mut mask = vec![false; c_words(m, n, int32) * OBUF_WORD_BYTES];
    for row in 0..m {
        for col in 0..n {
            if int32 {
                let off = c_i32_offset(m, row, col);
                mask[off..off + 4].iter_mut().for_each(|b| *b = true);
            } else {
                mask[c_i8_offset(m, row, col)] = true;
            }
        }
    }
    mask
}
