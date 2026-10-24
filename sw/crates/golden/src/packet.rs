//! Frame encoding for the host link, SPEC §3.8 (reference for packet tests).

use crate::checksum::crc8;

/// Request sync byte.
pub const SYNC_REQ: u8 = 0xA5;
/// Response sync byte.
pub const SYNC_RSP: u8 = 0x5A;

fn frame(sync: u8, code: u8, payload: &[u8]) -> Vec<u8> {
    assert!(
        payload.len() <= usize::from(u16::MAX),
        "payload too long for u16 len"
    );
    let len = payload.len() as u16;
    let mut f = Vec::with_capacity(payload.len() + 5);
    f.push(sync);
    f.push(code);
    f.extend_from_slice(&len.to_le_bytes());
    f.extend_from_slice(payload);
    let crc = crc8(&f[1..]);
    f.push(crc);
    f
}

/// `0xA5 | opcode | len u16 LE | payload | crc8(opcode..payload)`.
pub fn request_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    frame(SYNC_REQ, opcode, payload)
}

/// `0x5A | status | len u16 LE | payload | crc8(status..payload)`.
pub fn response_frame(status: u8, payload: &[u8]) -> Vec<u8> {
    frame(SYNC_RSP, status, payload)
}

/// Decoded frame (sync-independent).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub sync: u8,
    pub code: u8,
    pub payload: Vec<u8>,
}

/// Why a byte string is not a complete, valid frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// Fewer bytes than the header + len + crc require.
    Truncated,
    /// The trailing CRC does not match.
    BadCrc { expected: u8, got: u8 },
    /// The first byte is not the expected sync byte.
    BadSync,
}

/// Parse exactly one frame from `bytes` (extra trailing bytes are ignored).
pub fn parse_frame(bytes: &[u8], sync: u8) -> Result<(Frame, usize), FrameError> {
    if bytes.is_empty() {
        return Err(FrameError::Truncated);
    }
    if bytes[0] != sync {
        return Err(FrameError::BadSync);
    }
    if bytes.len() < 4 {
        return Err(FrameError::Truncated);
    }
    let len = usize::from(u16::from_le_bytes([bytes[2], bytes[3]]));
    let total = 4 + len + 1;
    if bytes.len() < total {
        return Err(FrameError::Truncated);
    }
    let expected = crc8(&bytes[1..4 + len]);
    let got = bytes[4 + len];
    if expected != got {
        return Err(FrameError::BadCrc { expected, got });
    }
    Ok((
        Frame {
            sync,
            code: bytes[1],
            payload: bytes[4..4 + len].to_vec(),
        },
        total,
    ))
}

/// Parser-level outcome of a byte stream (reference for the RTL frame parser).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RxEvent {
    /// A complete frame with a good CRC (opcode/status validation happens above this).
    Frame { code: u8, payload: Vec<u8> },
    /// A complete frame with a bad CRC: respond NACK_CRC (len 0), payload ignored.
    BadCrc { code: u8, len: usize },
    /// `len > max_payload`: the frame is dropped, the parser waits for RX_TIMEOUT of
    /// idle, then responds NACK_RANGE (SPEC §3.8 v1.1, D-010).
    Oversize { code: u8, len: usize },
    /// The stream ended inside a frame: after RX_TIMEOUT of idle the parser resets and
    /// hunts for sync again. No response.
    Timeout,
}

/// Reference parser (SPEC §3.8, D-010). `stream` is sent back to back (no idle gaps)
/// and is followed by at least RX_TIMEOUT cycles of idle. Bytes outside a frame that
/// are not `sync` are skipped while hunting.
pub fn rx_reference(stream: &[u8], sync: u8, max_payload: usize) -> Vec<RxEvent> {
    let mut ev = Vec::new();
    let mut i = 0;
    while i < stream.len() {
        if stream[i] != sync {
            i += 1;
            continue;
        }
        if i + 4 > stream.len() {
            ev.push(RxEvent::Timeout);
            break;
        }
        let code = stream[i + 1];
        let len = usize::from(u16::from_le_bytes([stream[i + 2], stream[i + 3]]));
        if len > max_payload {
            ev.push(RxEvent::Oversize { code, len });
            break; // everything up to the idle gap is dropped
        }
        let end = i + 4 + len; // index of the crc byte
        if end >= stream.len() {
            ev.push(RxEvent::Timeout);
            break;
        }
        if crc8(&stream[i + 1..end]) == stream[end] {
            ev.push(RxEvent::Frame {
                code,
                payload: stream[i + 4..end].to_vec(),
            });
        } else {
            ev.push(RxEvent::BadCrc { code, len });
        }
        i = end + 1;
    }
    ev
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_errors() {
        let f = request_frame(0x10, &[1, 2, 3]);
        assert_eq!(f[0], SYNC_REQ);
        assert_eq!(&f[2..4], &[3, 0]);
        let (p, used) = parse_frame(&f, SYNC_REQ).unwrap();
        assert_eq!(used, f.len());
        assert_eq!(p.code, 0x10);
        assert_eq!(p.payload, vec![1, 2, 3]);
        let mut bad = f.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(matches!(
            parse_frame(&bad, SYNC_REQ),
            Err(FrameError::BadCrc { .. })
        ));
        assert_eq!(
            parse_frame(&f[..f.len() - 1], SYNC_REQ),
            Err(FrameError::Truncated)
        );
        assert_eq!(parse_frame(&f, SYNC_RSP), Err(FrameError::BadSync));
    }

    #[test]
    fn rx_reference_events() {
        let f = request_frame(0x10, &[1, 2, 3]);
        let mut two = f.clone();
        two.extend_from_slice(&request_frame(0x01, &[]));
        assert_eq!(
            rx_reference(&two, SYNC_REQ, 1024),
            vec![
                RxEvent::Frame {
                    code: 0x10,
                    payload: vec![1, 2, 3]
                },
                RxEvent::Frame {
                    code: 0x01,
                    payload: vec![]
                },
            ]
        );
        let mut bad = f.clone();
        bad[5] ^= 0x40;
        assert_eq!(
            rx_reference(&bad, SYNC_REQ, 1024),
            vec![RxEvent::BadCrc { code: 0x10, len: 3 }]
        );
        assert_eq!(
            rx_reference(&f[..6], SYNC_REQ, 1024),
            vec![RxEvent::Timeout]
        );
        assert_eq!(
            rx_reference(&f[..2], SYNC_REQ, 1024),
            vec![RxEvent::Timeout]
        );
        let big = [SYNC_REQ, 0x10, 0x01, 0x04, 0, 0];
        assert_eq!(
            rx_reference(&big, SYNC_REQ, 1024),
            vec![RxEvent::Oversize {
                code: 0x10,
                len: 1025
            }]
        );
        assert_eq!(rx_reference(&[0, 1, 2], SYNC_REQ, 1024), vec![]);
    }
}
