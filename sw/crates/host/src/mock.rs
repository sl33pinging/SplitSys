//! In-process mock board: implements the SPEC §3.8 protocol byte-for-byte on top
//! of `splitsys_golden` (numerics, layouts, descriptor validation) and
//! `splitsys_cycle` (counter values). Everything it reports is MODEL data.
//!
//! Behaviour that the RTL contract does not pin down, and which the mock assumes
//! (listed in the host-dashboard handoff for spec-steward / rtl-designer):
//! - BUSY (SPEC §3.8 v1.1): RUN_GEMM, SELFTEST, WRITE_MEM, READ_MEM while a job runs.
//! - A descriptor that fails `Descriptor::validate` is answered NACK_RANGE.
//! - Requant parameters outside SPEC §1.2 at run time: job completes, OBUF untouched.
//! - A request whose `len` field exceeds MAX_PAYLOAD: the rest of that burst is
//!   dropped and NACK_RANGE is sent (RTL: after RX_TIMEOUT idle).
//! - Strictly request/response, like rtl/packet.sv: one response per frame.
//! - Wrong payload length for an opcode is answered NACK_RANGE.
//! - The GEMM "runs" for `busy_polls` POLLs (no wall-clock model).

use crate::frame::{encode_response, Decoded, Decoder};
use splitsys_cycle as cycle;
use splitsys_golden as golden;
use splitsys_golden::Descriptor;
use splitsys_spec as spec;

/// Build id the mock reports in PING (ASCII "MOCK"); never a real bitstream hash.
pub const MOCK_BUILD_ID: u32 = u32::from_be_bytes(*b"MOCK");

/// Knobs for fault injection and pacing.
#[derive(Clone, Debug)]
pub struct MockConfig {
    /// POLL answers BUSY this many times after each RUN_GEMM before DONE.
    pub busy_polls: u32,
    /// The next N received frames are treated as CRC failures (NACK_CRC).
    pub fail_rx_crc: u32,
    /// The CRC byte of the next N responses is corrupted.
    pub corrupt_tx: u32,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            busy_polls: 1,
            fail_rx_crc: 0,
            corrupt_tx: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RunState {
    Idle,
    Busy(u32),
    Done,
}

/// The mock board. Feed request bytes with [`MockBoard::rx`]; collect response
/// bytes from its return value.
pub struct MockBoard {
    pub cfg: MockConfig,
    dec: Decoder,
    mem: [Vec<u8>; spec::N_REGIONS],
    state: RunState,
    counters: cycle::Counters,
    pub display: Option<(u16, u8)>,
    /// Count of frames received (any status).
    pub frames_rx: u64,
    /// Count of GEMMs run.
    pub gemms: u64,
}

impl Default for MockBoard {
    fn default() -> Self {
        Self::new(MockConfig::default())
    }
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

impl MockBoard {
    pub fn new(cfg: MockConfig) -> Self {
        let mem = spec::REGIONS.map(|(_, _, words, wb)| vec![0u8; words * wb]);
        Self {
            cfg,
            dec: Decoder::requests(),
            mem,
            state: RunState::Idle,
            counters: cycle::Counters::default(),
            display: None,
            frames_rx: 0,
            gemms: 0,
        }
    }

    /// Feed received bytes; returns the bytes the board transmits in response.
    pub fn rx(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for &b in bytes {
            if let Some(d) = self.dec.push(b) {
                self.frames_rx += 1;
                // An oversize frame is dropped: the RTL ignores the rest of the burst
                // and answers NACK_RANGE after RX_TIMEOUT idle (end of this rx call).
                let too_long = matches!(d, Decoded::TooLong { .. });
                let (st, payload) = self.handle(d);
                let mut f = encode_response(st, &payload);
                if self.cfg.corrupt_tx > 0 {
                    self.cfg.corrupt_tx -= 1;
                    *f.last_mut().expect("frame") ^= 0xFF;
                }
                out.extend_from_slice(&f);
                if too_long {
                    self.dec.reset();
                    break;
                }
            }
        }
        out
    }

    /// Direct memory view (tests).
    pub fn region(&self, region: u8) -> &[u8] {
        &self.mem[usize::from(region)]
    }

    fn busy(&self) -> bool {
        matches!(self.state, RunState::Busy(_))
    }

    fn handle(&mut self, d: Decoded) -> (u8, Vec<u8>) {
        let f = match d {
            Decoded::Ok(f) => f,
            Decoded::BadCrc { .. } => return (spec::ST_NACK_CRC, vec![]),
            Decoded::TooLong { .. } => return (spec::ST_NACK_RANGE, vec![]),
        };
        if self.cfg.fail_rx_crc > 0 {
            self.cfg.fail_rx_crc -= 1;
            return (spec::ST_NACK_CRC, vec![]);
        }
        let p = &f.payload;
        let range = (spec::ST_NACK_RANGE, vec![]);
        // SPEC §3.8 v1.1 precedence: CRC → unknown opcode → BUSY → length/range.
        if !spec::OPCODES.iter().any(|(_, op)| *op == f.code) {
            return (spec::ST_NACK_OPCODE, vec![]);
        }
        if self.busy()
            && matches!(
                f.code,
                spec::OP_RUN_GEMM | spec::OP_SELFTEST | spec::OP_WRITE_MEM | spec::OP_READ_MEM
            )
        {
            return (spec::ST_BUSY, vec![]);
        }
        match f.code {
            spec::OP_PING => {
                if !p.is_empty() {
                    return range;
                }
                let mut r = spec::PING_MAGIC.to_vec();
                r.push(spec::PROTO_VERSION);
                r.extend_from_slice(&MOCK_BUILD_ID.to_le_bytes());
                (spec::ST_ACK, r)
            }
            spec::OP_POLL => {
                if !p.is_empty() {
                    return range;
                }
                let s = match self.state {
                    RunState::Idle => spec::POLL_IDLE,
                    RunState::Busy(0) => {
                        self.state = RunState::Done;
                        spec::POLL_DONE
                    }
                    RunState::Busy(n) => {
                        self.state = RunState::Busy(n - 1);
                        spec::POLL_BUSY
                    }
                    RunState::Done => spec::POLL_DONE,
                };
                (spec::ST_ACK, vec![s])
            }
            spec::OP_WRITE_MEM => {
                if p.len() < 6 {
                    return range;
                }
                let (region, addr, data) = (p[0], u32_at(p, 1) as usize, &p[5..]);
                match self.mem.get_mut(usize::from(region)) {
                    Some(m) if addr + data.len() <= m.len() => {
                        m[addr..addr + data.len()].copy_from_slice(data);
                        (spec::ST_ACK, vec![])
                    }
                    _ => range,
                }
            }
            spec::OP_READ_MEM => {
                if p.len() != 7 {
                    return range;
                }
                let (region, addr, len) = (p[0], u32_at(p, 1) as usize, usize::from(u16_at(p, 5)));
                if len == 0 || len > spec::MAX_PAYLOAD {
                    return range;
                }
                match self.mem.get(usize::from(region)) {
                    Some(m) if addr + len <= m.len() => (spec::ST_ACK, m[addr..addr + len].to_vec()),
                    _ => range,
                }
            }
            spec::OP_RUN_GEMM => {
                let Ok(desc) = Descriptor::from_bytes(p) else {
                    return range;
                };
                if desc.validate().is_err() {
                    return range;
                }
                self.execute(&desc);
                (spec::ST_ACK, vec![])
            }
            spec::OP_READ_COUNTERS => {
                if !p.is_empty() {
                    return range;
                }
                let mut r = Vec::with_capacity(spec::N_COUNTERS * spec::COUNTER_BYTES);
                for (name, v) in self.counters.readback() {
                    debug_assert!(spec::COUNTER_NAMES.contains(&name));
                    r.extend_from_slice(&v.to_le_bytes()[..spec::COUNTER_BYTES]);
                }
                (spec::ST_ACK, r)
            }
            spec::OP_SELFTEST => {
                if !p.is_empty() {
                    return range;
                }
                let ((pf, ff), (ps, fs)) = self.selftest();
                let mut r = vec![u8::from(pf), u8::from(ps)];
                r.extend_from_slice(&ff.to_le_bytes());
                r.extend_from_slice(&fs.to_le_bytes());
                (spec::ST_ACK, r)
            }
            spec::OP_SET_DISPLAY => {
                if p.len() != 3 || u16_at(p, 0) > 1000 || p[2] > 1 {
                    return range;
                }
                self.display = Some((u16_at(p, 0), p[2]));
                (spec::ST_ACK, vec![])
            }
            _ => (spec::ST_NACK_OPCODE, vec![]),
        }
    }

    /// Run the GEMM described by `desc` against the board memories.
    fn execute(&mut self, desc: &Descriptor) {
        let (m, k, n) = desc.dims();
        let a_off = desc.a_base as usize * spec::ABUF_WORD_BYTES;
        let w_off = desc.w_base as usize * spec::WBUF_WORD_BYTES;
        let rq_off = usize::from(desc.rq_base) * spec::RQ_WORD_BYTES;
        let o_off = desc.out_base as usize * spec::OBUF_WORD_BYTES;
        let abuf = &self.mem[usize::from(spec::REGION_ABUF)];
        let wbuf = &self.mem[usize::from(spec::REGION_WBUF)];
        let rqb = &self.mem[usize::from(spec::REGION_RQPARAM)];
        let a = golden::unpack_a_blocks8(&abuf[a_off..], m, k);
        let w = golden::unpack_w_blocks16(&wbuf[w_off..], k, n);
        let acc = golden::gemm_i32(&a, &w);
        // Requant params outside SPEC §1.2 (e.g. never-loaded RQPARAM): the RTL has no
        // error state (POLL ERROR is reserved), its outputs are just meaningless. The
        // mock leaves OBUF untouched in that case and still completes the job.
        let image = if desc.requant_en() {
            let rq = golden::unpack_rqparams(&rqb[rq_off..], n);
            rq.iter()
                .all(|&(mu, sh)| golden::rq_params_valid(mu, sh))
                .then(|| golden::pack_c_i8_blocks16(&golden::requant_mat(&acc, &rq)))
        } else {
            Some(golden::pack_c_i32(&acc))
        };
        let image = image.unwrap_or_default();
        let mask = golden::in_bounds_mask(desc);
        let obuf = &mut self.mem[usize::from(spec::REGION_OBUF)];
        for (i, (&b, &wr)) in image.iter().zip(&mask).enumerate() {
            if wr {
                obuf[o_off + i] = b;
            }
        }
        let mode = cycle::Mode::from_code(desc.mode).expect("validated mode");
        let cd = cycle::Descriptor::new(mode, desc.flags, m as u64, k as u64, n as u64);
        self.counters =
            cycle::predict(&cd, &cycle::Latencies::default()).expect("validated descriptor");
        self.gemms += 1;
        self.state = RunState::Busy(self.cfg.busy_polls);
    }
}

impl MockBoard {
    /// SELFTEST: load each ROM job (from `splitsys_vec::selftest::build_rom`) into the
    /// buffers like the RTL does (overwriting them), run it, and compare the
    /// Fletcher-32 of its C extent with the ROM's stored value. Afterwards the
    /// counters hold the last (STRIP) run and POLL reads DONE.
    fn selftest(&mut self) -> ((bool, u32), (bool, u32)) {
        let (jobs, stored) = selftest_jobs();
        let mut res = [(false, 0u32); 2];
        for (job, &stored) in jobs.iter().zip(&stored) {
            for (region, addr, img) in [
                (spec::REGION_ABUF, job.a_byte_addr(), &job.a_image),
                (spec::REGION_WBUF, job.w_byte_addr(), &job.w_image),
                (spec::REGION_RQPARAM, job.rq_byte_addr(), &job.rq_image),
            ] {
                self.mem[usize::from(region)][addr..addr + img.len()].copy_from_slice(img);
            }
            self.execute(&job.desc);
            let o = job.out_byte_addr();
            let got = golden::fletcher32(
                &self.mem[usize::from(spec::REGION_OBUF)][o..o + job.obuf_image.len()],
            );
            let slot = usize::from(job.desc.mode == spec::MODE_STRIP);
            res[slot] = (got == stored, got);
        }
        self.state = RunState::Done;
        (res[0], res[1])
    }
}

/// Self-test jobs and their stored Fletcher-32 values.
#[cfg(feature = "vec-rom")]
fn selftest_jobs() -> (Vec<golden::Job>, Vec<u32>) {
    let rom = splitsys_vec::selftest::build_rom();
    (rom.jobs, rom.fletcher)
}

/// Without the `vec-rom` feature: two stand-in jobs (M mod 8 = 0, N mod 16 = 0,
/// int8, per SPEC §3.9), stored values computed by golden. Not the ROM's values.
#[cfg(not(feature = "vec-rom"))]
fn selftest_jobs() -> (Vec<golden::Job>, Vec<u32>) {
    let mut rng = golden::rng::Rng::new(0x5E1F_7E57);
    let jobs: Vec<golden::Job> = [spec::MODE_FULL, spec::MODE_STRIP]
        .iter()
        .map(|&mode| {
            let (m, k, n) = (8, 24, 32);
            let a = golden::MatI8::from_fn(m, k, |_, _| rng.i8());
            let w = golden::MatI8::from_fn(k, n, |_, _| rng.i8());
            golden::Job::new(mode, a, w, Some(vec![(1234u16, 12u8); n]), golden::Bases::default())
                .expect("stand-in self-test job")
        })
        .collect();
    let f = jobs.iter().map(|j| golden::fletcher32(&j.obuf_image)).collect();
    (jobs, f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{encode_request, Decoder, Frame};

    fn txn(b: &mut MockBoard, op: u8, p: &[u8]) -> Frame {
        let out = b.rx(&encode_request(op, p));
        let mut d = Decoder::responses();
        let fr: Vec<Decoded> = out.iter().filter_map(|&x| d.push(x)).collect();
        assert_eq!(fr.len(), 1, "exactly one response per request");
        match fr.into_iter().next().unwrap() {
            Decoded::Ok(f) => f,
            other => panic!("bad response {other:?}"),
        }
    }

    fn start_job(b: &mut MockBoard) {
        let d = Descriptor { mode: spec::MODE_FULL, flags: spec::FLAGS_INT32, m: 1, k: 1, n: 1, ..Default::default() };
        assert_eq!(txn(b, spec::OP_RUN_GEMM, &d.to_bytes()).code, spec::ST_ACK);
    }

    #[test]
    fn error_precedence_and_busy_set() {
        let mut b = MockBoard::new(MockConfig { busy_polls: 5, ..Default::default() });
        start_job(&mut b);
        // unknown opcode beats BUSY
        assert_eq!(txn(&mut b, 0x7E, &[]).code, spec::ST_NACK_OPCODE);
        // BUSY beats a bad payload length
        assert_eq!(txn(&mut b, spec::OP_RUN_GEMM, &[0; 3]).code, spec::ST_BUSY);
        for op in [spec::OP_SELFTEST, spec::OP_READ_MEM, spec::OP_WRITE_MEM] {
            let f = txn(&mut b, op, &[]);
            assert_eq!(f.code, spec::ST_BUSY, "{op:#x}");
            assert!(f.payload.is_empty(), "error responses have len 0");
        }
        assert_eq!(txn(&mut b, spec::OP_PING, &[]).code, spec::ST_ACK);
        assert_eq!(txn(&mut b, spec::OP_READ_COUNTERS, &[]).payload.len(), 72);
        assert_eq!(txn(&mut b, spec::OP_SET_DISPLAY, &[0, 0, 0]).code, spec::ST_ACK);
        assert_eq!(txn(&mut b, spec::OP_POLL, &[]).payload, vec![spec::POLL_BUSY]);
    }

    #[test]
    fn set_display_ranges() {
        let mut b = MockBoard::default();
        assert_eq!(txn(&mut b, spec::OP_SET_DISPLAY, &[0xE8, 0x03, 1]).code, spec::ST_ACK); // 1000
        assert_eq!(txn(&mut b, spec::OP_SET_DISPLAY, &[0xE9, 0x03, 1]).code, spec::ST_NACK_RANGE); // 1001
        assert_eq!(txn(&mut b, spec::OP_SET_DISPLAY, &[0, 0, 2]).code, spec::ST_NACK_RANGE);
        assert_eq!(txn(&mut b, spec::OP_SET_DISPLAY, &[0, 0]).code, spec::ST_NACK_RANGE);
        assert_eq!(b.display, Some((1000, 1)));
    }

    #[test]
    fn oversize_frame_dropped_with_nack_range() {
        let mut b = MockBoard::default();
        let len = (spec::MAX_PAYLOAD + 1) as u16;
        let mut burst = vec![spec::SYNC_REQ, spec::OP_WRITE_MEM, len as u8, (len >> 8) as u8];
        burst.extend(std::iter::repeat_n(spec::SYNC_REQ, 1100)); // payload full of sync bytes
        let out = b.rx(&burst);
        let mut d = Decoder::responses();
        let fr: Vec<Decoded> = out.iter().filter_map(|&x| d.push(x)).collect();
        assert_eq!(fr, vec![Decoded::Ok(Frame { code: spec::ST_NACK_RANGE, payload: vec![] })]);
        // the board is usable afterwards
        assert_eq!(txn(&mut b, spec::OP_PING, &[]).code, spec::ST_ACK);
    }

    #[test]
    fn mem_payload_rules() {
        let mut b = MockBoard::default();
        // WRITE_MEM needs >= 1 data byte; READ_MEM rlen 1..=1024 within region
        assert_eq!(txn(&mut b, spec::OP_WRITE_MEM, &[0, 0, 0, 0, 0]).code, spec::ST_NACK_RANGE);
        assert_eq!(txn(&mut b, spec::OP_READ_MEM, &[0, 0, 0, 0, 0, 0, 0]).code, spec::ST_NACK_RANGE);
        assert_eq!(txn(&mut b, spec::OP_READ_MEM, &[0, 0, 0, 0, 0, 0x00, 0x04]).payload.len(), 1024);
        assert_eq!(txn(&mut b, spec::OP_READ_MEM, &[0, 0, 0, 0, 0, 0x00, 0x04]).code, spec::ST_ACK);
        assert_eq!(txn(&mut b, spec::OP_READ_MEM, &[0, 0, 0, 0, 0, 0x01, 0x04]).code, spec::ST_NACK_RANGE); // 1025
        assert_eq!(txn(&mut b, spec::OP_READ_MEM, &[4, 0, 0, 0, 0, 1, 0]).code, spec::ST_NACK_RANGE);
        assert_eq!(txn(&mut b, spec::OP_PING, &[1]).code, spec::ST_NACK_RANGE);
    }

    #[test]
    fn selftest_passes_and_leaves_done() {
        let mut b = MockBoard::default();
        let f = txn(&mut b, spec::OP_SELFTEST, &[]);
        assert_eq!(f.payload.len(), 10);
        assert_eq!(&f.payload[..2], &[1, 1]);
        assert_eq!(txn(&mut b, spec::OP_POLL, &[]).payload, vec![spec::POLL_DONE]);
    }
}
