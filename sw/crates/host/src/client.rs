//! Protocol driver: request/response transactions with CRC-retry, chunked memory
//! access, and the typed opcodes of SPEC §3.8.

use crate::frame::{encode_request, opcode_name, status_name, Decoded, Decoder};
use crate::transport::{Source, Transport};
use splitsys_cycle as cycle;
use splitsys_spec as spec;
use std::fmt;
use std::time::{Duration, Instant};

/// Retries after NACK_CRC (or a corrupted / missing response) before failing loudly.
pub const MAX_RETRIES: u32 = 3;
/// WRITE_MEM data bytes per frame: MAX_PAYLOAD minus region u8 + addr u32 (1019).
pub const WRITE_CHUNK: usize = spec::MAX_PAYLOAD - 5;
/// READ_MEM bytes per frame.
pub const READ_CHUNK: usize = spec::MAX_PAYLOAD;

#[derive(Debug, Clone, PartialEq)]
pub enum HostError {
    Io(String),
    /// No (complete) response within the per-frame timeout.
    Timeout { op: u8, after_ms: u64 },
    /// NACK_OPCODE / NACK_RANGE (not retried).
    Nack { op: u8, status: u8 },
    /// BUSY: the board is running a GEMM.
    Busy { op: u8 },
    /// MAX_RETRIES exhausted on NACK_CRC / bad response CRC.
    RetriesExhausted { op: u8, last: String },
    Protocol(String),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::Io(e) => write!(f, "I/O error: {e}"),
            HostError::Timeout { op, after_ms } => write!(
                f,
                "{} timed out after {after_ms} ms (no response; is the board connected and the right bitstream loaded?)",
                opcode_name(*op)
            ),
            HostError::Nack { op, status } => {
                write!(f, "{} rejected: {}", opcode_name(*op), status_name(*status))
            }
            HostError::Busy { op } => write!(f, "{} rejected: BUSY", opcode_name(*op)),
            HostError::RetriesExhausted { op, last } => write!(
                f,
                "{} failed after {MAX_RETRIES} retries (last: {last})",
                opcode_name(*op)
            ),
            HostError::Protocol(s) => write!(f, "protocol error: {s}"),
        }
    }
}

impl std::error::Error for HostError {}

pub type Result<T> = std::result::Result<T, HostError>;

/// Link statistics (for the operator console / dashboard).
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct LinkStats {
    pub frames: u64,
    pub retries: u64,
    pub nack_crc: u64,
    pub bad_rsp_crc: u64,
    pub timeouts: u64,
    pub busy: u64,
    pub bytes_tx: u64,
    pub bytes_rx: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PingInfo {
    pub version: u8,
    pub build_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelftestInfo {
    pub pass_full: bool,
    pub pass_strip: bool,
    pub fletcher_full: u32,
    pub fletcher_strip: u32,
}

/// A board connection over any transport.
pub struct Board {
    t: Box<dyn Transport>,
    dec: Decoder,
    pub stats: LinkStats,
    /// Base per-frame timeout before scaling.
    pub base_timeout: Duration,
    /// build_id from the last successful PING (see [`Board::effective_source`]).
    last_build_id: Option<u32>,
}

fn is_idempotent(op: u8) -> bool {
    op != spec::OP_RUN_GEMM && op != spec::OP_SELFTEST
}

impl Board {
    pub fn new(t: Box<dyn Transport>) -> Self {
        Self {
            t,
            dec: Decoder::responses(),
            stats: LinkStats::default(),
            base_timeout: Duration::from_millis(500),
            last_build_id: None,
        }
    }

    /// What the transport is (serial = hw, tcp = sim, in-process mock = model).
    /// Use [`Board::effective_source`] to label data.
    pub fn source(&self) -> Source {
        self.t.source()
    }

    /// The label for data from this board: the transport's source, except that a
    /// board which answered PING with the mock build id (e.g. `mock-bridge` over
    /// tcp:) is MODEL data whatever the transport. Before any PING this is the
    /// transport's source.
    pub fn effective_source(&self) -> Source {
        if self.last_build_id == Some(crate::mock::MOCK_BUILD_ID) {
            Source::Model
        } else {
            self.t.source()
        }
    }
    pub fn describe(&self) -> String {
        self.t.describe()
    }
    /// Raw access for `echo`.
    pub fn transport(&mut self) -> &mut dyn Transport {
        self.t.as_mut()
    }

    fn frame_timeout(&self, bytes: usize) -> Duration {
        let wire = self
            .t
            .bytes_per_sec()
            .map(|bps| Duration::from_secs_f64(bytes as f64 / bps))
            .unwrap_or_default();
        (self.base_timeout + wire) * self.t.timeout_scale()
    }

    /// Wait for one response frame (or timeout).
    fn read_response(&mut self, timeout: Duration) -> Result<Option<Decoded>> {
        let deadline = Instant::now() + timeout;
        let mut buf = [0u8; 2048];
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            let n = self
                .t
                .recv(&mut buf, deadline - now)
                .map_err(|e| HostError::Io(e.to_string()))?;
            if n == 0 {
                if self.t.source() == Source::Model {
                    // The mock answers synchronously; nothing more will arrive.
                    return Ok(None);
                }
                continue;
            }
            self.stats.bytes_rx += n as u64;
            // Bytes after a complete frame belong to no request we sent; drop them.
            for &b in &buf[..n] {
                if let Some(d) = self.dec.push(b) {
                    return Ok(Some(d));
                }
            }
        }
    }

    /// One request/response transaction with the retry policy of the role spec:
    /// NACK_CRC → resend (max 3); corrupted or missing response → resend for
    /// idempotent opcodes (max 3); NACK_OPCODE/NACK_RANGE/BUSY → error at once.
    pub fn transact(&mut self, op: u8, payload: &[u8], rsp_max: usize) -> Result<Vec<u8>> {
        let req = encode_request(op, payload);
        let timeout = self.frame_timeout(req.len() + rsp_max + 5);
        let mut last = String::new();
        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                self.stats.retries += 1;
            }
            self.dec.reset();
            self.t
                .send(&req)
                .map_err(|e| HostError::Io(e.to_string()))?;
            self.stats.frames += 1;
            self.stats.bytes_tx += req.len() as u64;
            match self.read_response(timeout)? {
                Some(Decoded::Ok(f)) => match f.code {
                    spec::ST_ACK => return Ok(f.payload),
                    spec::ST_NACK_CRC => {
                        self.stats.nack_crc += 1;
                        last = "NACK_CRC".into();
                    }
                    spec::ST_BUSY => {
                        self.stats.busy += 1;
                        return Err(HostError::Busy { op });
                    }
                    st @ (spec::ST_NACK_OPCODE | spec::ST_NACK_RANGE) => {
                        return Err(HostError::Nack { op, status: st })
                    }
                    st => {
                        return Err(HostError::Protocol(format!(
                            "{}: unknown status {st}",
                            opcode_name(op)
                        )))
                    }
                },
                Some(Decoded::BadCrc { expected, got, .. }) => {
                    self.stats.bad_rsp_crc += 1;
                    last = format!("response CRC mismatch (expected {expected:#04x}, got {got:#04x})");
                    if !is_idempotent(op) {
                        return Err(HostError::Protocol(format!(
                            "{}: {last}; not retried (non-idempotent), POLL to learn the board state",
                            opcode_name(op)
                        )));
                    }
                }
                Some(Decoded::TooLong { len, .. }) => {
                    last = format!("response len {len} > MAX_PAYLOAD");
                }
                None => {
                    self.stats.timeouts += 1;
                    if !is_idempotent(op) || attempt == MAX_RETRIES {
                        return Err(HostError::Timeout {
                            op,
                            after_ms: timeout.as_millis() as u64,
                        });
                    }
                    last = "timeout".into();
                }
            }
        }
        Err(HostError::RetriesExhausted { op, last })
    }

    pub fn ping(&mut self) -> Result<PingInfo> {
        let p = self.transact(spec::OP_PING, &[], 9)?;
        if p.len() != 9 || p[..4] != spec::PING_MAGIC {
            return Err(HostError::Protocol(format!(
                "PING: unexpected payload {p:02x?} (want \"SPLT\", version, build_id)"
            )));
        }
        let info = PingInfo {
            version: p[4],
            build_id: u32::from_le_bytes([p[5], p[6], p[7], p[8]]),
        };
        self.last_build_id = Some(info.build_id);
        Ok(info)
    }

    pub fn write_mem(&mut self, region: u8, addr: usize, data: &[u8]) -> Result<()> {
        for (i, chunk) in data.chunks(WRITE_CHUNK).enumerate() {
            let a = (addr + i * WRITE_CHUNK) as u32;
            let mut p = Vec::with_capacity(5 + chunk.len());
            p.push(region);
            p.extend_from_slice(&a.to_le_bytes());
            p.extend_from_slice(chunk);
            self.transact(spec::OP_WRITE_MEM, &p, 0)?;
        }
        Ok(())
    }

    pub fn read_mem(&mut self, region: u8, addr: usize, len: usize) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            let n = (len - out.len()).min(READ_CHUNK);
            let a = (addr + out.len()) as u32;
            let mut p = vec![region];
            p.extend_from_slice(&a.to_le_bytes());
            p.extend_from_slice(&(n as u16).to_le_bytes());
            let r = self.transact(spec::OP_READ_MEM, &p, n)?;
            if r.len() != n {
                return Err(HostError::Protocol(format!(
                    "READ_MEM returned {} bytes, asked for {n}",
                    r.len()
                )));
            }
            out.extend_from_slice(&r);
        }
        Ok(out)
    }

    pub fn run_gemm(&mut self, desc: &[u8; spec::DESC_BYTES]) -> Result<()> {
        self.transact(spec::OP_RUN_GEMM, desc, 0).map(|_| ())
    }

    pub fn poll(&mut self) -> Result<u8> {
        let p = self.transact(spec::OP_POLL, &[], 1)?;
        p.first()
            .copied()
            .ok_or_else(|| HostError::Protocol("POLL: empty payload".into()))
    }

    /// Poll until DONE. Errors on POLL_ERROR, on IDLE (job lost), or after `limit`.
    pub fn wait_done(&mut self, limit: Duration) -> Result<u32> {
        let t0 = Instant::now();
        let mut polls = 0u32;
        loop {
            polls += 1;
            match self.poll()? {
                spec::POLL_DONE => return Ok(polls),
                spec::POLL_BUSY => {}
                spec::POLL_ERROR => {
                    return Err(HostError::Protocol("POLL: board reports error state".into()))
                }
                spec::POLL_IDLE => {
                    return Err(HostError::Protocol(
                        "POLL: board idle after RUN_GEMM (job lost / board reset?)".into(),
                    ))
                }
                s => return Err(HostError::Protocol(format!("POLL: unknown state {s}"))),
            }
            if t0.elapsed() > limit * self.t.timeout_scale() {
                return Err(HostError::Protocol(format!(
                    "GEMM still busy after {polls} polls"
                )));
            }
            if self.t.source() != Source::Model {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }

    pub fn read_counters(&mut self) -> Result<cycle::Counters> {
        let want = spec::N_COUNTERS * spec::COUNTER_BYTES;
        let p = self.transact(spec::OP_READ_COUNTERS, &[], want)?;
        if p.len() != want {
            return Err(HostError::Protocol(format!(
                "READ_COUNTERS returned {} bytes, want {want}",
                p.len()
            )));
        }
        Ok(parse_counters(&p))
    }

    pub fn selftest(&mut self) -> Result<SelftestInfo> {
        let p = self.transact(spec::OP_SELFTEST, &[], 10)?;
        if p.len() != 10 {
            return Err(HostError::Protocol(format!(
                "SELFTEST returned {} bytes, want 10",
                p.len()
            )));
        }
        Ok(SelftestInfo {
            pass_full: p[0] != 0,
            pass_strip: p[1] != 0,
            fletcher_full: u32::from_le_bytes([p[2], p[3], p[4], p[5]]),
            fletcher_strip: u32::from_le_bytes([p[6], p[7], p[8], p[9]]),
        })
    }

    pub fn set_display(&mut self, util_permille: u16, mode: u8) -> Result<()> {
        let mut p = util_permille.to_le_bytes().to_vec();
        p.push(mode);
        self.transact(spec::OP_SET_DISPLAY, &p, 0).map(|_| ())
    }
}

/// Decode a READ_COUNTERS payload using the spec.toml counter order.
pub fn parse_counters(p: &[u8]) -> cycle::Counters {
    let v = |i: usize| {
        let mut b = [0u8; 8];
        b[..spec::COUNTER_BYTES]
            .copy_from_slice(&p[i * spec::COUNTER_BYTES..(i + 1) * spec::COUNTER_BYTES]);
        u64::from_le_bytes(b)
    };
    cycle::Counters {
        cycles_total: v(spec::CTR_CYCLES_TOTAL),
        cycles_setup: v(spec::CTR_CYCLES_SETUP),
        cycles_stream: v(spec::CTR_CYCLES_STREAM),
        cycles_flush: v(spec::CTR_CYCLES_FLUSH),
        cycles_drain: v(spec::CTR_CYCLES_DRAIN),
        cycles_stall: v(spec::CTR_CYCLES_STALL),
        mac_useful: v(spec::CTR_MAC_USEFUL),
        mac_useful_q: [
            v(spec::CTR_MAC_USEFUL_Q0),
            v(spec::CTR_MAC_USEFUL_Q1),
            v(spec::CTR_MAC_USEFUL_Q2),
            v(spec::CTR_MAC_USEFUL_Q3),
        ],
        tiles: v(spec::CTR_TILES),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{MockConfig, MOCK_BUILD_ID};
    use crate::transport::fake::{spawn_tcp_bridge, FakeSerial};
    use crate::transport::{MockTransport, StreamTransport};

    fn mock(cfg: MockConfig) -> Board {
        Board::new(Box::new(MockTransport::new(cfg)))
    }

    #[test]
    fn ping_mock() {
        let mut b = mock(MockConfig::default());
        let p = b.ping().unwrap();
        assert_eq!(p.version, spec::PROTO_VERSION);
        assert_eq!(p.build_id, MOCK_BUILD_ID);
    }

    #[test]
    fn chunked_write_read_roundtrip_all_regions() {
        let mut b = mock(MockConfig::default());
        for (_, region, words, wb) in spec::REGIONS {
            let len = words * wb;
            let data: Vec<u8> = (0..len).map(|i| (i * 7 + usize::from(region)) as u8).collect();
            b.write_mem(region, 0, &data).unwrap();
            assert_eq!(b.read_mem(region, 0, len).unwrap(), data);
            // unaligned partial access
            assert_eq!(b.read_mem(region, 3, 1500.min(len - 3)).unwrap(), data[3..3 + 1500.min(len - 3)]);
        }
        const { assert!(WRITE_CHUNK == 1019) };
    }

    #[test]
    fn out_of_range_is_nack_range_not_retried() {
        let mut b = mock(MockConfig::default());
        let e = b.read_mem(spec::REGION_OBUF, spec::OBUF_BYTES, 1).unwrap_err();
        assert_eq!(
            e,
            HostError::Nack {
                op: spec::OP_READ_MEM,
                status: spec::ST_NACK_RANGE
            }
        );
        let e = b.write_mem(9, 0, &[1]).unwrap_err();
        assert!(matches!(e, HostError::Nack { status: spec::ST_NACK_RANGE, .. }));
        assert_eq!(b.stats.retries, 0);
    }

    #[test]
    fn unknown_opcode_is_nack_opcode() {
        let mut b = mock(MockConfig::default());
        let e = b.transact(0x7F, &[], 0).unwrap_err();
        assert_eq!(
            e,
            HostError::Nack {
                op: 0x7F,
                status: spec::ST_NACK_OPCODE
            }
        );
    }

    #[test]
    fn nack_crc_is_retried_then_succeeds() {
        let mut b = mock(MockConfig {
            fail_rx_crc: 2,
            ..Default::default()
        });
        b.ping().unwrap();
        assert_eq!(b.stats.nack_crc, 2);
        assert_eq!(b.stats.retries, 2);
    }

    #[test]
    fn nack_crc_max_three_retries_then_fails_loudly() {
        let mut b = mock(MockConfig {
            fail_rx_crc: 10,
            ..Default::default()
        });
        let e = b.ping().unwrap_err();
        assert!(matches!(e, HostError::RetriesExhausted { op: spec::OP_PING, .. }), "{e:?}");
        assert_eq!(b.stats.frames, u64::from(MAX_RETRIES) + 1);
        assert!(e.to_string().contains("NACK_CRC"));
    }

    #[test]
    fn corrupted_response_retried_for_idempotent_ops() {
        let mut b = mock(MockConfig {
            corrupt_tx: 1,
            ..Default::default()
        });
        b.ping().unwrap();
        assert_eq!(b.stats.bad_rsp_crc, 1);
        assert_eq!(b.stats.retries, 1);
    }

    #[test]
    fn busy_while_running_and_poll_sequence() {
        let mut b = mock(MockConfig {
            busy_polls: 2,
            ..Default::default()
        });
        let job = splitsys_golden::Job::new(
            spec::MODE_FULL,
            splitsys_golden::MatI8::from_fn(3, 5, |r, c| (r * 5 + c) as i8),
            splitsys_golden::MatI8::from_fn(5, 7, |r, c| (r as i8) - (c as i8)),
            Some(vec![(100, 4); 7]),
            splitsys_golden::Bases::default(),
        )
        .unwrap();
        assert_eq!(b.poll().unwrap(), spec::POLL_IDLE);
        b.run_gemm(&job.desc.to_bytes()).unwrap();
        // While running: RUN_GEMM, SELFTEST, WRITE_MEM, READ_MEM answer BUSY.
        assert_eq!(
            b.run_gemm(&job.desc.to_bytes()).unwrap_err(),
            HostError::Busy { op: spec::OP_RUN_GEMM }
        );
        assert!(matches!(b.write_mem(0, 0, &[1]), Err(HostError::Busy { .. })));
        assert!(matches!(b.selftest(), Err(HostError::Busy { .. })));
        b.read_counters().unwrap(); // always served (SPEC §3.8 v1.1)
        b.set_display(500, 1).unwrap();
        assert!(matches!(b.read_mem(2, 0, 4), Err(HostError::Busy { .. })));
        b.ping().unwrap();
        assert_eq!(b.poll().unwrap(), spec::POLL_BUSY);
        assert_eq!(b.poll().unwrap(), spec::POLL_BUSY);
        assert_eq!(b.poll().unwrap(), spec::POLL_DONE);
        assert_eq!(b.poll().unwrap(), spec::POLL_DONE);
        let c = b.read_counters().unwrap();
        assert_eq!(c.mac_useful, 3 * 5 * 7);
    }

    #[test]
    fn bad_descriptor_is_nack_range() {
        let mut b = mock(MockConfig::default());
        let mut d = splitsys_golden::Descriptor {
            mode: spec::MODE_INDEP,
            flags: spec::FLAGS_INT8,
            m: 1,
            k: 1,
            n: 1,
            ..Default::default()
        };
        let e = b.run_gemm(&d.to_bytes()).unwrap_err();
        assert!(matches!(e, HostError::Nack { status: spec::ST_NACK_RANGE, .. }));
        d.mode = spec::MODE_FULL;
        d.m = 65;
        assert!(b.run_gemm(&d.to_bytes()).is_err());
    }

    #[test]
    fn counters_parse_in_spec_order() {
        let mut p = vec![0u8; spec::N_COUNTERS * spec::COUNTER_BYTES];
        for i in 0..spec::N_COUNTERS {
            let v = (i as u64 + 1) << 40 | i as u64;
            p[i * 6..i * 6 + 6].copy_from_slice(&v.to_le_bytes()[..6]);
        }
        let c = parse_counters(&p);
        let rb = c.readback();
        for (i, (name, v)) in rb.iter().enumerate() {
            assert_eq!(*name, spec::COUNTER_NAMES[i]);
            assert_eq!(*v, (i as u64 + 1) << 40 | i as u64);
        }
    }

    #[test]
    fn tcp_transport_against_fake_bridge() {
        let addr = spawn_tcp_bridge(MockConfig::default(), 7);
        let t = crate::transport::open(
            &crate::transport::TransportSpec::Tcp(addr),
            115200,
            Some(1),
            MockConfig::default(),
        )
        .unwrap();
        assert_eq!(t.source(), Source::Sim);
        let mut b = Board::new(t);
        assert_eq!(b.effective_source(), Source::Sim, "before PING: the transport's label");
        assert_eq!(b.ping().unwrap().build_id, MOCK_BUILD_ID);
        // The mock bridge serves MODEL data: never labelled SIM.
        assert_eq!(b.source(), Source::Sim);
        assert_eq!(b.effective_source(), Source::Model);
        let data: Vec<u8> = (0..3000).map(|i| i as u8).collect();
        b.write_mem(spec::REGION_WBUF, 16, &data).unwrap();
        assert_eq!(b.read_mem(spec::REGION_WBUF, 16, 3000).unwrap(), data);
    }

    #[test]
    fn tcp_nack_retry_against_fake_bridge() {
        let addr = spawn_tcp_bridge(
            MockConfig {
                fail_rx_crc: 1,
                corrupt_tx: 1,
                ..Default::default()
            },
            1,
        );
        let t = crate::transport::open(
            &crate::transport::TransportSpec::Tcp(addr),
            115200,
            Some(1),
            MockConfig::default(),
        )
        .unwrap();
        let mut b = Board::new(t);
        b.ping().unwrap();
        // NACK_CRC response itself corrupted → host sees a bad response CRC, resends once.
        assert_eq!(b.stats.retries, 1);
        assert_eq!(b.stats.bad_rsp_crc, 1);
    }

    #[test]
    fn serial_stream_transport_against_fake_line() {
        let t = StreamTransport::new(
            FakeSerial::new(MockConfig {
                fail_rx_crc: 1,
                ..Default::default()
            }),
            Source::Hw,
            "fake-serial".into(),
            1,
            Some(11520.0),
        );
        let mut b = Board::new(Box::new(t));
        assert_eq!(b.ping().unwrap().version, spec::PROTO_VERSION);
        assert_eq!(b.stats.nack_crc, 1);
        let st = b.selftest().unwrap();
        assert!(st.pass_full && st.pass_strip);
    }

    #[test]
    fn serial_timeout_when_line_is_silent() {
        use crate::transport::fake::EchoLine;
        // An echo line never produces a 0x5A response frame → timeout, retried, then error.
        let t = StreamTransport::new(EchoLine::default(), Source::Hw, "echo".into(), 1, None);
        let mut b = Board::new(Box::new(t));
        b.base_timeout = Duration::from_millis(20);
        let e = b.ping().unwrap_err();
        assert!(matches!(e, HostError::Timeout { .. }), "{e:?}");
        assert_eq!(b.stats.timeouts, u64::from(MAX_RETRIES) + 1);
    }
}
