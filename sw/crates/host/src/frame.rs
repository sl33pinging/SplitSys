//! Host-link framing, SPEC §3.8.
//!
//! Request:  `0xA5 | opcode u8 | len u16 LE | payload[len] | crc8`
//! Response: `0x5A | status u8 | len u16 LE | payload[len] | crc8`
//! CRC-8 (poly/init from spec.toml) covers every byte after the sync byte up to
//! and including the payload. All constants come from `splitsys_spec`.

use splitsys_spec as spec;

/// CRC-8, MSB-first, no reflection, no final XOR; poly `CRC8_POLY`, init `CRC8_INIT`.
pub fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = spec::CRC8_INIT;
    for &b in bytes {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ spec::CRC8_POLY
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn encode(sync: u8, code: u8, payload: &[u8]) -> Vec<u8> {
    assert!(
        payload.len() <= spec::MAX_PAYLOAD,
        "payload {} > MAX_PAYLOAD {} (chunk it)",
        payload.len(),
        spec::MAX_PAYLOAD
    );
    let mut f = Vec::with_capacity(payload.len() + 5);
    f.push(sync);
    f.push(code);
    f.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    f.extend_from_slice(payload);
    let c = crc8(&f[1..]);
    f.push(c);
    f
}

/// Encode a request frame. Panics if `payload` exceeds `MAX_PAYLOAD`.
pub fn encode_request(opcode: u8, payload: &[u8]) -> Vec<u8> {
    encode(spec::SYNC_REQ, opcode, payload)
}

/// Encode a response frame (used by the mock board and test fakes).
pub fn encode_response(status: u8, payload: &[u8]) -> Vec<u8> {
    encode(spec::SYNC_RSP, status, payload)
}

/// One decoded frame. `code` is the opcode (request) or status (response).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub code: u8,
    pub payload: Vec<u8>,
}

/// What the streaming decoder produced for one complete frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decoded {
    Ok(Frame),
    /// The frame arrived complete but its CRC did not match.
    BadCrc { code: u8, expected: u8, got: u8 },
    /// The length field exceeds `MAX_PAYLOAD`; the decoder resynchronizes.
    TooLong { code: u8, len: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum St {
    Sync,
    Code,
    Len0,
    Len1,
    Payload,
    Crc,
}

/// Byte-at-a-time frame decoder with resynchronization on the sync byte.
/// Bytes before a sync byte are counted in `skipped` (line noise / garbage).
#[derive(Clone, Debug)]
pub struct Decoder {
    sync: u8,
    st: St,
    code: u8,
    len: usize,
    buf: Vec<u8>,
    pub skipped: u64,
}

impl Decoder {
    pub fn new(sync: u8) -> Self {
        Self {
            sync,
            st: St::Sync,
            code: 0,
            len: 0,
            buf: Vec::new(),
            skipped: 0,
        }
    }
    /// Decoder for responses (board → host).
    pub fn responses() -> Self {
        Self::new(spec::SYNC_RSP)
    }
    /// Decoder for requests (host → board; used by the mock board).
    pub fn requests() -> Self {
        Self::new(spec::SYNC_REQ)
    }

    /// Drop any partial frame (the RTL's RX_TIMEOUT behaviour).
    pub fn reset(&mut self) {
        self.st = St::Sync;
        self.buf.clear();
    }

    /// True while a frame is partially received.
    pub fn in_frame(&self) -> bool {
        self.st != St::Sync
    }

    /// Feed one byte; returns a frame result when one completes.
    pub fn push(&mut self, b: u8) -> Option<Decoded> {
        match self.st {
            St::Sync => {
                if b == self.sync {
                    self.st = St::Code;
                } else {
                    self.skipped += 1;
                }
                None
            }
            St::Code => {
                self.code = b;
                self.st = St::Len0;
                None
            }
            St::Len0 => {
                self.len = usize::from(b);
                self.st = St::Len1;
                None
            }
            St::Len1 => {
                self.len |= usize::from(b) << 8;
                self.buf.clear();
                if self.len > spec::MAX_PAYLOAD {
                    self.st = St::Sync;
                    return Some(Decoded::TooLong {
                        code: self.code,
                        len: self.len,
                    });
                }
                self.st = if self.len == 0 { St::Crc } else { St::Payload };
                None
            }
            St::Payload => {
                self.buf.push(b);
                if self.buf.len() == self.len {
                    self.st = St::Crc;
                }
                None
            }
            St::Crc => {
                self.st = St::Sync;
                let mut hdr = Vec::with_capacity(3 + self.buf.len());
                hdr.push(self.code);
                hdr.extend_from_slice(&(self.len as u16).to_le_bytes());
                hdr.extend_from_slice(&self.buf);
                let expected = crc8(&hdr);
                let payload = std::mem::take(&mut self.buf);
                if expected == b {
                    Some(Decoded::Ok(Frame {
                        code: self.code,
                        payload,
                    }))
                } else {
                    Some(Decoded::BadCrc {
                        code: self.code,
                        expected,
                        got: b,
                    })
                }
            }
        }
    }
}

/// Human name of a status code.
pub fn status_name(st: u8) -> &'static str {
    spec::STATUSES
        .iter()
        .find(|(_, v)| *v == st)
        .map(|(n, _)| *n)
        .unwrap_or("UNKNOWN_STATUS")
}

/// Human name of an opcode.
pub fn opcode_name(op: u8) -> &'static str {
    spec::OPCODES
        .iter()
        .find(|(_, v)| *v == op)
        .map(|(n, _)| *n)
        .unwrap_or("UNKNOWN_OPCODE")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc8_check_value_matches_spec() {
        assert_eq!(crc8(b"123456789"), spec::CRC8_CHECK);
        assert_eq!(crc8(&[]), spec::CRC8_INIT);
    }

    #[test]
    fn crc8_matches_golden_reference() {
        for n in 0..300usize {
            let v: Vec<u8> = (0..n).map(|i| (i * 31 + 7) as u8).collect();
            assert_eq!(crc8(&v), splitsys_golden::crc8(&v), "len {n}");
        }
    }

    #[test]
    fn request_layout_and_golden_agreement() {
        let f = encode_request(spec::OP_WRITE_MEM, &[1, 2, 3]);
        assert_eq!(f[0], spec::SYNC_REQ);
        assert_eq!(f[1], spec::OP_WRITE_MEM);
        assert_eq!(&f[2..4], &[3, 0]);
        assert_eq!(&f[4..7], &[1, 2, 3]);
        assert_eq!(f[7], crc8(&f[1..7]));
        assert_eq!(
            f,
            splitsys_golden::packet::request_frame(spec::OP_WRITE_MEM, &[1, 2, 3])
        );
        assert_eq!(
            encode_response(spec::ST_ACK, b"xyz"),
            splitsys_golden::packet::response_frame(spec::ST_ACK, b"xyz")
        );
    }

    #[test]
    fn decoder_roundtrip_all_lengths() {
        let mut d = Decoder::requests();
        for len in [0usize, 1, 2, 255, 256, 1023, 1024] {
            let p: Vec<u8> = (0..len).map(|i| (i ^ 0x5A) as u8).collect();
            let f = encode_request(0x11, &p);
            let mut out = None;
            for (i, &b) in f.iter().enumerate() {
                let r = d.push(b);
                if i + 1 < f.len() {
                    assert!(r.is_none());
                } else {
                    out = r;
                }
            }
            assert_eq!(
                out,
                Some(Decoded::Ok(Frame {
                    code: 0x11,
                    payload: p
                }))
            );
        }
    }

    #[test]
    fn decoder_resyncs_after_garbage_and_flags_bad_crc() {
        let mut d = Decoder::responses();
        let mut stream = vec![0x00, 0xFF, 0x13];
        let mut bad = encode_response(spec::ST_ACK, &[9, 9]);
        *bad.last_mut().unwrap() ^= 0x40;
        stream.extend_from_slice(&bad);
        stream.extend_from_slice(&encode_response(spec::ST_BUSY, &[]));
        let got: Vec<Decoded> = stream.iter().filter_map(|&b| d.push(b)).collect();
        assert_eq!(d.skipped, 3);
        assert_eq!(got.len(), 2);
        assert!(matches!(got[0], Decoded::BadCrc { code: 0, .. }));
        assert_eq!(
            got[1],
            Decoded::Ok(Frame {
                code: spec::ST_BUSY,
                payload: vec![]
            })
        );
    }

    #[test]
    fn decoder_rejects_oversize_len() {
        let mut d = Decoder::requests();
        let len = (spec::MAX_PAYLOAD + 1) as u16;
        let hdr = [spec::SYNC_REQ, 0x10, len as u8, (len >> 8) as u8];
        let got: Vec<Decoded> = hdr.iter().filter_map(|&b| d.push(b)).collect();
        assert_eq!(
            got,
            vec![Decoded::TooLong {
                code: 0x10,
                len: spec::MAX_PAYLOAD + 1
            }]
        );
        assert!(!d.in_frame());
    }

    #[test]
    #[should_panic(expected = "MAX_PAYLOAD")]
    fn encoder_refuses_oversize() {
        let _ = encode_request(0x10, &vec![0; spec::MAX_PAYLOAD + 1]);
    }

    #[test]
    fn names() {
        assert_eq!(status_name(spec::ST_NACK_CRC), "NACK_CRC");
        assert_eq!(opcode_name(spec::OP_RUN_GEMM), "RUN_GEMM");
    }
}
