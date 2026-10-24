//! Property and directed tests for splitsys_golden.
//!
//! Randomized tests use a hand-rolled RNG with an explicit seed. The seed is printed
//! (visible on failure, or with `--nocapture`) and can be overridden with
//! `SPLITSYS_SEED=<u64>`; iteration counts scale with `SPLITSYS_ITERS` (default 1).

// Address formulas are written out literally (mirroring SPEC §1.3), and index loops
// mirror the math; both lints would obscure that.
#![allow(clippy::needless_range_loop, clippy::identity_op)]

use splitsys_golden::consts::*;
use splitsys_golden::pack::*;
use splitsys_golden::rng::Rng;
use splitsys_golden::*;

fn seed(test: &str) -> u64 {
    let base = std::env::var("SPLITSYS_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x5EED_2026u64);
    let mut h = base;
    for b in test.bytes() {
        h = h.rotate_left(5) ^ u64::from(b);
    }
    println!("[{test}] seed = {h} (SPLITSYS_SEED base = {base})");
    h
}

fn iters(n: usize) -> usize {
    let k: usize = std::env::var("SPLITSYS_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    n * k
}

/// Exact rational reference: round-half-up(acc*mult / 2^shift) = floor((2p + 2^s) / 2^(s+1)),
/// computed with i128 Euclidean division (no shifts involved), then clamped.
fn requant_rational(acc: i32, mult: u16, shift: u8) -> i8 {
    let p = i128::from(acc) * i128::from(mult);
    let den = 1i128 << (shift + 1);
    let q = (2 * p + (1i128 << shift)).div_euclid(den);
    q.clamp(-128, 127) as i8
}

fn rand_params(r: &mut Rng) -> (u16, u8) {
    let mult = match r.below(4) {
        0 => *r.pick(&[RQ_MULT_MIN, RQ_MULT_MAX, 2, RQ_MULT_MAX - 1]),
        _ => r.u16_in(RQ_MULT_MIN, RQ_MULT_MAX),
    };
    let shift = match r.below(4) {
        0 => *r.pick(&[RQ_SHIFT_MIN, RQ_SHIFT_MAX]),
        _ => r.u8_in(RQ_SHIFT_MIN, RQ_SHIFT_MAX),
    };
    (mult, shift)
}

fn rand_acc(r: &mut Rng) -> i32 {
    match r.below(5) {
        0 => *r.pick(&[
            i32::MIN,
            i32::MAX,
            0,
            1,
            -1,
            4_194_304,
            -4_194_304,
            4_161_536,
        ]),
        1 => r.range_i64(-4_194_304, 4_194_304) as i32,
        2 => r.range_i64(-1024, 1024) as i32,
        _ => r.i32(),
    }
}

fn rand_mat(r: &mut Rng, rows: usize, cols: usize) -> MatI8 {
    let style = r.below(4);
    MatI8::from_fn(rows, cols, |_, _| match style {
        0 => *r.pick(&[-128i8, 127, -1, 0, 1]),
        1 => r.range_i64(-3, 3) as i8,
        _ => r.i8(),
    })
}

// ---------------------------------------------------------------- requant

#[test]
fn requant_matches_rational_reference() {
    let mut r = Rng::new(seed("requant_rational"));
    for _ in 0..iters(400_000) {
        let (m, s) = rand_params(&mut r);
        let acc = rand_acc(&mut r);
        assert_eq!(
            requant(acc, m, s),
            requant_rational(acc, m, s),
            "acc={acc} mult={m} shift={s}"
        );
    }
}

#[test]
fn requant_matches_f64_at_non_ties() {
    let mut r = Rng::new(seed("requant_f64"));
    let mut checked = 0usize;
    for _ in 0..iters(400_000) {
        let (m, s) = rand_params(&mut r);
        let acc = rand_acc(&mut r);
        if requant_is_tie(acc, m, s) {
            continue;
        }
        // |acc*mult| < 2^46, so p and p / 2^s are exact in f64.
        let x = (f64::from(acc) * f64::from(m)) / f64::from(2u32).powi(i32::from(s));
        let want = x.round().clamp(-128.0, 127.0) as i8; // half-away-from-zero; no ties here
        assert_eq!(
            requant(acc, m, s),
            want,
            "acc={acc} mult={m} shift={s} x={x}"
        );
        checked += 1;
    }
    assert!(checked > iters(300_000));
}

#[test]
fn requant_ties_round_toward_plus_infinity() {
    // Hand-checked cases: value = acc*mult/2^shift.
    let cases: &[(i32, u16, u8, i8)] = &[
        (-1, 1, 1, 0),          // -0.5 -> 0
        (-3, 1, 1, -1),         // -1.5 -> -1
        (1, 1, 1, 1),           // 0.5 -> 1
        (3, 1, 1, 2),           // 1.5 -> 2
        (-2, 3, 2, -1),         // -1.5 -> -1
        (5, 1, 1, 3),           // 2.5 -> 3
        (-5, 1, 1, -2),         // -2.5 -> -2
        (-255, 1, 1, -127),     // -127.5 -> -127
        (-257, 1, 1, -128),     // -128.5 -> -128
        (253, 1, 1, 127),       // 126.5 -> 127
        (255, 1, 1, 127),       // 127.5 -> 128 -> clamp 127
        (-(1 << 30), 1, 31, 0), // -0.5 at shift 31
        ((1 << 30), 1, 31, 1),  // 0.5 at shift 31
    ];
    for &(acc, m, s, y) in cases {
        assert!(requant_is_tie(acc, m, s), "not a tie: {acc} {m} {s}");
        assert_eq!(requant(acc, m, s), y, "acc={acc} mult={m} shift={s}");
        assert_eq!(requant_rational(acc, m, s), y);
    }
    // Random ties: acc*mult = y0*2^s + 2^(s-1) with mult = 1 must give y0 + 1 (clamped).
    let mut r = Rng::new(seed("requant_ties"));
    for _ in 0..iters(100_000) {
        let s = r.u8_in(RQ_SHIFT_MIN, 30);
        let lim = (i64::from(i32::MAX) - (1i64 << (s - 1))) >> s;
        let y0 = r.range_i64(-lim, lim.min(300)).max(-lim);
        let acc = y0 * (1i64 << s) + (1i64 << (s - 1));
        let acc = i32::try_from(acc).expect("tie acc fits i32");
        assert!(requant_is_tie(acc, 1, s));
        let want = (y0 + 1).clamp(-128, 127) as i8;
        assert_eq!(requant(acc, 1, s), want, "acc={acc} shift={s}");
    }
}

#[test]
fn requant_is_monotonic_in_acc() {
    let mut r = Rng::new(seed("requant_monotonic"));
    for _ in 0..iters(2_000) {
        let (m, s) = rand_params(&mut r);
        let start = rand_acc(&mut r).saturating_sub(2000);
        let mut prev = requant(start, m, s);
        for d in 1..=4000i32 {
            let a = start.saturating_add(d);
            let y = requant(a, m, s);
            assert!(y >= prev, "not monotonic at acc={a} mult={m} shift={s}");
            prev = y;
        }
        let (a1, a2) = (rand_acc(&mut r), rand_acc(&mut r));
        let (lo, hi) = (a1.min(a2), a1.max(a2));
        assert!(requant(lo, m, s) <= requant(hi, m, s));
    }
}

#[test]
fn requant_extremes_and_saturation() {
    assert_eq!(requant(i32::MAX, RQ_MULT_MAX, RQ_SHIFT_MIN), 127);
    assert_eq!(requant(i32::MIN, RQ_MULT_MAX, RQ_SHIFT_MIN), -128);
    assert_eq!(requant(i32::MIN, RQ_MULT_MAX, RQ_SHIFT_MAX), -128);
    assert_eq!(requant(i32::MAX, RQ_MULT_MAX, RQ_SHIFT_MAX), 127);
    assert_eq!(requant(4_194_304, RQ_MULT_MAX, RQ_SHIFT_MAX), 64); // 4194304*32767/2^31 = 63.998
    assert_eq!(requant(0, RQ_MULT_MAX, RQ_SHIFT_MIN), 0);
    assert_eq!(requant(0, RQ_MULT_MIN, RQ_SHIFT_MAX), 0);
    assert_eq!(requant(-1, RQ_MULT_MIN, RQ_SHIFT_MAX), 0);
    assert_eq!(requant(254, 1, 1), 127);
    assert_eq!(requant(256, 1, 1), 127);
    assert_eq!(requant(-256, 1, 1), -128);
    assert_eq!(requant(-258, 1, 1), -128);
}

#[test]
#[should_panic]
fn requant_panics_mult_zero() {
    requant(1, 0, 1);
}
#[test]
#[should_panic]
fn requant_panics_mult_too_big() {
    requant(1, RQ_MULT_MAX + 1, 1);
}
#[test]
#[should_panic]
fn requant_panics_shift_zero() {
    requant(1, 1, 0);
}
#[test]
#[should_panic]
fn requant_panics_shift_too_big() {
    requant(1, 1, RQ_SHIFT_MAX + 1);
}

// ---------------------------------------------------------------- GEMM

#[test]
fn gemm_q_equals_naive_reference() {
    let mut r = Rng::new(seed("gemm_q"));
    for _ in 0..iters(300) {
        let m = r.range_usize(1, 20);
        let kmax = if r.chance(1, 8) { K_MAX } else { 40 };
        let k = r.range_usize(1, kmax);
        let n = r.range_usize(1, 40);
        let a = rand_mat(&mut r, m, k);
        let w = rand_mat(&mut r, k, n);
        let params: Vec<(u16, u8)> = (0..n).map(|_| rand_params(&mut r)).collect();
        let got = gemm_q(&a, &w, &params);
        let acc = gemm_i32(&a, &w);
        for i in 0..m {
            for j in 0..n {
                let mut s: i128 = 0;
                for kk in 0..k {
                    s += i128::from(a.get(i, kk)) * i128::from(w.get(kk, j));
                }
                assert_eq!(i128::from(acc.get(i, j)), s, "acc mismatch at ({i},{j})");
                let (pm, ps) = params[j];
                assert_eq!(got.get(i, j), requant_rational(s as i32, pm, ps));
            }
        }
    }
}

#[test]
fn gemm_worst_case_k256() {
    let a = MatI8::from_fn(1, K_MAX, |_, _| -128);
    let w = MatI8::from_fn(K_MAX, 1, |_, _| -128);
    assert_eq!(gemm_i32(&a, &w).get(0, 0), (K_MAX as i32) * 16384);
}

// ---------------------------------------------------------------- packing

#[test]
fn pack_unpack_identity_and_zero_padding() {
    let mut r = Rng::new(seed("pack"));
    for _ in 0..iters(500) {
        let m = r.range_usize(M_MIN, M_MAX);
        let k = r.range_usize(K_MIN, 64);
        let n = r.range_usize(N_MIN, 80);
        let a = rand_mat(&mut r, m, k);
        let w = rand_mat(&mut r, k, n);
        let ai = pack_a_blocks8(&a);
        let wi = pack_w_blocks16(&w);
        assert_eq!(ai.len(), m.div_ceil(8) * k * 8);
        assert_eq!(wi.len(), n.div_ceil(16) * k * 16);
        assert_eq!(unpack_a_blocks8(&ai, m, k), a);
        assert_eq!(unpack_w_blocks16(&wi, k, n), w);
        // Padding bytes are zero: the image of A embedded in a zero M_pad×K matrix.
        let a_pad = MatI8::from_fn(m_pad(m), k, |i, j| if i < m { a.get(i, j) } else { 0 });
        assert_eq!(pack_a_blocks8(&a_pad), ai);
        let n_pad = n.div_ceil(16) * 16;
        let w_pad = MatI8::from_fn(k, n_pad, |i, j| if j < n { w.get(i, j) } else { 0 });
        assert_eq!(pack_w_blocks16(&w_pad), wi);

        let c8 = rand_mat(&mut r, m, n);
        let ci = pack_c_i8_blocks16(&c8);
        assert_eq!(ci.len(), n.div_ceil(16) * m_pad(m) * 16);
        assert_eq!(unpack_c_i8_blocks16(&ci, m, n), c8);
        let nz_logical = c8.data().iter().filter(|&&v| v != 0).count();
        assert_eq!(
            ci.iter().filter(|&&b| b != 0).count(),
            nz_logical,
            "C padding not zero"
        );

        let c32 = MatI32::from_fn(m, n, |_, _| r.i32());
        let c32i = pack_c_i32(&c32);
        assert_eq!(c32i.len(), n.div_ceil(16) * m_pad(m) * 4 * 16);
        assert_eq!(unpack_c_i32(&c32i, m, n), c32);

        let params: Vec<(u16, u8)> = (0..n).map(|_| rand_params(&mut r)).collect();
        let rqi = pack_rqparams(&params);
        assert_eq!(rqi.len(), n * 4); // exactly N entries (SPEC §1.3 v1.1)
        assert_eq!(unpack_rqparams(&rqi, n), params);
    }
}

#[test]
fn pack_addresses_match_spec_formulas() {
    // A: word64 index mb*K + k holds A[8*mb + r, k] in byte lane r.
    let (m, k, n) = (11, 3, 21);
    let a = MatI8::from_fn(m, k, |i, j| (i * 10 + j) as i8);
    let ai = pack_a_blocks8(&a);
    assert_eq!(ai[(k + 2) * 8 + 1], a.get(9, 2) as u8); // mb=1,k=2,r=1
                                                        // W: word128 index nb*K + k holds W[k, 16*nb + c] in lane c.
    let w = MatI8::from_fn(k, n, |i, j| (i * 30 + j) as i8);
    let wi = pack_w_blocks16(&w);
    assert_eq!(wi[(k + 1) * 16 + 4], w.get(1, 20) as u8); // nb=1,k=1,c=4
    assert_eq!(wi[(k + 1) * 16 + 5], 0); // col 21 is padding
                                         // C int8: word128 index nb*M_pad + m holds C[m, 16*nb + c] in lane c.
    let c = MatI8::from_fn(m, n, |i, j| (i * 16 + j) as i8);
    let ci = pack_c_i8_blocks16(&c);
    assert_eq!(ci[(16 + 10) * 16 + 3], c.get(10, 19) as u8); // M_pad=16,nb=1,m=10,c=3
                                                             // C int32: word128 index (nb*M_pad + m)*4 + q holds int32 lanes 4q..4q+3.
    let c32 = MatI32::from_fn(m, n, |i, j| (i as i32) * 1_000_000 - (j as i32) * 7);
    let c32i = pack_c_i32(&c32);
    let word = (16 + 10) * 4 + 3 / 4; // nb=1, m=10, lane 3 -> q=0
    let off = word * 16 + (3 % 4) * 4;
    assert_eq!(&c32i[off..off + 4], &c32.get(10, 19).to_le_bytes());
    let word = (10) * 4 + 13 / 4; // nb=0, m=10, lane 13 -> q=3, lane-in-word 1
    let off = word * 16 + 4;
    assert_eq!(&c32i[off..off + 4], &c32.get(10, 13).to_le_bytes());
    // RQPARAM: {8'h0, shift, mult}.
    assert_eq!(
        pack_rqparams(&[(0x1234, 0x1F)])[..4],
        [0x34, 0x12, 0x1F, 0x00]
    );
}

// ---------------------------------------------------------------- descriptor

#[test]
fn descriptor_bytes_follow_field_map() {
    let d = Descriptor {
        mode: MODE_STRIP,
        flags: FLAG_REQUANT_EN,
        m: 0x0102,
        k: 0x0304,
        n: 0x0506,
        a_base: 0x0A0B_0C0D,
        w_base: 0x1A1B_1C1D,
        out_base: 0x2A2B_2C2D,
        rq_base: 0x3A3B,
        reserved: 0,
    };
    let b = d.to_bytes();
    assert_eq!(b.len(), 24);
    // SPEC §3.5 table, little-endian.
    let want: [u8; 24] = [
        1, 1, 0x02, 0x01, 0x04, 0x03, 0x06, 0x05, 0x0D, 0x0C, 0x0B, 0x0A, 0x1D, 0x1C, 0x1B, 0x1A,
        0x2D, 0x2C, 0x2B, 0x2A, 0x3B, 0x3A, 0, 0,
    ];
    assert_eq!(b, want);
    assert_eq!(Descriptor::from_bytes(&b).unwrap(), d);
    assert!(Descriptor::from_bytes(&b[..23]).is_err());
}

#[test]
fn descriptor_validation() {
    let ok = Descriptor {
        mode: MODE_FULL,
        flags: Descriptor::flags_for(true),
        m: 64,
        k: 256,
        n: 256,
        ..Default::default()
    };
    assert_eq!(ok.validate(), Ok(()));
    assert!(Descriptor {
        mode: MODE_INDEP,
        ..ok
    }
    .validate()
    .is_err());
    assert!(Descriptor { m: 0, ..ok }.validate().is_err());
    assert!(Descriptor { m: 65, ..ok }.validate().is_err());
    assert!(Descriptor { k: 257, ..ok }.validate().is_err());
    assert!(Descriptor { n: 257, ..ok }.validate().is_err());
    assert!(Descriptor { out_base: 1, ..ok }.validate().is_err()); // OBUF exactly full
    assert!(Descriptor { a_base: 1, ..ok }.validate().is_err()); // ABUF exactly full
    for bad_flags in [0u8, 3, 4, 0x81, 0xFF] {
        assert!(Descriptor {
            flags: bad_flags,
            ..ok
        }
        .validate()
        .is_err());
    }
    assert_eq!(
        Descriptor {
            flags: FLAGS_INT8,
            ..ok
        }
        .validate(),
        Ok(())
    );
    // rq_base + N <= RQ_ENTRIES (D-005 check 7), for both flags values.
    let small = Descriptor {
        m: 8,
        k: 8,
        n: 3,
        ..ok
    };
    assert_eq!(
        Descriptor {
            rq_base: (RQ_ENTRIES - 3) as u16,
            ..small
        }
        .validate(),
        Ok(())
    );
    assert!(Descriptor {
        rq_base: (RQ_ENTRIES - 2) as u16,
        ..small
    }
    .validate()
    .is_err());
    let small32 = Descriptor {
        flags: FLAGS_INT32,
        ..small
    };
    assert_eq!(
        Descriptor {
            rq_base: (RQ_ENTRIES - 3) as u16,
            ..small32
        }
        .validate(),
        Ok(())
    );
    assert!(Descriptor {
        rq_base: (RQ_ENTRIES - 2) as u16,
        ..small32
    }
    .validate()
    .is_err());
    // int32 extent: out_base + NB*M_pad*4 <= OBUF_WORDS. M=64, N=64 -> 4*64*4 = 1024.
    let b32 = Descriptor {
        flags: FLAGS_INT32,
        m: 64,
        n: 64,
        ..ok
    };
    assert_eq!(b32.validate(), Ok(()));
    assert!(Descriptor { out_base: 1, ..b32 }.validate().is_err());
    assert!(Descriptor { n: 65, ..b32 }.validate().is_err());
    // Bases are u32: no wraparound.
    assert!(Descriptor {
        a_base: u32::MAX,
        ..small
    }
    .validate()
    .is_err());
    assert!(Descriptor {
        w_base: u32::MAX,
        ..small
    }
    .validate()
    .is_err());
    assert!(Descriptor {
        out_base: u32::MAX,
        ..small
    }
    .validate()
    .is_err());
    assert!(Descriptor { reserved: 1, ..ok }.validate().is_err());
    // int32 bypass of the max shape does not fit OBUF.
    assert!(Descriptor {
        flags: Descriptor::flags_for(false),
        ..ok
    }
    .validate()
    .is_err());
}

// ---------------------------------------------------------------- jobs / modes

#[test]
fn mode_changes_schedule_not_layout_or_expected_c() {
    let mut r = Rng::new(seed("modes"));
    for _ in 0..iters(200) {
        let m = r.range_usize(1, 24);
        let k = r.range_usize(1, 40);
        let n = r.range_usize(1, 40);
        let a = rand_mat(&mut r, m, k);
        let w = rand_mat(&mut r, k, n);
        let rq = if r.chance(3, 4) {
            Some((0..n).map(|_| rand_params(&mut r)).collect::<Vec<_>>())
        } else {
            None
        };
        let full = Job::new(
            MODE_FULL,
            a.clone(),
            w.clone(),
            rq.clone(),
            Bases::default(),
        )
        .unwrap();
        let strip = Job::new(MODE_STRIP, a, w, rq, Bases::default()).unwrap();
        assert_eq!(full.with_mode(MODE_STRIP), strip);
        // Identical images and expected output; only the descriptor mode differs.
        assert_eq!(full.a_image, strip.a_image);
        assert_eq!(full.w_image, strip.w_image);
        assert_eq!(full.rq_image, strip.rq_image);
        assert_eq!(full.obuf_image, strip.obuf_image);
        assert_eq!(full.c, strip.c);
        let (mut fb, mut sb) = (full.desc.to_bytes(), strip.desc.to_bytes());
        assert_ne!(fb[DESC_OFF_MODE], sb[DESC_OFF_MODE]);
        fb[DESC_OFF_MODE] = 0;
        sb[DESC_OFF_MODE] = 0;
        assert_eq!(fb, sb);
        assert_eq!(full.compare, strip.compare);
        // `compare` is exactly the in-bounds bytes of every logical C[m, n].
        let int32 = full.c.is_none();
        let mut want = vec![false; full.obuf_image.len()];
        for i in 0..m {
            for j in 0..n {
                if int32 {
                    let o = c_i32_offset(m, i, j);
                    want[o..o + 4].iter_mut().for_each(|b| *b = true);
                } else {
                    want[c_i8_offset(m, i, j)] = true;
                }
            }
        }
        assert_eq!(full.compare, want);
        for (i, (&cmp, &v)) in full.compare.iter().zip(&full.obuf_image).enumerate() {
            if !cmp {
                assert_eq!(v, 0, "padding byte {i} not shown as 0");
            }
        }
        // Garbage in out-of-bounds bytes must not fail a check; a wrong in-bounds byte must.
        let mut rb: Vec<u8> = full
            .obuf_image
            .iter()
            .zip(&full.compare)
            .map(|(&v, &c)| if c { v } else { 0xA5 })
            .collect();
        assert_eq!(full.check_obuf(&rb), Ok(()));
        let first = full.compare.iter().position(|&c| c).unwrap();
        rb[first] ^= 1;
        assert_eq!(full.check_obuf(&rb), Err(first));
    }
}

#[test]
fn compare_mask_examples() {
    let mk = |m, n| {
        Job::new(
            MODE_FULL,
            MatI8::zeros(m, 4),
            MatI8::zeros(4, n),
            Some(vec![(1, 1); n]),
            Bases::default(),
        )
        .unwrap()
    };
    assert!(mk(8, 16).extent_fully_defined());
    assert!(mk(16, 32).extent_fully_defined());
    assert!(!mk(3, 16).extent_fully_defined()); // rows 3..7 are padding
    assert!(!mk(8, 8).extent_fully_defined()); // lanes 8..15 are padding
    assert_eq!(mk(4, 16).compare_ranges(), vec![(0, 64)]);
    assert_eq!(mk(2, 3).compare_ranges(), vec![(0, 3), (16, 3)]);
}

#[test]
fn choose_rq_param_hits_target() {
    for (max_abs, target) in [(4_194_304u64, 127u64), (1000, 100), (1, 127), (50_000, 90)] {
        let (m, s) = choose_rq_param(max_abs, target);
        assert!(rq_params_valid(m, s));
        let y = requant(max_abs.min(i32::MAX as u64) as i32, m, s);
        assert!(
            (i64::from(y) - target.min(127) as i64).abs() <= 1,
            "{max_abs} {target} -> {y}"
        );
    }
}

#[test]
fn rng_is_deterministic() {
    let a: Vec<u64> = {
        let mut r = Rng::new(42);
        (0..100).map(|_| r.next_u64()).collect()
    };
    let b: Vec<u64> = {
        let mut r = Rng::new(42);
        (0..100).map(|_| r.next_u64()).collect()
    };
    assert_eq!(a, b);
    let mut r = Rng::new(7);
    for _ in 0..10_000 {
        let v = r.range_i64(-5, 5);
        assert!((-5..=5).contains(&v));
    }
}

#[test]
fn fletcher_and_crc_agree_with_spec_crate() {
    let mut r = Rng::new(seed("checksums"));
    assert_eq!(fletcher32(b"abcde"), splitsys_spec::FLETCHER32_CHECK_ABCDE);
    assert_eq!(
        fletcher32(b"abcdef"),
        splitsys_spec::FLETCHER32_CHECK_ABCDEF
    );
    assert_eq!(crc8(b"123456789"), splitsys_spec::CRC8_CHECK);
    for _ in 0..iters(500) {
        let len = r.range_usize(0, 300);
        let v: Vec<u8> = (0..len).map(|_| r.next_u64() as u8).collect();
        assert_eq!(fletcher32(&v), splitsys_spec::fletcher32(&v));
        assert_eq!(
            crc8(&v),
            splitsys_spec::crc8(splitsys_spec::CRC8_POLY, splitsys_spec::CRC8_INIT, &v)
        );
    }
}
