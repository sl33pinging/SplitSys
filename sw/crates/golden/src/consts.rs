//! Spec constants re-typed for local use. Every value comes from `splitsys-spec`
//! (generated from `spec/spec.toml`); nothing here is a hand-written number.
#![allow(clippy::unnecessary_cast)]

use splitsys_spec as s;

/// Physical array rows / cols.
pub const ROWS: usize = s::ROWS as usize;
pub const COLS: usize = s::COLS as usize;

pub const MODE_FULL: u8 = s::MODE_FULL as u8;
pub const MODE_STRIP: u8 = s::MODE_STRIP as u8;
pub const MODE_INDEP: u8 = s::MODE_INDEP as u8;

pub const DESC_BYTES: usize = s::DESC_BYTES as usize;
pub const DESC_OFF_MODE: usize = s::DESC_OFF_MODE as usize;
pub const DESC_OFF_FLAGS: usize = s::DESC_OFF_FLAGS as usize;
pub const DESC_OFF_M: usize = s::DESC_OFF_M as usize;
pub const DESC_OFF_K: usize = s::DESC_OFF_K as usize;
pub const DESC_OFF_N: usize = s::DESC_OFF_N as usize;
pub const DESC_OFF_A_BASE: usize = s::DESC_OFF_A_BASE as usize;
pub const DESC_OFF_W_BASE: usize = s::DESC_OFF_W_BASE as usize;
pub const DESC_OFF_OUT_BASE: usize = s::DESC_OFF_OUT_BASE as usize;
pub const DESC_OFF_RQ_BASE: usize = s::DESC_OFF_RQ_BASE as usize;
pub const DESC_OFF_RESERVED: usize = s::DESC_OFF_RESERVED as usize;

pub const FLAG_REQUANT_EN: u8 = s::FLAG_REQUANT_EN as u8;
pub const FLAG_OUT_INT32: u8 = s::FLAG_OUT_INT32 as u8;

pub const M_MIN: usize = s::M_MIN as usize;
pub const M_MAX: usize = s::M_MAX as usize;
pub const K_MIN: usize = s::K_MIN as usize;
pub const K_MAX: usize = s::K_MAX as usize;
pub const N_MIN: usize = s::N_MIN as usize;
pub const N_MAX: usize = s::N_MAX as usize;

pub const RQ_MULT_MIN: u16 = s::RQ_MULT_MIN as u16;
pub const RQ_MULT_MAX: u16 = s::RQ_MULT_MAX as u16;
pub const RQ_SHIFT_MIN: u8 = s::RQ_SHIFT_MIN as u8;
pub const RQ_SHIFT_MAX: u8 = s::RQ_SHIFT_MAX as u8;

pub const ABUF_WORDS: usize = s::ABUF_WORDS as usize;
pub const WBUF_WORDS: usize = s::WBUF_WORDS as usize;
pub const OBUF_WORDS: usize = s::OBUF_WORDS as usize;
pub const RQ_ENTRIES: usize = s::RQ_ENTRIES as usize;

pub const ABUF_WORD_BYTES: usize = s::ABUF_WORD_BYTES as usize;
pub const WBUF_WORD_BYTES: usize = s::WBUF_WORD_BYTES as usize;
pub const OBUF_WORD_BYTES: usize = s::OBUF_WORD_BYTES as usize;
pub const RQ_WORD_BYTES: usize = s::RQ_WORD_BYTES as usize;

/// Rows per A block (= byte lanes of an ABUF word): one per physical array row.
pub const A_BLOCK_ROWS: usize = ABUF_WORD_BYTES;
/// Columns per W / C block (= byte lanes of a WBUF / OBUF word).
pub const W_BLOCK_COLS: usize = WBUF_WORD_BYTES;
/// int32 lanes per OBUF word in bypass mode.
pub const C_I32_PER_WORD: usize = OBUF_WORD_BYTES / 4;
/// OBUF words per drained row in int32 bypass mode.
pub const C_I32_WORDS_PER_ROW: usize = W_BLOCK_COLS / C_I32_PER_WORD;

pub const FULL_TILE_M: usize = s::FULL_TILE_M as usize;
pub const FULL_TILE_N: usize = s::FULL_TILE_N as usize;
pub const STRIP_TILE_M: usize = s::STRIP_TILE_M as usize;
pub const STRIP_TILE_N: usize = s::STRIP_TILE_N as usize;

pub const REGION_WBUF: u8 = s::REGION_WBUF as u8;
pub const REGION_ABUF: u8 = s::REGION_ABUF as u8;
pub const REGION_OBUF: u8 = s::REGION_OBUF as u8;
pub const REGION_RQPARAM: u8 = s::REGION_RQPARAM as u8;

/// Output tile (rows, cols) per mode (SPEC §3.2 / §3.5).
pub const fn tile_shape(mode: u8) -> (usize, usize) {
    if mode == MODE_STRIP {
        (STRIP_TILE_M, STRIP_TILE_N)
    } else {
        (FULL_TILE_M, FULL_TILE_N)
    }
}

// Compile-time consistency checks between the packing layout (SPEC §1.3) and the
// spec constants. If spec.toml changes these, the layout code must be revisited.
const _: () = assert!(A_BLOCK_ROWS == ROWS);
const _: () = assert!(W_BLOCK_COLS == 2 * COLS);
const _: () = assert!(OBUF_WORD_BYTES == W_BLOCK_COLS);
const _: () = assert!(RQ_WORD_BYTES == 4);
const _: () = assert!(OBUF_WORD_BYTES.is_multiple_of(4));
