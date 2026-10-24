//! Memory images, SPEC §1.3. Every function returns or reads the exact byte image
//! of a buffer region in address order (little-endian), ready for WRITE_MEM.
//! A/W/C packers zero-pad out-of-range rows and columns (A/W padding must be zero;
//! C padding is unspecified on hardware, D-006).

use crate::consts::{
    A_BLOCK_ROWS, C_I32_PER_WORD, C_I32_WORDS_PER_ROW, OBUF_WORD_BYTES, RQ_WORD_BYTES, W_BLOCK_COLS,
};
use crate::mat::{MatI32, MatI8};
use crate::numerics::check_rq_params;

/// Number of 8-row A blocks: ceil(M/8).
pub fn m_blocks(m: usize) -> usize {
    m.div_ceil(A_BLOCK_ROWS)
}

/// `M_pad = ceil(M/8)*8`.
pub fn m_pad(m: usize) -> usize {
    m_blocks(m) * A_BLOCK_ROWS
}

/// Number of 16-column W / C blocks: ceil(N/16).
pub fn n_blocks(n: usize) -> usize {
    n.div_ceil(W_BLOCK_COLS)
}

/// ABUF words used by an M×K activation matrix.
pub fn a_words(m: usize, k: usize) -> usize {
    m_blocks(m) * k
}

/// WBUF words used by a K×N weight matrix.
pub fn w_words(k: usize, n: usize) -> usize {
    n_blocks(n) * k
}

/// OBUF words used by an M×N output (int8 or int32 bypass).
pub fn c_words(m: usize, n: usize, int32: bool) -> usize {
    n_blocks(n) * m_pad(m) * if int32 { C_I32_WORDS_PER_ROW } else { 1 }
}

/// A image: word64 index `mb*K + k` holds `A[8*mb + r, k]` in byte lane r.
pub fn pack_a_blocks8(a: &MatI8) -> Vec<u8> {
    let (m, k) = (a.rows(), a.cols());
    let mut out = vec![0u8; a_words(m, k) * A_BLOCK_ROWS];
    for mb in 0..m_blocks(m) {
        for kk in 0..k {
            for r in 0..A_BLOCK_ROWS {
                let row = mb * A_BLOCK_ROWS + r;
                if row < m {
                    out[(mb * k + kk) * A_BLOCK_ROWS + r] = a.get(row, kk) as u8;
                }
            }
        }
    }
    out
}

/// Inverse of [`pack_a_blocks8`].
pub fn unpack_a_blocks8(bytes: &[u8], m: usize, k: usize) -> MatI8 {
    assert!(
        bytes.len() >= a_words(m, k) * A_BLOCK_ROWS,
        "unpack_a: image too short"
    );
    MatI8::from_fn(m, k, |i, kk| {
        let (mb, r) = (i / A_BLOCK_ROWS, i % A_BLOCK_ROWS);
        bytes[(mb * k + kk) * A_BLOCK_ROWS + r] as i8
    })
}

/// W image: word128 index `nb*K + k` holds `W[k, 16*nb + c]` in byte lane c.
pub fn pack_w_blocks16(w: &MatI8) -> Vec<u8> {
    let (k, n) = (w.rows(), w.cols());
    let mut out = vec![0u8; w_words(k, n) * W_BLOCK_COLS];
    for nb in 0..n_blocks(n) {
        for kk in 0..k {
            for c in 0..W_BLOCK_COLS {
                let col = nb * W_BLOCK_COLS + c;
                if col < n {
                    out[(nb * k + kk) * W_BLOCK_COLS + c] = w.get(kk, col) as u8;
                }
            }
        }
    }
    out
}

/// Inverse of [`pack_w_blocks16`].
pub fn unpack_w_blocks16(bytes: &[u8], k: usize, n: usize) -> MatI8 {
    assert!(
        bytes.len() >= w_words(k, n) * W_BLOCK_COLS,
        "unpack_w: image too short"
    );
    MatI8::from_fn(k, n, |kk, j| {
        let (nb, c) = (j / W_BLOCK_COLS, j % W_BLOCK_COLS);
        bytes[(nb * k + kk) * W_BLOCK_COLS + c] as i8
    })
}

/// Byte offset of `C[m, n]` (int8) relative to `out_base`.
pub fn c_i8_offset(m_total: usize, m: usize, n: usize) -> usize {
    let (nb, c) = (n / W_BLOCK_COLS, n % W_BLOCK_COLS);
    (nb * m_pad(m_total) + m) * OBUF_WORD_BYTES + c
}

/// Byte offset of the first byte of `C[m, n]` (int32 bypass) relative to `out_base`.
/// Word128 index `(nb*M_pad + m)*4 + q` holds int32 lanes 4q..4q+3.
pub fn c_i32_offset(m_total: usize, m: usize, n: usize) -> usize {
    let (nb, c) = (n / W_BLOCK_COLS, n % W_BLOCK_COLS);
    let (q, l) = (c / C_I32_PER_WORD, c % C_I32_PER_WORD);
    ((nb * m_pad(m_total) + m) * C_I32_WORDS_PER_ROW + q) * OBUF_WORD_BYTES + l * 4
}

/// C int8 image: word128 index `nb*M_pad + m` holds `C[m, 16*nb + c]` in lane c.
pub fn pack_c_i8_blocks16(c: &MatI8) -> Vec<u8> {
    let (m, n) = (c.rows(), c.cols());
    let mut out = vec![0u8; c_words(m, n, false) * OBUF_WORD_BYTES];
    for i in 0..m {
        for j in 0..n {
            out[c_i8_offset(m, i, j)] = c.get(i, j) as u8;
        }
    }
    out
}

/// Read an M×N int8 result out of an OBUF image (starting at `out_base`).
pub fn unpack_c_i8_blocks16(bytes: &[u8], m: usize, n: usize) -> MatI8 {
    assert!(
        bytes.len() >= c_words(m, n, false) * OBUF_WORD_BYTES,
        "unpack_c_i8: image too short"
    );
    MatI8::from_fn(m, n, |i, j| bytes[c_i8_offset(m, i, j)] as i8)
}

/// C int32 bypass image.
pub fn pack_c_i32(c: &MatI32) -> Vec<u8> {
    let (m, n) = (c.rows(), c.cols());
    let mut out = vec![0u8; c_words(m, n, true) * OBUF_WORD_BYTES];
    for i in 0..m {
        for j in 0..n {
            let off = c_i32_offset(m, i, j);
            out[off..off + 4].copy_from_slice(&c.get(i, j).to_le_bytes());
        }
    }
    out
}

/// Read an M×N int32 result out of an OBUF image (starting at `out_base`).
pub fn unpack_c_i32(bytes: &[u8], m: usize, n: usize) -> MatI32 {
    assert!(
        bytes.len() >= c_words(m, n, true) * OBUF_WORD_BYTES,
        "unpack_c_i32: image too short"
    );
    MatI32::from_fn(m, n, |i, j| {
        let off = c_i32_offset(m, i, j);
        i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
    })
}

/// RQPARAM word for one column: `{8'h0, shift[7:0], mult[15:0]}`, little-endian.
pub fn rq_word(mult: u16, shift: u8) -> [u8; 4] {
    let w: u32 = u32::from(mult) | (u32::from(shift) << 16);
    w.to_le_bytes()
}

/// RQPARAM image: exactly N entries (columns `rq_base .. rq_base+N-1`, SPEC §1.3).
/// No padding entries: padding-column outputs are unspecified (D-006), and writing
/// past `rq_base+N` could exceed RQ_ENTRIES.
pub fn pack_rqparams(params: &[(u16, u8)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(params.len() * RQ_WORD_BYTES);
    for &(m, s) in params {
        check_rq_params(m, s);
        out.extend_from_slice(&rq_word(m, s));
    }
    out
}

/// Inverse of [`pack_rqparams`] for the first `n` columns.
pub fn unpack_rqparams(bytes: &[u8], n: usize) -> Vec<(u16, u8)> {
    (0..n)
        .map(|i| {
            let b = &bytes[i * RQ_WORD_BYTES..(i + 1) * RQ_WORD_BYTES];
            (u16::from_le_bytes([b[0], b[1]]), b[2])
        })
        .collect()
}
