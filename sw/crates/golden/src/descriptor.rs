//! GEMM descriptor, SPEC §3.5. Byte offsets come from the generated field map.

use crate::consts::*;
use crate::pack::{a_words, c_words, rq_entries, w_words};

/// The 24-byte RUN_GEMM descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Descriptor {
    pub mode: u8,
    pub flags: u8,
    pub m: u16,
    pub k: u16,
    pub n: u16,
    /// Word index into ABUF.
    pub a_base: u32,
    /// Word index into WBUF.
    pub w_base: u32,
    /// Word index into OBUF.
    pub out_base: u32,
    /// Index into RQPARAM for column 0.
    pub rq_base: u16,
    pub reserved: u16,
}

/// Why a descriptor is not a legal job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DescError {
    WrongLength(usize),
    BadMode(u8),
    BadFlags(u8),
    DimOutOfRange {
        field: &'static str,
        value: u16,
    },
    BufferOverflow {
        buffer: &'static str,
        end: usize,
        size: usize,
    },
    ReservedNonZero(u16),
}

impl std::fmt::Display for DescError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for DescError {}

fn put_u16(b: &mut [u8], off: usize, v: u16) {
    b[off..off + 2].copy_from_slice(&v.to_le_bytes());
}
fn put_u32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
fn get_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}
fn get_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

impl Descriptor {
    /// Serialize to the little-endian wire/ROM format.
    pub fn to_bytes(&self) -> [u8; DESC_BYTES] {
        let mut b = [0u8; DESC_BYTES];
        b[DESC_OFF_MODE] = self.mode;
        b[DESC_OFF_FLAGS] = self.flags;
        put_u16(&mut b, DESC_OFF_M, self.m);
        put_u16(&mut b, DESC_OFF_K, self.k);
        put_u16(&mut b, DESC_OFF_N, self.n);
        put_u32(&mut b, DESC_OFF_A_BASE, self.a_base);
        put_u32(&mut b, DESC_OFF_W_BASE, self.w_base);
        put_u32(&mut b, DESC_OFF_OUT_BASE, self.out_base);
        put_u16(&mut b, DESC_OFF_RQ_BASE, self.rq_base);
        put_u16(&mut b, DESC_OFF_RESERVED, self.reserved);
        b
    }

    /// Parse (no range validation; see [`Descriptor::validate`]).
    pub fn from_bytes(b: &[u8]) -> Result<Self, DescError> {
        if b.len() != DESC_BYTES {
            return Err(DescError::WrongLength(b.len()));
        }
        Ok(Self {
            mode: b[DESC_OFF_MODE],
            flags: b[DESC_OFF_FLAGS],
            m: get_u16(b, DESC_OFF_M),
            k: get_u16(b, DESC_OFF_K),
            n: get_u16(b, DESC_OFF_N),
            a_base: get_u32(b, DESC_OFF_A_BASE),
            w_base: get_u32(b, DESC_OFF_W_BASE),
            out_base: get_u32(b, DESC_OFF_OUT_BASE),
            rq_base: get_u16(b, DESC_OFF_RQ_BASE),
            reserved: get_u16(b, DESC_OFF_RESERVED),
        })
    }

    pub fn requant_en(&self) -> bool {
        self.flags & FLAG_REQUANT_EN != 0
    }

    /// Output is the int32 bypass layout. SPEC §1.2: bypass iff `requant_en = 0`.
    pub fn out_int32(&self) -> bool {
        !self.requant_en()
    }

    /// Canonical flags for a job: requant (`requant_en=1, out_int32=0`) or bypass
    /// (`requant_en=0, out_int32=1`). Other combinations are not generated.
    pub fn flags_for(requant: bool) -> u8 {
        if requant {
            FLAG_REQUANT_EN
        } else {
            FLAG_OUT_INT32
        }
    }

    pub fn dims(&self) -> (usize, usize, usize) {
        (
            usize::from(self.m),
            usize::from(self.k),
            usize::from(self.n),
        )
    }

    /// Check the job against SPEC §3.5 ranges and buffer capacities.
    pub fn validate(&self) -> Result<(), DescError> {
        if self.mode != MODE_FULL && self.mode != MODE_STRIP {
            return Err(DescError::BadMode(self.mode));
        }
        if self.flags != Self::flags_for(true) && self.flags != Self::flags_for(false) {
            return Err(DescError::BadFlags(self.flags));
        }
        for (field, v, lo, hi) in [
            ("M", self.m, M_MIN, M_MAX),
            ("K", self.k, K_MIN, K_MAX),
            ("N", self.n, N_MIN, N_MAX),
        ] {
            if !(lo..=hi).contains(&usize::from(v)) {
                return Err(DescError::DimOutOfRange { field, value: v });
            }
        }
        if self.reserved != 0 {
            return Err(DescError::ReservedNonZero(self.reserved));
        }
        let (m, k, n) = self.dims();
        let checks = [
            ("ABUF", self.a_base as usize + a_words(m, k), ABUF_WORDS),
            ("WBUF", self.w_base as usize + w_words(k, n), WBUF_WORDS),
            (
                "OBUF",
                self.out_base as usize + c_words(m, n, self.out_int32()),
                OBUF_WORDS,
            ),
            (
                "RQPARAM",
                usize::from(self.rq_base) + rq_entries(n),
                RQ_ENTRIES,
            ),
        ];
        for (buffer, end, size) in checks {
            if end > size {
                return Err(DescError::BufferOverflow { buffer, end, size });
            }
        }
        Ok(())
    }
}
