//! Per-module unit vectors: `pe`, `requant`, `skew`, `packet`.

use crate::{hex, Meta, OutFile};
use serde_json::{json, Value};
use splitsys_golden::consts::{RQ_MULT_MAX, RQ_MULT_MIN, RQ_SHIFT_MAX, RQ_SHIFT_MIN};
use splitsys_golden::numerics::requant_unclamped;
use splitsys_golden::packet::{request_frame, response_frame};
use splitsys_golden::rng::Rng;
use splitsys_golden::{crc8, requant, requant_is_tie};

pub const MODULES: &[&str] = &["pe", "requant", "skew", "packet"];

/// Default `--n` per module.
pub fn default_n(module: &str) -> usize {
    match module {
        "pe" => 10_000,
        "requant" => 100_000,
        "skew" => 2_000,
        "packet" => 1_000,
        _ => 100,
    }
}

pub fn generate(module: &str, seed: u64, n: usize, meta: &Meta) -> Result<Vec<OutFile>, String> {
    match module {
        "pe" => Ok(pe(seed, n, meta)),
        "requant" => Ok(requant_vectors(seed, n, meta)),
        "skew" => Ok(skew(seed, n, meta)),
        "packet" => Ok(packet(seed, n, meta)),
        _ => Err(format!("unknown module '{module}'; known: {MODULES:?}")),
    }
}

// ------------------------------------------------------------------------- PE

/// One cycle of PE stimulus.
#[derive(Clone, Copy, Debug)]
struct PeCycle {
    a: i8,
    w: i8,
    vld: bool,
    first: bool,
}

/// PE vectors. Stimulus is per cycle (a, w, vld, first) at the PE inputs. `acc[t]` is
/// the accumulator value after cycle t's operands have been applied (zero-latency
/// model; the DUT shows it at `acc_out` L_PE cycles later). Each group is one
/// accumulation (from a `first` to the last valid operand before the next `first`).
pub fn pe(seed: u64, n: usize, meta: &Meta) -> Vec<OutFile> {
    let mut r = Rng::new(seed);
    let mut cyc: Vec<PeCycle> = Vec::new();
    // (start cycle, directed name) per group; the start cycle carries `first`.
    let mut starts: Vec<(usize, Option<String>)> = Vec::new();

    // Append one group: `len` valid operands, with optional vld gaps of garbage
    // operands inside (gap_prob %), then `tail_idle` idle cycles.
    #[allow(clippy::too_many_arguments)]
    fn stream(
        r: &mut Rng,
        cyc: &mut Vec<PeCycle>,
        starts: &mut Vec<(usize, Option<String>)>,
        name: Option<String>,
        len: usize,
        gen: &dyn Fn(&mut Rng, usize) -> (i8, i8),
        gap_prob: u64,
        tail_idle: usize,
    ) {
        starts.push((cyc.len(), name));
        for i in 0..len {
            if i > 0 && gap_prob > 0 {
                while r.chance(gap_prob, 100) {
                    let (a, w) = (r.i8(), r.i8());
                    cyc.push(PeCycle {
                        a,
                        w,
                        vld: false,
                        first: false,
                    });
                }
            }
            let (a, w) = gen(r, i);
            cyc.push(PeCycle {
                a,
                w,
                vld: true,
                first: i == 0,
            });
        }
        for _ in 0..tail_idle {
            let (a, w) = (r.i8(), r.i8());
            cyc.push(PeCycle {
                a,
                w,
                vld: false,
                first: false,
            });
        }
    }

    type Gen = Box<dyn Fn(&mut Rng, usize) -> (i8, i8)>;
    let directed: Vec<(&str, usize, Gen, u64, usize)> = vec![
        (
            "neg128_x_neg128_k256",
            256,
            Box::new(|_, _| (-128, -128)),
            0,
            0,
        ),
        (
            "neg128_x_pos127_k256",
            256,
            Box::new(|_, _| (-128, 127)),
            0,
            0,
        ),
        (
            "pos127_x_neg128_k256",
            256,
            Box::new(|_, _| (127, -128)),
            0,
            0,
        ),
        (
            "pos127_x_pos127_k256",
            256,
            Box::new(|_, _| (127, 127)),
            0,
            3,
        ),
        ("zeros_k64", 64, Box::new(|_, _| (0, 0)), 0, 0),
        ("single_neg128_sq", 1, Box::new(|_, _| (-128, -128)), 0, 0),
        (
            "alternating_sign",
            100,
            Box::new(|_, i| {
                if i % 2 == 0 {
                    (-128, -128)
                } else {
                    (127, -128)
                }
            }),
            0,
            0,
        ),
        (
            "vld_gaps_garbage",
            40,
            Box::new(|r, _| (r.i8(), r.i8())),
            50,
            2,
        ),
    ];
    for (name, len, f, gap, idle) in &directed {
        stream(
            &mut r,
            &mut cyc,
            &mut starts,
            Some(name.to_string()),
            *len,
            f.as_ref(),
            *gap,
            *idle,
        );
    }
    let rand_op: Gen = Box::new(|r, _| (r.i8(), r.i8()));
    // Back-to-back clears: 16 single-operand groups, no idle cycles between them.
    for i in 0..16 {
        stream(
            &mut r,
            &mut cyc,
            &mut starts,
            Some(format!("back_to_back_len1_{i}")),
            1,
            rand_op.as_ref(),
            0,
            0,
        );
    }
    // `first` mid-stream: short groups back to back (a new first while accumulating).
    for i in 0..8 {
        let len = 2 + i % 4;
        stream(
            &mut r,
            &mut cyc,
            &mut starts,
            Some(format!("first_mid_stream_{i}")),
            len,
            rand_op.as_ref(),
            0,
            0,
        );
    }
    let extreme: Gen = Box::new(|r, _| {
        (
            *r.pick(&[-128i8, 127, -1, 0, 1]),
            *r.pick(&[-128i8, 127, -1, 0, 1]),
        )
    });
    for _ in 0..n {
        let len = match r.below(4) {
            0 => r.range_usize(1, 4),
            1 => r.range_usize(200, 256),
            _ => r.range_usize(1, 256),
        };
        let g = if r.chance(1, 3) {
            extreme.as_ref()
        } else {
            rand_op.as_ref()
        };
        let gap = *r.pick(&[0u64, 0, 10, 40]);
        let idle = *r.pick(&[0usize, 0, 1, 5]);
        stream(&mut r, &mut cyc, &mut starts, None, len, g, gap, idle);
    }
    // Trailing idle cycles so the last result reaches acc_out.
    for _ in 0..8 {
        cyc.push(PeCycle {
            a: 0,
            w: 0,
            vld: false,
            first: false,
        });
    }

    // Zero-latency reference (wrapping = low 32 bits of the 48-bit P register).
    let mut acc: i32 = 0;
    let mut accs = Vec::with_capacity(cyc.len());
    for c in &cyc {
        if c.vld {
            let p = i32::from(c.a) * i32::from(c.w);
            acc = if c.first { p } else { acc.wrapping_add(p) };
        }
        accs.push(acc);
    }
    let mut groups: Vec<Value> = Vec::new();
    for (g, (s, name)) in starts.iter().enumerate() {
        let next = starts.get(g + 1).map_or(cyc.len(), |x| x.0);
        let end = (*s..next)
            .rev()
            .find(|&t| cyc[t].vld)
            .expect("group has a valid cycle");
        groups.push(json!({"index": g, "start": s, "end": end, "acc": accs[end], "name": name}));
    }

    let doc = json!({
        "meta": meta.json(seed, "unit/pe"),
        "description": "PE stimulus per cycle at the PE inputs (a_in, w_in, a_vld_in, a_first_in). \
    acc[t] = accumulator after cycle t's operands are applied (zero-latency model); compare acc_out \
    at cycle t + L_PE. groups: one accumulation each; acc at group end must appear at acc_out at end + L_PE.",
        "l_pe": splitsys_spec::L_PE,
        "cycles": cyc.len(),
        "a": cyc.iter().map(|c| c.a).collect::<Vec<_>>(),
        "w": cyc.iter().map(|c| c.w).collect::<Vec<_>>(),
        "vld": cyc.iter().map(|c| u8::from(c.vld)).collect::<Vec<_>>(),
        "first": cyc.iter().map(|c| u8::from(c.first)).collect::<Vec<_>>(),
        "acc": accs,
        "groups": groups,
    });
    let mut stim = meta.mem_header(
        seed,
        "unit/pe stimulus",
        &["one line per cycle: {first, vld, a[7:0], w[7:0]} = 18 bits".into()],
    );
    let mut exp = meta.mem_header(
        seed,
        "unit/pe expected",
        &[
            "one line per cycle: acc after that cycle (zero-latency model), int32 two's complement"
                .into(),
        ],
    );
    for (c, a) in cyc.iter().zip(&accs) {
        let v = (u32::from(c.first) << 17)
            | (u32::from(c.vld) << 16)
            | (u32::from(c.a as u8) << 8)
            | u32::from(c.w as u8);
        stim.push_str(&format!("{v:05x}\n"));
        exp.push_str(&format!("{:08x}\n", *a as u32));
    }
    vec![
        OutFile::json_compact("unit/pe.json", &doc),
        OutFile::text("unit/pe_stim.mem", stim),
        OutFile::text("unit/pe_acc.mem", exp),
    ]
}

// -------------------------------------------------------------------- requant

/// Directed requant cases (also embedded in `directed.json`).
pub fn requant_directed() -> Vec<(String, i32, u16, u8)> {
    let mut v: Vec<(String, i32, u16, u8)> = vec![
        ("tie_neg_half_to_0".into(), -1, 1, 1),
        ("tie_neg_1p5_to_neg1".into(), -3, 1, 1),
        ("tie_pos_half_to_1".into(), 1, 1, 1),
        ("tie_pos_1p5_to_2".into(), 3, 1, 1),
        ("tie_neg_2p5_mult3".into(), -10, 3, 2), // -7.5 -> -7
        ("tie_neg_127p5".into(), -255, 1, 1),
        ("tie_neg_128p5".into(), -257, 1, 1),
        ("tie_pos_126p5".into(), 253, 1, 1),
        ("tie_pos_127p5_sat".into(), 255, 1, 1),
        ("tie_shift31_neg".into(), -(1 << 30), 1, 31),
        ("tie_shift31_pos".into(), 1 << 30, 1, 31),
        ("zero_s1".into(), 0, 1, 1),
        ("zero_max".into(), 0, RQ_MULT_MAX, RQ_SHIFT_MAX),
        ("neg1_shift31".into(), -1, RQ_MULT_MIN, RQ_SHIFT_MAX),
        (
            "acc_max_sat_pos".into(),
            i32::MAX,
            RQ_MULT_MAX,
            RQ_SHIFT_MIN,
        ),
        (
            "acc_min_sat_neg".into(),
            i32::MIN,
            RQ_MULT_MAX,
            RQ_SHIFT_MIN,
        ),
        (
            "acc_max_shift31".into(),
            i32::MAX,
            RQ_MULT_MAX,
            RQ_SHIFT_MAX,
        ),
        (
            "acc_min_shift31".into(),
            i32::MIN,
            RQ_MULT_MAX,
            RQ_SHIFT_MAX,
        ),
        (
            "k256_worst_pos_shift31".into(),
            4_194_304,
            RQ_MULT_MAX,
            RQ_SHIFT_MAX,
        ),
        (
            "k256_worst_neg_shift31".into(),
            -4_161_536,
            RQ_MULT_MAX,
            RQ_SHIFT_MAX,
        ),
        ("sat_pos_edge_127".into(), 254, 1, 1),
        ("sat_pos_edge_128".into(), 256, 1, 1),
        ("sat_neg_edge_128".into(), -256, 1, 1),
        ("sat_neg_edge_129".into(), -258, 1, 1),
        ("mult1_shift1_big".into(), 1_000_000, 1, 1),
        ("mult32767_shift1_small".into(), 1, RQ_MULT_MAX, 1),
        ("mult32767_shift15".into(), -3, RQ_MULT_MAX, 15),
        ("product_bit47".into(), i32::MIN, RQ_MULT_MAX, 24),
    ];
    // Every shift with a tie just below and above zero, and saturation both ways.
    for s in RQ_SHIFT_MIN..=RQ_SHIFT_MAX {
        let half = 1i64 << (s - 1);
        if half <= i64::from(i32::MAX) {
            v.push((format!("tie_pos_s{s}"), half as i32, 1, s));
            v.push((format!("tie_neg_s{s}"), (-half) as i32, 1, s));
        }
        v.push((format!("sat_pos_s{s}"), i32::MAX, RQ_MULT_MAX, s));
        v.push((format!("sat_neg_s{s}"), i32::MIN, RQ_MULT_MAX, s));
    }
    v
}

fn requant_case_json(name: Option<&str>, acc: i32, m: u16, s: u8) -> Value {
    let y = requant(acc, m, s);
    let raw = requant_unclamped(acc, m, s);
    json!({
        "name": name, "acc": acc, "mult": m, "shift": s, "y": y,
        "tie": requant_is_tie(acc, m, s), "sat": raw != i64::from(y),
    })
}

/// Requant vectors: directed cases followed by `n` random ones.
pub fn requant_vectors(seed: u64, n: usize, meta: &Meta) -> Vec<OutFile> {
    let mut r = Rng::new(seed);
    let mut cases: Vec<(Option<String>, i32, u16, u8)> = requant_directed()
        .into_iter()
        .map(|(nm, a, m, s)| (Some(nm), a, m, s))
        .collect();
    for _ in 0..n {
        let m = match r.below(5) {
            0 => *r.pick(&[RQ_MULT_MIN, RQ_MULT_MAX]),
            _ => r.u16_in(RQ_MULT_MIN, RQ_MULT_MAX),
        };
        let s = match r.below(5) {
            0 => *r.pick(&[RQ_SHIFT_MIN, RQ_SHIFT_MAX]),
            _ => r.u8_in(RQ_SHIFT_MIN, RQ_SHIFT_MAX),
        };
        let acc = match r.below(6) {
            0 => r.i32(),
            1 => r.range_i64(-4_194_304, 4_194_304) as i32,
            2 => {
                // Force a tie: acc*m ≡ 2^(s-1) (mod 2^s) by picking acc with m = 1-ish.
                let y0 = r.range_i64(-200, 200);
                let half = 1i64 << (s - 1);
                let a = y0 * (1i64 << s) + half;
                if (i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&a) {
                    a as i32
                } else {
                    half.min(i64::from(i32::MAX)) as i32
                }
            }
            _ => {
                // Near the requant output range for these params.
                let scale = (1i64 << s) / i64::from(m).max(1);
                let span = (200 * scale).clamp(1, i64::from(i32::MAX));
                r.range_i64(-span, span) as i32
            }
        };
        cases.push((None, acc, m, s));
    }
    let arr: Vec<Value> = cases
        .iter()
        .map(|(nm, a, m, s)| requant_case_json(nm.as_deref(), *a, *m, *s))
        .collect();
    let n_directed = requant_directed().len();
    let doc = json!({
        "meta": meta.json(seed, "unit/requant"),
        "description": "SPEC §1.2: y = clamp((acc*mult + (1<<(shift-1))) >>> shift, -128, 127). \
    tie = exact .5 case (rounds toward +inf); sat = clamped. The first n_directed cases are named.",
        "n_directed": n_directed,
        "cases": arr,
    });
    let mut mem = meta.mem_header(
        seed,
        "unit/requant",
        &["one line per case: {acc[31:0], mult[15:0], shift[7:0], y[7:0]} = 64 bits".into()],
    );
    for (_, a, m, s) in &cases {
        let y = requant(*a, *m, *s);
        mem.push_str(&format!(
            "{:08x}{:04x}{:02x}{:02x}\n",
            *a as u32, m, s, y as u8
        ));
    }
    vec![
        OutFile::json_compact("unit/requant.json", &doc),
        OutFile::text("unit/requant.mem", mem),
    ]
}

// ----------------------------------------------------------------------- skew

/// Skew vectors for the A unit (lane j delayed j) and the W unit in STRIP mode
/// (lane j delayed `W_SKEW_BASE_STRIP[j] + (j % COLS)`, per the generated spec).
/// `out[t][j] = in[t - delay[j]][j]` for t ≥ delay[j], else 0 (reset state).
pub fn skew(seed: u64, n: usize, meta: &Meta) -> Vec<OutFile> {
    let mut r = Rng::new(seed);
    let cols = splitsys_spec::COLS;
    let a_delays: Vec<usize> = (0..splitsys_spec::A_LANES).collect();
    let w_full_delays: Vec<usize> = (0..splitsys_spec::W_LANES).map(|j| j % cols).collect();
    let w_strip_delays: Vec<usize> = (0..splitsys_spec::W_LANES)
        .map(|j| splitsys_spec::W_SKEW_BASE_STRIP[j] + (j % cols))
        .collect();
    let configs: Vec<(&str, &str, Vec<usize>)> = vec![
        (
            "a",
            "A skew, all modes: lane j delayed j (BASE = 0)",
            a_delays,
        ),
        (
            "w_full",
            "W skew, FULL: lanes 0..7 delayed j; lanes 8..15 carry the second FULL half-block \
and are compared only if the DUT drives them (they are unused in FULL)",
            w_full_delays,
        ),
        (
            "w_strip",
            "W skew, STRIP: lane j delayed W_SKEW_BASE_STRIP[j] + (j % COLS) => lanes 8..15 \
delayed 4 + c (SPEC §3.2)",
            w_strip_delays,
        ),
    ];
    let mut out = Vec::new();
    for (name, desc, delays) in configs {
        let lanes = delays.len();
        let maxd = *delays.iter().max().unwrap_or(&0);
        let t_in = n.max(1);
        let total = t_in + maxd + 2;
        // Input per cycle per lane: (data, vld, first). Inputs are zero after t_in.
        let mut inp = vec![vec![(0u8, 0u8, 0u8); lanes]; total];
        for (t, row) in inp.iter_mut().enumerate().take(t_in) {
            for cell in row.iter_mut() {
                let vld = u8::from(r.chance(3, 4));
                let first = u8::from(vld == 1 && (t == 0 || r.chance(1, 16)));
                *cell = (r.next_u64() as u8, vld, first);
            }
        }
        let mut exp = vec![vec![(0u8, 0u8, 0u8); lanes]; total];
        for t in 0..total {
            for j in 0..lanes {
                if t >= delays[j] {
                    exp[t][j] = inp[t - delays[j]][j];
                }
            }
        }
        let lane_major = |m: &Vec<Vec<(u8, u8, u8)>>, f: fn(&(u8, u8, u8)) -> u8| -> Vec<Vec<u8>> {
            (0..lanes)
                .map(|j| m.iter().map(|row| f(&row[j])).collect())
                .collect()
        };
        let doc = json!({
            "meta": meta.json(seed, &format!("unit/skew_{name}")),
            "description": desc,
            "rule": "out_*[j][t] = in_*[j][t - delay[j]] if t >= delay[j] else 0; arrays are lane-major [lane][cycle]",
            "lanes": lanes,
            "delays": delays,
            "cycles": total,
            "in_data": lane_major(&inp, |c| c.0),
            "in_vld": lane_major(&inp, |c| c.1),
            "in_first": lane_major(&inp, |c| c.2),
            "out_data": lane_major(&exp, |c| c.0),
            "out_vld": lane_major(&exp, |c| c.1),
            "out_first": lane_major(&exp, |c| c.2),
        });
        out.push(OutFile::json_compact(
            format!("unit/skew_{name}.json"),
            &doc,
        ));
    }
    out
}

// --------------------------------------------------------------------- packet

/// Packet-layer vectors: CRC-8 known answers, good request/response frames, bad CRC,
/// truncated frames, and a back-to-back stream.
pub fn packet(seed: u64, n: usize, meta: &Meta) -> Vec<OutFile> {
    let mut r = Rng::new(seed);
    let opcodes: Vec<u8> = splitsys_spec::OPCODES.iter().map(|(_, c)| *c).collect();
    let max_payload = splitsys_spec::MAX_PAYLOAD;
    let rand_payload = |r: &mut Rng, max: usize| -> Vec<u8> {
        let len = match r.below(4) {
            0 => *r.pick(&[0usize, 1, max - 1, max]),
            1 => r.range_usize(0, max),
            _ => r.range_usize(0, 32),
        };
        (0..len).map(|_| r.next_u64() as u8).collect()
    };
    let frame_json =
        |kind: &str, dir: &str, code: u8, payload: &[u8], bytes: &[u8], expect: &str| {
            json!({
                "kind": kind, "dir": dir, "code": code, "len": payload.len(),
                "payload_hex": hex(payload), "crc": crc8(&{
                    let mut v = vec![code];
                    v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
                    v.extend_from_slice(payload);
                    v
                }),
                "bytes_hex": hex(bytes), "expect": expect,
            })
        };
    let mut frames: Vec<Value> = Vec::new();
    // Directed: every opcode, empty and small payloads; plus an unknown opcode.
    for &op in &opcodes {
        for p in [vec![], vec![0x00], vec![0xFF, 0x00, 0xA5, 0x5A]] {
            let f = request_frame(op, &p);
            frames.push(frame_json("good", "req", op, &p, &f, "parse_ok"));
        }
    }
    let unk = request_frame(0x7F, &[1, 2]);
    frames.push(frame_json(
        "good",
        "req",
        0x7F,
        &[1, 2],
        &unk,
        "parse_ok_unknown_opcode_nack_opcode",
    ));
    for &st in &[0u8, 1, 2, 3, 4] {
        let p = vec![st; 3];
        let f = response_frame(st, &p);
        frames.push(frame_json("good", "rsp", st, &p, &f, "parse_ok"));
    }
    // Random.
    for _ in 0..n {
        let op = *r.pick(&opcodes);
        let p = rand_payload(&mut r, max_payload);
        let good = request_frame(op, &p);
        match r.below(4) {
            0 => {
                let mut bad = good.clone();
                let pos = r.range_usize(1, bad.len() - 1); // any byte after sync
                bad[pos] ^= 1 << r.below(8);
                // A flipped len byte can also make the frame look longer -> may time out.
                let kind = if pos == 2 || pos == 3 {
                    "bad_len_or_crc"
                } else {
                    "bad_crc"
                };
                let expect = if pos == 2 || pos == 3 {
                    "nack_crc_or_timeout"
                } else {
                    "nack_crc"
                };
                frames.push(frame_json(kind, "req", op, &p, &bad, expect));
            }
            1 => {
                let cut = r.range_usize(1, good.len() - 1);
                frames.push(frame_json(
                    "truncated",
                    "req",
                    op,
                    &p,
                    &good[..cut],
                    "timeout_reset",
                ));
            }
            _ => frames.push(frame_json("good", "req", op, &p, &good, "parse_ok")),
        }
    }
    // Back-to-back: 8 good frames concatenated without idle time.
    let mut b2b = Vec::new();
    let mut parts = Vec::new();
    for _ in 0..8 {
        let op = *r.pick(&opcodes);
        let p = rand_payload(&mut r, 64);
        let f = request_frame(op, &p);
        parts.push(json!({"code": op, "payload_hex": hex(&p)}));
        b2b.extend_from_slice(&f);
    }
    let doc = json!({
        "meta": meta.json(seed, "unit/packet"),
        "description": "SPEC §3.8 frames: sync | code | len u16 LE | payload | crc8. CRC-8 poly 0x07, init 0, \
    over every byte after sync through the payload. expect: parse_ok, nack_crc, timeout_reset (truncated: \
    parser must reset after RX_TIMEOUT idle cycles), nack_crc_or_timeout (corrupted len byte).",
        "crc8_known_answers": [
            {"data_hex": hex(b"123456789"), "crc": crc8(b"123456789")},
            {"data_hex": "", "crc": 0},
            {"data_hex": "01", "crc": crc8(&[1])},
        ],
        "frames": frames,
        "back_to_back": {"bytes_hex": hex(&b2b), "frames": parts},
    });
    vec![OutFile::json_compact("unit/packet.json", &doc)]
}
