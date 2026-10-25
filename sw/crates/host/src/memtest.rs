//! `memtest`: random WRITE_MEM / READ_MEM round trips over all four regions
//! (checklist step H-3). Each frame picks a region, a possibly unaligned byte
//! address, and a length of 1..=1019 bytes (the WRITE_MEM single-frame maximum,
//! SPEC §3.8) inside the region, writes random data, reads it back, and compares
//! byte-exactly.

use crate::client::{Board, HostError, LinkStats};
use splitsys_golden::rng::Rng;
use splitsys_spec as spec;

/// Largest WRITE_MEM data length in one frame (MAX_PAYLOAD - region - addr).
pub const MAX_WRITE_DATA: usize = spec::MAX_PAYLOAD - 5;

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct MemtestReport {
    pub seed: u64,
    pub frames: u64,
    pub bytes: u64,
    /// Frames per region (WBUF, ABUF, OBUF, RQPARAM): H-3 requires all four exercised.
    pub per_region: [u64; spec::N_REGIONS],
    /// Frames whose read-back differed from what was written.
    pub bad_frames: u64,
    /// Mismatched bytes over all frames.
    pub bad_bytes: u64,
    /// Protocol errors (NACK_RANGE / NACK_OPCODE / BUSY / timeouts / retries exhausted).
    pub errors: u64,
    pub first_error: Option<String>,
    pub link: LinkStats,
}

impl MemtestReport {
    pub fn ok(&self) -> bool {
        self.bad_frames == 0 && self.errors == 0
    }
}

/// One frame's choice: (region, byte address, length).
pub fn pick(rng: &mut Rng) -> (u8, usize, usize) {
    let (_, region, words, wb) = spec::REGIONS[rng.below(spec::N_REGIONS as u64) as usize];
    let size = words * wb;
    let len = rng.range_usize(1, MAX_WRITE_DATA.min(size));
    let addr = rng.range_usize(0, size - len);
    (region, addr, len)
}

/// Run `frames` round trips. `progress(i, report)` is called after each frame.
pub fn memtest(
    board: &mut Board,
    frames: u64,
    seed: u64,
    mut progress: impl FnMut(u64, &MemtestReport),
) -> MemtestReport {
    let mut rng = Rng::new(seed);
    let mut r = MemtestReport {
        seed,
        ..Default::default()
    };
    for i in 0..frames {
        let (region, addr, len) = pick(&mut rng);
        let data: Vec<u8> = (0..len).map(|_| rng.next_u64() as u8).collect();
        let res: Result<Vec<u8>, HostError> = board
            .write_mem(region, addr, &data)
            .and_then(|_| board.read_mem(region, addr, len));
        r.frames += 1;
        r.per_region[usize::from(region)] += 1;
        match res {
            Ok(back) => {
                r.bytes += len as u64;
                let bad = data.iter().zip(&back).filter(|(a, b)| a != b).count() as u64
                    + back.len().abs_diff(len) as u64;
                if bad > 0 {
                    r.bad_frames += 1;
                    r.bad_bytes += bad;
                    if r.first_error.is_none() {
                        let off = data.iter().zip(&back).position(|(a, b)| a != b).unwrap_or(0);
                        r.first_error = Some(format!(
                            "frame {i}: region {region} addr {addr} len {len}: {bad} byte(s) differ, first at +{off} (wrote {:#04x}, read {:#04x})",
                            data.get(off).copied().unwrap_or(0),
                            back.get(off).copied().unwrap_or(0)
                        ));
                    }
                }
            }
            Err(e) => {
                r.errors += 1;
                if r.first_error.is_none() {
                    r.first_error =
                        Some(format!("frame {i}: region {region} addr {addr} len {len}: {e}"));
                }
            }
        }
        r.link = board.stats;
        progress(i, &r);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockConfig;
    use crate::transport::MockTransport;

    fn board(cfg: MockConfig) -> Board {
        Board::new(Box::new(MockTransport::new(cfg)))
    }

    #[test]
    fn picks_stay_in_bounds_and_cover_all_regions() {
        let mut rng = Rng::new(42);
        let mut seen = [false; spec::N_REGIONS];
        let mut unaligned = false;
        for _ in 0..2000 {
            let (region, addr, len) = pick(&mut rng);
            let (_, _, words, wb) = spec::REGIONS[usize::from(region)];
            assert!((1..=MAX_WRITE_DATA).contains(&len));
            assert!(addr + len <= words * wb);
            seen[usize::from(region)] = true;
            unaligned |= addr % wb != 0;
        }
        assert!(seen.iter().all(|&s| s), "all regions: {seen:?}");
        assert!(unaligned);
    }

    #[test]
    fn clean_mock_passes() {
        let mut b = board(MockConfig::default());
        let r = memtest(&mut b, 300, 7, |_, _| {});
        assert!(r.ok(), "{r:?}");
        assert_eq!(r.frames, 300);
        assert!(r.per_region.iter().all(|&c| c > 0), "{:?}", r.per_region);
        assert!(r.bytes > 300);
    }

    #[test]
    fn one_flipped_byte_is_caught() {
        let mut b = board(MockConfig {
            flip_read_byte: 1,
            ..Default::default()
        });
        let r = memtest(&mut b, 50, 7, |_, _| {});
        assert!(!r.ok());
        assert_eq!(r.bad_frames, 1);
        assert_eq!(r.bad_bytes, 1);
        assert!(r.first_error.as_deref().unwrap().contains("frame 0"), "{r:?}");
    }

    #[test]
    fn nack_crc_is_retried_and_counted() {
        let mut b = board(MockConfig {
            fail_rx_crc: 2,
            ..Default::default()
        });
        let r = memtest(&mut b, 20, 3, |_, _| {});
        assert!(r.ok(), "{r:?}");
        assert_eq!(r.link.nack_crc, 2);
    }

    #[test]
    fn works_over_tcp_fake_bridge() {
        let addr = crate::transport::fake::spawn_tcp_bridge(MockConfig::default(), 33);
        let t = crate::transport::open(
            &crate::transport::TransportSpec::Tcp(addr),
            115200, // any rate: tcp has no baud
            Some(1),
            MockConfig::default(),
        )
        .unwrap();
        let mut b = Board::new(t);
        let r = memtest(&mut b, 40, 11, |_, _| {});
        assert!(r.ok(), "{r:?}");
    }
}
