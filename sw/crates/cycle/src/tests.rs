//! Unit and property tests for the cycle model.
//!
//! Exact-number tests use an explicit, deliberately "odd" [`Latencies`]
//! (`LAT`) so they test the model's structure, not today's provisional
//! spec values; a separate group runs against `Latencies::default()`.

use super::*;

/// Distinct, nonzero values so a swapped latency shows up in a sum.
const LAT: Latencies = Latencies {
    l_pe: 3,
    l_rd: 2,
    l_setup: 5,
    l_rq: 7,
    l_wb: 11,
    l_accept: 13,
    l_done: 17,
};

const RQ: u8 = spec::FLAGS_INT8;

fn d(mode: Mode, flags: u8, m: u64, k: u64, n: u64) -> Descriptor {
    Descriptor::new(mode, flags, m, k, n)
}

/// Assert walk == closed form == predict, invariants hold; return counters.
fn check(desc: &Descriptor, lat: &Latencies) -> Counters {
    let w = phase_walk(desc, lat);
    let c = closed_form(desc, lat);
    assert_eq!(w, c, "walk vs closed form for {desc:?} {lat:?}");
    w.check_invariants(desc, lat).unwrap();
    assert_eq!(predict(desc, lat).unwrap(), w);
    w
}

fn flush(lat: &Latencies) -> u64 {
    (spec::ROWS - 1) + (spec::COLS - 1) + lat.l_pe
}

#[test]
fn single_tile_full() {
    let k = 16;
    let c = check(&d(Mode::Full, RQ, 8, k, 8), &LAT);
    assert_eq!(c.tiles, 1);
    assert_eq!(c.cycles_setup, LAT.l_setup);
    assert_eq!(c.cycles_stream, LAT.l_rd + k);
    assert_eq!(c.cycles_flush, 14 + LAT.l_pe);
    assert_eq!(c.cycles_drain, 8 + LAT.l_rq + LAT.l_wb);
    assert_eq!(c.cycles_stall, 0);
    assert_eq!(
        c.cycles_total,
        13 + (5 + (2 + 16) + (14 + 3) + 8 + 7 + 11) + 17
    );
    assert_eq!(c.mac_useful, 8 * 8 * k);
    assert_eq!(c.mac_useful_q, [16 * k; 4]);
    assert!((c.utilization() - (1024.0 / (64.0 * c.cycles_total as f64))).abs() < 1e-12);
}

#[test]
fn single_tile_strip() {
    let k = 16;
    let c = check(&d(Mode::Strip, RQ, 4, k, 16), &LAT);
    assert_eq!(c.tiles, 1);
    assert_eq!(c.cycles_stream, LAT.l_rd + k);
    assert_eq!(c.cycles_flush, flush(&LAT));
    assert_eq!(c.cycles_drain, 4 + LAT.l_rq + LAT.l_wb);
    assert_eq!(c.mac_useful, 4 * 16 * k);
    // Each quadrant holds 4 rows x 4 output columns.
    assert_eq!(c.mac_useful_q, [16 * k; 4]);
}

#[test]
fn k_equals_one() {
    for mode in Mode::ALL {
        let c = check(&d(mode, RQ, 8, 1, 16), &LAT);
        assert_eq!(c.mac_useful, 8 * 16);
        assert_eq!(c.cycles_stream, c.tiles * (LAT.l_rd + 1));
    }
    let c = check(&d(Mode::Full, RQ, 1, 1, 1), &LAT);
    assert_eq!(c.tiles, 1);
    assert_eq!(c.mac_useful_q, [1, 0, 0, 0]);
    let c = check(&d(Mode::Strip, RQ, 1, 1, 1), &LAT);
    assert_eq!(c.mac_useful_q, [1, 0, 0, 0]);
}

#[test]
fn edge_tiles_full() {
    // M = 5, N = 11, K = 3: rows {0..4} lo, {4} hi; cols n%8<4: 0..4,8..11 -> 7; else 4.
    let c = check(&d(Mode::Full, RQ, 5, 3, 11), &LAT);
    assert_eq!(c.tiles, 2);
    assert_eq!(c.mac_useful_q, [3 * 4 * 7, 3 * 4 * 4, 3 * 7, 3 * 4]);
    assert_eq!(c.mac_useful, 5 * 11 * 3);
}

#[test]
fn edge_tiles_strip() {
    // M = 5, N = 11, K = 3: n%16 groups of 4 -> [4, 4, 3, 0], every row in both halves.
    let c = check(&d(Mode::Strip, RQ, 5, 3, 11), &LAT);
    assert_eq!(c.tiles, 2);
    assert_eq!(c.mac_useful_q, [3 * 5 * 4, 3 * 5 * 4, 3 * 5 * 3, 0]);
}

#[test]
fn strip_lower_half_holds_columns_8_to_15() {
    // N = 9: only output column 8 is in the second slab -> PE(4+i, 0) -> Q2.
    let c = check(&d(Mode::Strip, RQ, 4, 2, 9), &LAT);
    assert_eq!(c.mac_useful_q, [2 * 4 * 4, 2 * 4 * 4, 2 * 4, 0]);
    // FULL with M = 4 never touches the lower quadrants.
    let c = check(&d(Mode::Full, RQ, 4, 2, 64), &LAT);
    assert_eq!(c.mac_useful_q[2] + c.mac_useful_q[3], 0);
}

#[test]
fn tile_counts() {
    assert_eq!(check(&d(Mode::Full, RQ, 64, 8, 256), &LAT).tiles, 8 * 32);
    assert_eq!(check(&d(Mode::Strip, RQ, 64, 8, 256), &LAT).tiles, 16 * 16);
    assert_eq!(check(&d(Mode::Full, RQ, 9, 8, 17), &LAT).tiles, 2 * 3);
    assert_eq!(check(&d(Mode::Strip, RQ, 9, 8, 17), &LAT).tiles, 3 * 2);
}

#[test]
fn tile_order_column_block_outer() {
    let tl = timeline(&d(Mode::Full, RQ, 16, 4, 16), &LAT);
    let order: Vec<(u64, u64)> = tl
        .iter()
        .filter(|e| e.phase == Phase::TileSetup)
        .map(|e| (e.nb.unwrap(), e.mb.unwrap()))
        .collect();
    assert_eq!(order, vec![(0, 0), (0, 1), (1, 0), (1, 1)]);
    // Timeline is contiguous and covers cycles_total.
    let mut t = 0;
    for e in &tl {
        assert_eq!(e.start, t);
        t += e.len;
    }
    assert_eq!(
        t,
        closed_form(&d(Mode::Full, RQ, 16, 4, 16), &LAT).cycles_total
    );
}

#[test]
fn int32_bypass_drain() {
    let bypass = spec::FLAGS_INT32;
    // SPEC §3.5 / D-004: FULL 8 rows x 2 writes = 16, STRIP 4 rows x 4 writes = 16.
    for (mode, t8, t32) in [(Mode::Full, 8, 16), (Mode::Strip, 4, 16)] {
        let c8 = check(&d(mode, RQ, 8, 4, 16), &LAT);
        let c32 = check(&d(mode, bypass, 8, 4, 16), &LAT);
        assert_eq!(c8.cycles_drain, c8.tiles * (t8 + LAT.l_rq + LAT.l_wb));
        assert_eq!(c32.cycles_drain, c32.tiles * (t32 + LAT.l_rq + LAT.l_wb));
        assert_eq!(c32.cycles_total - c8.cycles_total, c8.tiles * (t32 - t8));
        assert_eq!(c8.mac_useful, c32.mac_useful);
        assert_eq!(c8.mac_useful_q, c32.mac_useful_q);
    }
    assert_eq!(t_drain(Mode::Full, bypass), 16);
    assert_eq!(t_drain(Mode::Strip, bypass), 16);
    assert_eq!(t_drain(Mode::Full, RQ), 8);
    assert_eq!(t_drain(Mode::Strip, RQ), 4);
}

#[test]
fn illegal_flags_rejected() {
    for flags in [0x00u8, 0x03, 0x04, 0x81, 0xFF] {
        assert_eq!(
            d(Mode::Full, flags, 1, 1, 1).validate(),
            Err(DescError::BadFlags(flags))
        );
        assert!(predict(&d(Mode::Strip, flags, 8, 8, 8), &LAT).is_err());
    }
}

#[test]
fn zero_latencies_still_consistent() {
    let z = Latencies {
        l_pe: 0,
        l_rd: 0,
        l_setup: 0,
        l_rq: 0,
        l_wb: 0,
        l_accept: 0,
        l_done: 0,
    };
    let c = check(&d(Mode::Strip, RQ, 3, 5, 30), &z);
    assert_eq!(c.cycles_total, c.tiles * (5 + 14 + 4));
}

#[test]
fn readback_order_matches_spec() {
    let c = check(&d(Mode::Full, RQ, 3, 4, 5), &LAT);
    let rb = c.readback();
    assert_eq!(rb.len(), 12);
    let names: Vec<&str> = rb.iter().map(|x| x.0).collect();
    assert_eq!(
        names,
        [
            "cycles_total",
            "cycles_setup",
            "cycles_stream",
            "cycles_flush",
            "cycles_drain",
            "cycles_stall",
            "mac_useful",
            "mac_useful_q0",
            "mac_useful_q1",
            "mac_useful_q2",
            "mac_useful_q3",
            "tiles"
        ]
    );
    assert_eq!(rb[0].1, c.cycles_total);
    assert_eq!(rb[11].1, c.tiles);
}

#[test]
fn out_of_range_rejected() {
    assert!(d(Mode::Full, RQ, 0, 1, 1).validate().is_err());
    assert!(d(Mode::Full, RQ, spec::M_MAX + 1, 1, 1).validate().is_err());
    assert!(d(Mode::Full, RQ, 1, spec::K_MAX + 1, 1).validate().is_err());
    assert!(d(Mode::Full, RQ, 1, 1, spec::N_MAX + 1).validate().is_err());
    assert!(predict(&d(Mode::Strip, RQ, 1, 0, 1), &LAT).is_err());
    assert!(Mode::from_code(2).is_none());
}

#[test]
fn m1_ceilings_below_spec_bounds_default_latencies() {
    let lat = Latencies::default();
    for (_, k, n) in spec::bench_shapes() {
        let f = check(&d(Mode::Full, RQ, 1, k, n), &lat);
        let s = check(&d(Mode::Strip, RQ, 1, k, n), &lat);
        assert!(f.utilization() < 0.125, "FULL M=1 util {}", f.utilization());
        assert!(s.utilization() < 0.25, "STRIP M=1 util {}", s.utilization());
        assert!(s.utilization() > f.utilization());
        assert_eq!(mapping_ceiling(&d(Mode::Full, RQ, 1, k, n)), 0.125);
        assert_eq!(mapping_ceiling(&d(Mode::Strip, RQ, 1, k, n)), 0.25);
    }
}

#[test]
fn operand_bandwidth_and_intensity() {
    assert_eq!(Mode::Full.operand_bytes_per_cycle(), 16);
    assert_eq!(Mode::Strip.operand_bytes_per_cycle(), 20);
    // ffn_up M = 1: FULL 32 tiles * 64 * 16 B; STRIP 16 tiles * 64 * 20 B.
    let df = d(Mode::Full, RQ, 1, 64, 256);
    let ds = d(Mode::Strip, RQ, 1, 64, 256);
    let (cf, cs) = (check(&df, &LAT), check(&ds, &LAT));
    assert_eq!(operand_bytes_streamed(&df, &cf), 32 * 64 * 16);
    assert_eq!(operand_bytes_streamed(&ds, &cs), 16 * 64 * 20);
    assert_eq!(arithmetic_intensity(&df, &cf), 0.5);
    assert_eq!(arithmetic_intensity(&ds, &cs), 0.8);
}

#[test]
fn choose_mode_prefers_strip_at_decode_and_full_on_narrow_n() {
    for lat in [LAT, Latencies::default()] {
        assert_eq!(choose_mode(1, 64, 256, RQ, &lat), Mode::Strip);
        // N = 8: STRIP wastes half of every tile's columns.
        assert_eq!(choose_mode(64, 64, 8, RQ, &lat), Mode::Full);
    }
}

#[test]
fn choose_mode_tie_goes_to_full() {
    // M = 8, N = 40, K = 1, only L_SETUP = 1: FULL 5 tiles x 24 = 120 cycles,
    // STRIP 6 tiles x 20 = 120 cycles. Equal totals -> FULL.
    let lat = Latencies {
        l_pe: 0,
        l_rd: 0,
        l_setup: 1,
        l_rq: 0,
        l_wb: 0,
        l_accept: 0,
        l_done: 0,
    };
    let f = check(&d(Mode::Full, RQ, 8, 1, 40), &lat);
    let s = check(&d(Mode::Strip, RQ, 8, 1, 40), &lat);
    assert_eq!((f.tiles, s.tiles), (5, 6));
    assert_eq!(f.cycles_total, 120);
    assert_eq!(s.cycles_total, 120);
    assert_eq!(choose_mode(8, 1, 40, RQ, &lat), Mode::Full);
    // And in general it picks the strictly lower total.
    for m in 1..=16 {
        for n in 1..=48 {
            let f = closed_form(&d(Mode::Full, RQ, m, 4, n), &LAT).cycles_total;
            let s = closed_form(&d(Mode::Strip, RQ, m, 4, n), &LAT).cycles_total;
            let want = if s < f { Mode::Strip } else { Mode::Full };
            assert_eq!(choose_mode(m, 4, n, RQ, &LAT), want);
        }
    }
}

// ---------------------------------------------------------------------------
// Property test: phase walk == closed form, and the three §3.7 invariants.
// ---------------------------------------------------------------------------

/// SplitMix64: tiny, dependency-free, reproducible forever.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in lo..=hi.
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }
}

fn seed() -> u64 {
    std::env::var("SPLITSYS_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x5EED_C7C1E)
}

#[test]
fn property_walk_equals_closed_form() {
    const CASES: usize = 10_000;
    let seed = seed();
    println!("property_walk_equals_closed_form: seed = {seed} (override with SPLITSYS_SEED)");
    let mut rng = Rng(seed);
    for case in 0..CASES {
        let mode = if rng.next() & 1 == 0 {
            Mode::Full
        } else {
            Mode::Strip
        };
        let flags = if rng.next() & 1 == 0 {
            spec::FLAGS_INT8
        } else {
            spec::FLAGS_INT32
        };
        // Bias half the cases toward small shapes (edge tiles dominate there).
        let small = rng.next() & 1 == 0;
        let m = rng.range(1, if small { 17 } else { spec::M_MAX });
        let k = rng.range(1, if small { 9 } else { spec::K_MAX });
        let n = rng.range(1, if small { 33 } else { spec::N_MAX });
        let lat = if rng.next() & 1 == 0 {
            Latencies::default()
        } else {
            Latencies {
                l_pe: rng.range(0, 8),
                l_rd: rng.range(0, 8),
                l_setup: rng.range(0, 32),
                l_rq: rng.range(0, 8),
                l_wb: rng.range(0, 8),
                l_accept: rng.range(0, 8),
                l_done: rng.range(0, 8),
            }
        };
        let desc = d(mode, flags, m, k, n);
        let w = phase_walk(&desc, &lat);
        let c = closed_form(&desc, &lat);
        assert_eq!(w, c, "case {case} seed {seed}: {desc:?} {lat:?}");
        if let Err(e) = w.check_invariants(&desc, &lat) {
            panic!("case {case} seed {seed}: {desc:?} {lat:?}: {e}");
        }
        assert_eq!(w.mac_useful, m * n * k, "case {case} seed {seed}");
        assert!(
            w.utilization() <= mapping_ceiling(&desc) + 1e-12,
            "case {case} seed {seed}"
        );
    }
}

#[test]
fn reports_are_deterministic() {
    let lat = Latencies::default();
    let a = serde_json::to_string(&report::sweep(&lat, true, "test")).unwrap();
    let b = serde_json::to_string(&report::sweep(&lat, true, "test")).unwrap();
    assert_eq!(a, b);
    let r = report::roofline(&lat, true, "test");
    assert_eq!(
        r.points.len(),
        spec::bench_shapes().len() * spec::bench_m_sweep().len() * 2
    );
    for p in &r.points {
        assert!(
            p.macs_per_cycle <= p.roof_macs_per_cycle + 1e-9,
            "{} M={} {}",
            p.name,
            p.m,
            p.mode
        );
    }
    let text = report::explain(&report::sweep(&lat, true, "test"));
    assert!(text.contains("STRIP"));
}

#[test]
fn model_block_schema_matches_demo_md() {
    let lat = Latencies::default();
    let rec = report::model_record("ffn_up", &d(Mode::Strip, RQ, 1, 64, 256), &lat, "abc1234");
    let v = serde_json::to_value(&rec.model).unwrap();
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    let mut want = vec![
        "cycles_total",
        "cycles_setup",
        "cycles_stream",
        "cycles_flush",
        "cycles_drain",
        "cycles_stall",
        "mac_useful",
        "mac_useful_q",
        "tiles",
        "model_git_hash",
    ];
    let mut got = keys.clone();
    got.sort_unstable();
    want.sort_unstable();
    assert_eq!(got, want);
    assert_eq!(v["mac_useful_q"].as_array().unwrap().len(), 4);
    assert_eq!(rec.source, "model");
}

#[test]
fn pe_map_is_a_bijection_onto_the_tile() {
    for mode in Mode::ALL {
        let (tr, tc) = (mode.tile_rows(), mode.tile_cols());
        assert_eq!(tr * tc, spec::PEAK_MACS_PER_CYCLE);
        let mut seen = vec![false; (tr * tc) as usize];
        for r in 0..spec::ROWS {
            for c in 0..spec::COLS {
                let (i, j) = mode.pe_to_output(r, c);
                assert!(i < tr && j < tc, "{mode} PE({r},{c}) -> ({i},{j})");
                let idx = (i * tc + j) as usize;
                assert!(!seen[idx], "{mode}: two PEs map to ({i},{j})");
                seen[idx] = true;
            }
        }
    }
    // SPEC §3.2: PE(4+i, c) holds C[i, 8+c] in STRIP.
    assert_eq!(Mode::Strip.pe_to_output(5, 3), (1, 11));
    assert_eq!(quadrant_of(5, 3), 2);
    assert_eq!(quadrant_of(2, 6), 1);
}
