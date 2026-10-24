//! Triage helpers: OBUF byte → (word, lane) → C[m, n] → tile → PE / quadrant
//! mapping, memory dumps, and the self-test OBUF diff.

use crate::client::{Board, HostError};
use splitsys_cycle as cycle;
use splitsys_golden as golden;
use splitsys_spec as spec;
use std::fmt;

/// Where one OBUF byte comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location {
    /// Byte offset relative to the job's `out_base` byte address.
    pub offset: usize,
    /// OBUF word relative to `out_base`, and byte lane in it.
    pub word: usize,
    pub lane: usize,
    /// Output element (row m, column n) and, for int32 bypass, the byte within it.
    pub m: usize,
    pub n: usize,
    pub int32_byte: Option<usize>,
    /// Output tile (row block, column block) in the mode's tile shape.
    pub tile: (usize, usize),
    /// Physical PE that accumulated C[m, n] and its quadrant.
    pub pe: (usize, usize),
    pub quadrant: usize,
    /// False for padding rows/columns (unspecified per D-006).
    pub in_bounds: bool,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "+{} word {} lane {} -> C[{},{}]{} tile({},{}) PE({},{}) Q{}{}",
            self.offset,
            self.word,
            self.lane,
            self.m,
            self.n,
            self.int32_byte.map(|b| format!(" byte {b}")).unwrap_or_default(),
            self.tile.0,
            self.tile.1,
            self.pe.0,
            self.pe.1,
            self.quadrant,
            if self.in_bounds { "" } else { " (padding)" }
        )
    }
}

/// Inverse of `splitsys_cycle::Mode::pe_to_output` for a tile-local element.
pub fn output_to_pe(mode: u8, i: usize, j: usize) -> (usize, usize) {
    if mode == spec::MODE_STRIP && j >= spec::COLS {
        (i + spec::ROWS / 2, j - spec::COLS)
    } else {
        (i, j)
    }
}

/// Locate byte `offset` (relative to `out_base`) of a job's C extent (SPEC §1.3).
pub fn locate(desc: &golden::Descriptor, offset: usize) -> Location {
    let (m_total, _k, n_total) = desc.dims();
    let m_pad = golden::pack::m_pad(m_total);
    let wb = spec::OBUF_WORD_BYTES;
    let (word, lane) = (offset / wb, offset % wb);
    let (m, n, int32_byte) = if desc.out_int32() {
        // word index (nb*M_pad + m)*4 + q holds int32 lanes 4q..4q+3.
        let row_word = word / 4;
        let q = word % 4;
        let (nb, m) = (row_word / m_pad, row_word % m_pad);
        (m, 16 * nb + 4 * q + lane / 4, Some(lane % 4))
    } else {
        let (nb, m) = (word / m_pad, word % m_pad);
        (m, 16 * nb + lane, None)
    };
    let (tm, tn) = golden::consts::tile_shape(desc.mode);
    let pe = output_to_pe(desc.mode, m % tm, n % tn);
    Location {
        offset,
        word,
        lane,
        m,
        n,
        int32_byte,
        tile: (m / tm, n / tn),
        pe,
        quadrant: cycle::quadrant_of(pe.0 as u64, pe.1 as u64),
        in_bounds: m < m_total && n < n_total,
    }
}

/// Parse a region name or number.
pub fn parse_region(s: &str) -> Result<u8, String> {
    let l = s.to_ascii_uppercase();
    spec::REGIONS
        .iter()
        .find(|(name, id, _, _)| *name == l || id.to_string() == l)
        .map(|r| r.1)
        .ok_or_else(|| format!("unknown region '{s}' (wbuf | abuf | obuf | rqparam | 0..3)"))
}

/// Size in bytes of a region.
pub fn region_bytes(region: u8) -> usize {
    let (_, _, words, wb) = spec::REGIONS[usize::from(region)];
    words * wb
}

/// Hex dump, 16 bytes per line, with the absolute byte address and word index.
pub fn hexdump(region: u8, addr: usize, bytes: &[u8]) -> String {
    let wb = spec::REGIONS[usize::from(region)].3;
    let mut s = String::new();
    for (i, chunk) in bytes.chunks(16).enumerate() {
        let a = addr + i * 16;
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        s.push_str(&format!("{a:06x} (w{:>5}+{:>2}): {}\n", a / wb, a % wb, hex.join(" ")));
    }
    s
}

/// One differing byte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diff {
    pub loc: Location,
    pub got: u8,
    pub want: u8,
}

/// Self-test diff result for one ROM job.
#[derive(Clone, Debug)]
pub struct JobDiff {
    pub mode: u8,
    pub out_byte_addr: usize,
    pub len: usize,
    pub stored_fletcher: u32,
    pub readback_fletcher: u32,
    pub mismatches: usize,
    pub diffs: Vec<Diff>,
}

/// Read back each self-test job's checked OBUF range (out_base .. + CHECK_WORDS)
/// and diff it against golden's expected image. Call after a SELFTEST.
pub fn selftest_diff(board: &mut Board, max_diffs: usize) -> Result<Vec<JobDiff>, HostError> {
    let rom = splitsys_vec::selftest::build_rom();
    let mut out = Vec::new();
    for (j, job) in rom.jobs.iter().enumerate() {
        let len = rom.check_words[j] * spec::OBUF_WORD_BYTES;
        let addr = job.out_byte_addr();
        let back = board.read_mem(spec::REGION_OBUF, addr, len)?;
        let want = &job.obuf_image[..len.min(job.obuf_image.len())];
        let mut diffs = Vec::new();
        let mut mismatches = 0;
        for (i, (&g, &w)) in back.iter().zip(want).enumerate() {
            if g != w {
                mismatches += 1;
                if diffs.len() < max_diffs {
                    diffs.push(Diff {
                        loc: locate(&job.desc, i),
                        got: g,
                        want: w,
                    });
                }
            }
        }
        out.push(JobDiff {
            mode: job.desc.mode,
            out_byte_addr: addr,
            len,
            stored_fletcher: rom.fletcher[j],
            readback_fletcher: golden::fletcher32(&back),
            mismatches,
            diffs,
        });
    }
    Ok(out)
}

/// True if any two self-test jobs' checked OBUF ranges overlap (a later job would
/// then overwrite an earlier one's output before it can be read back).
pub fn selftest_ranges_overlap() -> bool {
    let rom = splitsys_vec::selftest::build_rom();
    let r: Vec<(usize, usize)> = rom
        .jobs
        .iter()
        .enumerate()
        .map(|(j, job)| {
            let a = job.out_byte_addr();
            (a, a + rom.check_words[j] * spec::OBUF_WORD_BYTES)
        })
        .collect();
    r.iter()
        .enumerate()
        .any(|(i, a)| r.iter().skip(i + 1).any(|b| a.0 < b.1 && b.0 < a.1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockConfig;
    use crate::transport::MockTransport;

    #[test]
    fn output_to_pe_inverts_cycle_mapping() {
        for (mode, cm) in [(spec::MODE_FULL, cycle::Mode::Full), (spec::MODE_STRIP, cycle::Mode::Strip)] {
            for r in 0..spec::ROWS {
                for c in 0..spec::COLS {
                    let (i, j) = cm.pe_to_output(r as u64, c as u64);
                    assert_eq!(output_to_pe(mode, i as usize, j as usize), (r, c), "{cm:?} PE({r},{c})");
                }
            }
        }
    }

    #[test]
    fn locate_matches_golden_offsets() {
        for (mode, flags) in [
            (spec::MODE_FULL, spec::FLAGS_INT8),
            (spec::MODE_STRIP, spec::FLAGS_INT8),
            (spec::MODE_STRIP, spec::FLAGS_INT32),
        ] {
            let d = golden::Descriptor { mode, flags, m: 13, k: 5, n: 37, ..Default::default() };
            for m in 0..13 {
                for n in 0..37 {
                    let off = if flags == spec::FLAGS_INT8 {
                        golden::pack::c_i8_offset(13, m, n)
                    } else {
                        golden::pack::c_i32_offset(13, m, n) + 2
                    };
                    let l = locate(&d, off);
                    assert_eq!((l.m, l.n), (m, n), "mode {mode} flags {flags} off {off}");
                    assert!(l.in_bounds);
                    let (tm, tn) = golden::consts::tile_shape(mode);
                    assert_eq!(l.tile, (m / tm, n / tn));
                }
            }
        }
        // STRIP: C[1, 8] (tile-local column 8) sits in PE(5, 0), quadrant Q2.
        let d = golden::Descriptor { mode: spec::MODE_STRIP, flags: spec::FLAGS_INT8, m: 4, k: 1, n: 16, ..Default::default() };
        let l = locate(&d, golden::pack::c_i8_offset(4, 1, 8));
        assert_eq!((l.pe, l.quadrant), ((5, 0), 2));
    }

    #[test]
    fn selftest_ranges_do_not_overlap() {
        assert!(!selftest_ranges_overlap());
    }

    #[test]
    fn selftest_diff_clean_and_faulty_mock() {
        let mut b = Board::new(Box::new(MockTransport::new(MockConfig::default())));
        let st = b.selftest().unwrap();
        assert!(st.pass_full && st.pass_strip);
        let d = selftest_diff(&mut b, 8).unwrap();
        assert_eq!(d.len(), 2);
        for j in &d {
            assert_eq!(j.mismatches, 0);
            assert_eq!(j.readback_fletcher, j.stored_fletcher);
        }
        // A flipped bit in the first read-back (the FULL job) is located.
        let mut b = Board::new(Box::new(MockTransport::new(MockConfig { flip_read_byte: 1, ..Default::default() })));
        b.selftest().unwrap();
        let d = selftest_diff(&mut b, 8).unwrap();
        assert_eq!(d[0].mismatches, 1);
        assert_eq!(d[1].mismatches, 0);
        let x = &d[0].diffs[0];
        assert_eq!(x.got ^ x.want, 0x10);
        assert_eq!(x.loc.offset, d[0].len / 2);
        assert!(x.loc.in_bounds);
        assert_ne!(d[0].readback_fletcher, d[0].stored_fletcher);
    }

    #[test]
    fn regions_parse_and_hexdump() {
        assert_eq!(parse_region("obuf").unwrap(), spec::REGION_OBUF);
        assert_eq!(parse_region("RQPARAM").unwrap(), spec::REGION_RQPARAM);
        assert_eq!(parse_region("1").unwrap(), spec::REGION_ABUF);
        assert!(parse_region("dram").is_err());
        let h = hexdump(spec::REGION_OBUF, 32, &[0xAB; 20]);
        assert!(h.starts_with("000020 (w    2+ 0): ab ab"), "{h}");
        assert_eq!(h.lines().count(), 2);
    }
}
