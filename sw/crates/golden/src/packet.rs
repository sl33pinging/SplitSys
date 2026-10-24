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
}
