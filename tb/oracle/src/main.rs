//! splitsys-tb-oracle: expected values for cocotb tests, from `splitsys_golden` only.
//!
//! Line protocol (stdin -> stdout), one response line per request line:
//!   gemm M K N a[M*K] w[K*N]   -> M*N int32 (row-major) = golden::gemm_i32(A, W)
//!   requant acc mult shift     -> "y tie unclamped"     = golden::requant / requant_is_tie / requant_unclamped
//!   crc8 HEX                   -> u8                    = golden::crc8
//!   fletcher32 HEX             -> u32                   = golden::fletcher32
//!   reqframe OPCODE HEX        -> HEX                   = golden::packet::request_frame
//!   rspframe STATUS HEX        -> HEX                   = golden::packet::response_frame
//! HEX is a contiguous lowercase/uppercase hex string ("-" for empty).
//! Any malformed request aborts with a non-zero exit (never a guessed value).

use splitsys_golden::numerics::requant_unclamped;
use splitsys_golden::packet::{request_frame, response_frame};
use splitsys_golden::{crc8, fletcher32, gemm_i32, requant, requant_is_tie, MatI8};
use std::io::{self, BufRead, BufWriter, Write};

fn hex_decode(s: &str) -> Vec<u8> {
    if s == "-" {
        return Vec::new();
    }
    assert!(s.len() % 2 == 0, "odd hex length");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("bad hex"))
        .collect()
}

fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn num<T: std::str::FromStr>(s: Option<&str>) -> T
where
    T::Err: std::fmt::Debug,
{
    s.expect("missing field").parse().expect("bad number")
}

fn main() {
    let stdin = io::stdin();
    let mut out = BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line.expect("stdin");
        let mut it = line.split_ascii_whitespace();
        let Some(cmd) = it.next() else { continue };
        let resp = match cmd {
            "gemm" => {
                let m: usize = num(it.next());
                let k: usize = num(it.next());
                let n: usize = num(it.next());
                let a: Vec<i8> = (0..m * k).map(|_| num(it.next())).collect();
                let w: Vec<i8> = (0..k * n).map(|_| num(it.next())).collect();
                assert!(it.next().is_none(), "trailing fields in gemm");
                let c = gemm_i32(&MatI8::from_vec(m, k, a), &MatI8::from_vec(k, n, w));
                c.data()
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            "requant" => {
                let acc: i32 = num(it.next());
                let mult: u16 = num(it.next());
                let shift: u8 = num(it.next());
                format!(
                    "{} {} {}",
                    requant(acc, mult, shift),
                    u8::from(requant_is_tie(acc, mult, shift)),
                    requant_unclamped(acc, mult, shift)
                )
            }
            "crc8" => crc8(&hex_decode(it.next().expect("hex"))).to_string(),
            "fletcher32" => fletcher32(&hex_decode(it.next().expect("hex"))).to_string(),
            "reqframe" => {
                let op: u8 = num(it.next());
                hex_encode(&request_frame(op, &hex_decode(it.next().expect("hex"))))
            }
            "rspframe" => {
                let st: u8 = num(it.next());
                hex_encode(&response_frame(st, &hex_decode(it.next().expect("hex"))))
            }
            other => panic!("unknown oracle command {other:?}"),
        };
        writeln!(out, "{resp}").expect("stdout");
    }
}
