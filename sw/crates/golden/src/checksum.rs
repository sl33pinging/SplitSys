//! Checksums: Fletcher-32 (self-test) and CRC-8 (packet layer, SPEC §3.8).

/// Fletcher-32 as used by the self-test.
///
/// - The input bytes are taken in address order and grouped into 16-bit words,
///   little-endian: `word[i] = bytes[2i] | bytes[2i+1] << 8`.
/// - Odd length: the missing high byte of the last word is 0.
/// - `sum1 = sum2 = 0` initially; per word: `sum1 = (sum1 + word) mod 65535`,
///   then `sum2 = (sum2 + sum1) mod 65535`. Both sums are always in 0..=65534
///   (an end-around-carry implementation must map 0xFFFF to 0).
/// - Result: `(sum2 << 16) | sum1`.
pub fn fletcher32(bytes: &[u8]) -> u32 {
    let mut s1: u32 = 0;
    let mut s2: u32 = 0;
    for ch in bytes.chunks(2) {
        let lo = u32::from(ch[0]);
        let hi = u32::from(ch.get(1).copied().unwrap_or(0));
        s1 = (s1 + (lo | (hi << 8))) % 65535;
        s2 = (s2 + s1) % 65535;
    }
    (s2 << 16) | s1
}

/// CRC-8, polynomial 0x07, init 0x00, no reflection, no final XOR (SPEC §3.8).
pub fn crc8(bytes: &[u8]) -> u8 {
    let mut crc: u8 = 0;
    for &b in bytes {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fletcher32_known_answers() {
        // Standard Fletcher-32 test vectors (16-bit little-endian words, mod 65535).
        assert_eq!(fletcher32(b"abcde"), 0xF04F_C729);
        assert_eq!(fletcher32(b"abcdef"), 0x5650_2D2A);
        assert_eq!(fletcher32(b"abcdefgh"), 0xEBE1_9591);
        assert_eq!(fletcher32(&[]), 0);
        // Odd length is the same as an explicit zero pad byte.
        assert_eq!(fletcher32(b"abcde"), fletcher32(b"abcde\0"));
        // All-0xFF words reduce to 0 mod 65535.
        assert_eq!(fletcher32(&[0xFF; 64]), 0);
    }

    #[test]
    fn crc8_known_answers() {
        assert_eq!(crc8(b"123456789"), 0xF4); // CRC-8/SMBUS check value
        assert_eq!(crc8(&[]), 0);
        assert_eq!(crc8(&[0x01]), 0x07);
    }
}
