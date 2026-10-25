//! Codegen tests: the committed spec is valid, generated files are current and
//! deterministic, constants agree with the spec, and validation rejects
//! inconsistent values loudly.

use splitsys_spec as ss;
use splitsys_spec::model::{rs_out_path, sv_out_path, GENERATED_HEADER};
use splitsys_spec::{Spec, SpecError};

fn spec_text() -> String {
    std::fs::read_to_string(ss::model::default_spec_path()).expect("read spec/spec.toml")
}

fn table() -> toml::Table {
    spec_text().parse::<toml::Table>().expect("spec.toml parses as TOML")
}

/// Apply a mutation to the committed spec and return the validation errors.
fn errors_after(mutate: impl FnOnce(&mut toml::Table)) -> Vec<String> {
    let mut t = table();
    mutate(&mut t);
    let text = toml::to_string(&t).expect("serialize");
    match Spec::from_toml_str(&text) {
        Ok(_) => panic!("mutated spec was accepted but should have been rejected"),
        Err(SpecError::Invalid(v)) => v,
        Err(e) => vec![e.to_string()],
    }
}

fn tbl<'a>(t: &'a mut toml::Table, path: &[&str]) -> &'a mut toml::Table {
    let mut cur = t;
    for p in path {
        cur = cur.get_mut(*p).and_then(|v| v.as_table_mut()).unwrap_or_else(|| panic!("no table {p}"));
    }
    cur
}

fn set(t: &mut toml::Table, path: &[&str], key: &str, v: impl Into<toml::Value>) {
    tbl(t, path).insert(key.to_string(), v.into());
}

fn assert_rejects(errs: &[String], needle: &str) {
    assert!(
        errs.iter().any(|e| e.contains(needle)),
        "expected an error containing {needle:?}, got: {errs:#?}"
    );
}

#[test]
fn committed_spec_is_valid() {
    Spec::load_default().unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn generated_files_are_up_to_date() {
    let s = Spec::load_default().unwrap();
    let sv = std::fs::read_to_string(sv_out_path()).expect("rtl/gen/splitsys_pkg.sv exists");
    let rs = std::fs::read_to_string(rs_out_path()).expect("generated.rs exists");
    assert_eq!(sv, s.render_sv(), "rtl/gen/splitsys_pkg.sv is stale: run `cargo run -p splitsys-spec --bin gen`");
    assert_eq!(rs, s.render_rust(), "generated.rs is stale: run `cargo run -p splitsys-spec --bin gen`");
}

#[test]
fn rendering_is_deterministic_and_headed() {
    let a = Spec::load_default().unwrap();
    let b = Spec::from_toml_str(&spec_text()).unwrap();
    assert_eq!(a.render_sv(), b.render_sv());
    assert_eq!(a.render_rust(), b.render_rust());
    for text in [a.render_sv(), a.render_rust()] {
        assert!(text.lines().next().unwrap().contains(GENERATED_HEADER));
        assert!(!text.contains('\r'), "generated files use LF only");
        assert!(text.ends_with('\n'));
    }
}

#[test]
fn consts_match_spec() {
    let s = Spec::load_default().unwrap();
    assert_eq!(ss::ROWS as i64, s.array.rows);
    assert_eq!(ss::COLS as i64, s.array.cols);
    assert_eq!(ss::QUAD as i64, s.array.quad);
    assert_eq!(ss::L_PE as i64, s.lat("L_PE").value);
    assert_eq!(ss::L_SETUP as i64, s.lat("L_SETUP").value);
    assert_eq!(ss::WBUF_WORDS as i64, s.buf("WBUF").words);
    assert_eq!(ss::RQ_ENTRIES as i64, s.buf("RQPARAM").words);
    assert_eq!(ss::DESC_OFF_RQ_BASE as i64, s.desc_field("rq_base").offset);
    assert_eq!(ss::N_COUNTERS, s.counters.order.len());
    for (i, n) in s.counters.order.iter().enumerate() {
        assert_eq!(ss::COUNTER_NAMES[i], n);
    }
    assert_eq!(ss::LATENCIES.len(), 7);
    // D-016/D-017: all seven latencies confirmed by RTL (3786ad1, 283-job smoke, counters == cycle model).
    for l in ss::LATENCIES {
        assert!(!l.provisional, "{} should be confirmed", l.name);
        assert_eq!(l.provisional, s.lat(l.name).provisional);
    }
    assert_eq!(
        [ss::L_PE, ss::L_RD, ss::L_SETUP, ss::L_RQ, ss::L_WB, ss::L_ACCEPT, ss::L_DONE],
        [3, 2, 16, 7, 1, 1, 1]
    );
    assert_eq!(ss::OPERAND_PORT_OFFSET, ss::L_RD + 1);
    // FLUSH = (R-1)+(C-1)+L_PE = 17 confirmed by rtl-designer.
    assert_eq!((ss::ROWS as u64 - 1) + (ss::COLS as u64 - 1) + ss::L_PE, 17);
    // Legacy INT32_DRAIN_FACTOR was removed (D-016).
    assert!(!s.render_rust().contains("INT32_DRAIN_FACTOR"));
}

#[test]
fn values_the_other_crates_rely_on() {
    // The coordination contract from the lead's S0 brief.
    assert_eq!((ss::ROWS, ss::COLS, ss::QUAD), (8, 8, 4));
    assert_eq!((ss::MODE_FULL, ss::MODE_STRIP, ss::MODE_INDEP), (0, 1, 2));
    assert_eq!((ss::L_PE, ss::L_RD), (3, 2));
    assert_eq!((ss::ABUF_WORD_BYTES, ss::WBUF_WORD_BYTES, ss::OBUF_WORD_BYTES, ss::RQ_WORD_BYTES), (8, 16, 16, 4));
    assert_eq!((ss::REGION_WBUF, ss::REGION_ABUF, ss::REGION_OBUF, ss::REGION_RQPARAM), (0, 1, 2, 3));
    assert_eq!(ss::DESC_BYTES, 24);
    assert_eq!(
        [
            ss::DESC_OFF_MODE,
            ss::DESC_OFF_FLAGS,
            ss::DESC_OFF_M,
            ss::DESC_OFF_K,
            ss::DESC_OFF_N,
            ss::DESC_OFF_A_BASE,
            ss::DESC_OFF_W_BASE,
            ss::DESC_OFF_OUT_BASE,
            ss::DESC_OFF_RQ_BASE,
            ss::DESC_OFF_RESERVED
        ],
        [0, 1, 2, 4, 6, 8, 12, 16, 20, 22]
    );
    assert_eq!((ss::FLAG_REQUANT_EN, ss::FLAG_OUT_INT32), (0x01, 0x02));
    assert_eq!((ss::M_MIN, ss::M_MAX, ss::K_MIN, ss::K_MAX, ss::N_MIN, ss::N_MAX), (1, 64, 1, 256, 1, 256));
    assert_eq!((ss::RQ_MULT_MIN, ss::RQ_MULT_MAX, ss::RQ_SHIFT_MIN, ss::RQ_SHIFT_MAX), (1, 32767, 1, 31));
    assert_eq!((ss::CLK_HZ, ss::BAUD, ss::OVERSAMPLE), (100_000_000, 1_562_500, 16));
    assert_eq!(ss::RX_TIMEOUT, 1_000_000);
    assert_eq!((ss::SYNC_REQ, ss::SYNC_RSP, ss::MAX_PAYLOAD, ss::CRC8_POLY, ss::CRC8_INIT), (0xA5, 0x5A, 1024, 0x07, 0x00));
    assert_eq!(
        [ss::OP_PING, ss::OP_WRITE_MEM, ss::OP_READ_MEM, ss::OP_RUN_GEMM, ss::OP_POLL, ss::OP_READ_COUNTERS, ss::OP_SELFTEST, ss::OP_SET_DISPLAY],
        [0x01, 0x10, 0x11, 0x20, 0x21, 0x30, 0x40, 0x50]
    );
    assert_eq!([ss::ST_ACK, ss::ST_NACK_CRC, ss::ST_NACK_OPCODE, ss::ST_NACK_RANGE, ss::ST_BUSY], [0, 1, 2, 3, 4]);
    assert_eq!([ss::POLL_IDLE, ss::POLL_BUSY, ss::POLL_DONE, ss::POLL_ERROR], [0, 1, 2, 3]);
    // DEMO.md §4 counter order (mac_useful_q expands to q0..q3).
    assert_eq!(
        ss::COUNTER_NAMES,
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
    assert_eq!((ss::N_COUNTERS, ss::COUNTER_BYTES), (12, 6));
    assert_eq!((ss::CTR_CYCLES_TOTAL, ss::CTR_MAC_USEFUL, ss::CTR_MAC_USEFUL_Q0, ss::CTR_TILES), (0, 6, 7, 11));
    assert_eq!(ss::W_SKEW_BASE_STRIP, [0, 0, 0, 0, 0, 0, 0, 0, 4, 4, 4, 4, 4, 4, 4, 4]);
    assert_eq!(ss::BAUD_DIV, 4); // exact: 100 MHz / (1,562,500 x 16) (D-020)
    assert_eq!(&ss::PING_MAGIC, b"SPLT");
}

#[test]
fn buffers_hold_the_largest_legal_job() {
    // Extents of SPEC §3.5 validation rules at the max legal shape.
    const M_PAD: usize = ss::M_MAX.div_ceil(ss::ROWS) * ss::ROWS;
    const {
        assert!(ss::M_MAX.div_ceil(ss::A_LANES) * ss::K_MAX <= ss::ABUF_WORDS);
        assert!(ss::N_MAX.div_ceil(ss::W_LANES) * ss::K_MAX <= ss::WBUF_WORDS);
        assert!(ss::N_MAX.div_ceil(ss::W_LANES) * M_PAD <= ss::OBUF_WORDS);
        assert!(ss::N_MAX <= ss::RQ_ENTRIES);
        // int32 bypass at max shape does NOT fit: it is range-checked (DECISIONS D-005).
        assert!(ss::N_MAX.div_ceil(ss::W_LANES) * M_PAD * ss::INT32_WORDS_PER_ROW > ss::OBUF_WORDS);
    }
}

#[test]
fn crc8_and_fletcher32_reference_vectors() {
    assert_eq!(ss::crc8(ss::CRC8_POLY, ss::CRC8_INIT, b"123456789"), ss::CRC8_CHECK);
    assert_eq!(ss::CRC8_CHECK, 0xF4);
    assert_eq!(ss::fletcher32(b"abcde"), 0xF04F_C729);
    assert_eq!(ss::fletcher32(b"abcdef"), 0x5650_2D2A);
    assert_eq!(ss::fletcher32(b"abcdefgh"), 0xEBE1_9591);
    assert_eq!(ss::fletcher32(b""), 0);
    // A word of 0xFFFF is congruent to 0 (sums kept in 0..=65534).
    assert_eq!(ss::fletcher32(&[0xFF, 0xFF]), 0);
    // Odd trailing byte is zero-padded.
    assert_eq!(ss::fletcher32(b"a"), ss::fletcher32(b"a\0"));
}

#[test]
fn sv_package_exposes_contract_names() {
    let sv = Spec::load_default().unwrap().render_sv();
    for needle in [
        "package splitsys_pkg;",
        "endpackage : splitsys_pkg",
        "} mode_e;",
        "} opcode_e;",
        "} status_e;",
        "} poll_e;",
        "} region_e;",
        "OP_RUN_GEMM = 8'h20",
        "ST_NACK_RANGE = 8'h03",
        "localparam int unsigned L_PE = 3;",
        "localparam int unsigned STRIP_W_SKEW_BASE = 4;",
        "localparam int unsigned CTR_TILES = 11;",
        "localparam int unsigned DESC_OFF_RQ_BASE = 20;",
    ] {
        assert!(sv.contains(needle), "SV package lacks {needle:?}");
    }
    for bad in ["let ", "class ", "struct packed"] {
        assert!(!sv.contains(bad), "SV package uses Vivado-unfriendly construct {bad:?}");
    }
}

// ------------------------------------------------------------- rejections

#[test]
fn rejects_cols_not_multiple_of_quad() {
    let errs = errors_after(|t| set(t, &["array"], "cols", 6));
    assert_rejects(&errs, "COLS % QUAD != 0");
}

#[test]
fn rejects_non_quadrant_array() {
    let errs = errors_after(|t| {
        set(t, &["array"], "cols", 12);
    });
    assert_rejects(&errs, "2x2 quadrants");
}

#[test]
fn rejects_negative_latency() {
    let errs = errors_after(|t| set(t, &["latency", "L_RQ"], "value", -1));
    assert_rejects(&errs, "latency.L_RQ = -1 is negative");
}

#[test]
fn rejects_zero_pe_latency() {
    let errs = errors_after(|t| set(t, &["latency", "L_PE"], "value", 0));
    assert_rejects(&errs, "latency.L_PE must be >= 1");
}

#[test]
fn rejects_missing_and_unknown_latency() {
    let errs = errors_after(|t| {
        let l = tbl(t, &["latency"]);
        let v = l.remove("L_WB").unwrap();
        l.insert("L_BOGUS".into(), v);
    });
    assert_rejects(&errs, "latency.L_WB is missing");
    assert_rejects(&errs, "latency.L_BOGUS is not a known latency");
}

#[test]
fn rejects_wrong_wbuf_width() {
    let errs = errors_after(|t| set(t, &["buffers", "WBUF"], "word_bytes", 8));
    assert_rejects(&errs, "WBUF width must be");
}

#[test]
fn rejects_wrong_abuf_width() {
    let errs = errors_after(|t| set(t, &["buffers", "ABUF"], "word_bytes", 16));
    assert_rejects(&errs, "ABUF width must be");
}

#[test]
fn rejects_non_power_of_two_depth() {
    let errs = errors_after(|t| set(t, &["buffers", "OBUF"], "words", 1000));
    assert_rejects(&errs, "power of two");
}

#[test]
fn rejects_buffer_too_small_for_max_job() {
    let errs = errors_after(|t| set(t, &["buffers", "WBUF"], "words", 2048));
    assert_rejects(&errs, "WBUF words 2048 < ceil(N_MAX/16)*K_MAX");
}

#[test]
fn rejects_wrong_region_id() {
    let errs = errors_after(|t| set(t, &["buffers", "OBUF"], "region", 3));
    assert_rejects(&errs, "buffers.OBUF.region must be 2");
}

#[test]
fn rejects_descriptor_gap_and_misalignment() {
    let errs = errors_after(|t| {
        let fields = t["descriptor"]["fields"].as_array().unwrap().clone();
        let mut fields = fields;
        fields[5].as_table_mut().unwrap().insert("offset".into(), 9.into()); // a_base at 9
        tbl(t, &["descriptor"]).insert("fields".into(), toml::Value::Array(fields));
    });
    assert_rejects(&errs, "a_base offset 9 is not naturally aligned");
    assert_rejects(&errs, "must be 8 (contiguous)");
}

#[test]
fn rejects_descriptor_size_mismatch() {
    let errs = errors_after(|t| set(t, &["descriptor"], "bytes", 32));
    assert_rejects(&errs, "descriptor fields cover 24 bytes");
}

#[test]
fn rejects_k_max_that_overflows_int32() {
    let errs = errors_after(|t| {
        set(t, &["descriptor"], "k_max", 1 << 17);
        set(t, &["buffers", "WBUF"], "words", 1i64 << 22);
        set(t, &["buffers", "ABUF"], "words", 1i64 << 21);
    });
    assert_rejects(&errs, "int32 accumulator overflow");
}

#[test]
fn rejects_requant_ranges_outside_contract() {
    let errs = errors_after(|t| {
        set(t, &["requant"], "shift_max", 32);
        set(t, &["requant"], "mult_max", 32768);
    });
    assert_rejects(&errs, "shift range 1..32");
    assert_rejects(&errs, "mult range 1..32768");
}

#[test]
fn rejects_duplicate_opcode_and_status() {
    let errs = errors_after(|t| {
        set(t, &["opcodes"], "POLL", 0x20);
        set(t, &["status"], "BUSY", 3);
    });
    assert_rejects(&errs, "opcodes values are not unique");
    assert_rejects(&errs, "status values are not unique");
}

#[test]
fn rejects_unknown_opcode_name_and_missing_one() {
    let errs = errors_after(|t| {
        let o = tbl(t, &["opcodes"]);
        o.remove("SELFTEST");
        o.insert("FROB".into(), 0x77.into());
    });
    assert_rejects(&errs, "opcodes.SELFTEST is missing");
    assert_rejects(&errs, "opcodes.FROB is not a known name");
}

#[test]
fn rejects_wrong_crc_check_value() {
    let errs = errors_after(|t| set(t, &["protocol"], "crc8_init", 0xFF));
    assert_rejects(&errs, "crc8_check");
}

#[test]
fn rejects_wrong_fletcher_check_value() {
    let errs = errors_after(|t| set(t, &["selftest"], "fletcher32_check_abcde", 0x1234_5678));
    assert_rejects(&errs, "fletcher32_check_abcde");
}

#[test]
fn rejects_baud_with_large_divider_error() {
    // 921600 baud at 100 MHz x16 oversampling: divider 7 is -3.1 % off.
    let errs = errors_after(|t| {
        set(t, &["uart"], "baud", 921_600);
    });
    assert_rejects(&errs, "baud error");
}

#[test]
fn rejects_rx_timeout_shorter_than_two_bytes() {
    // 2 byte times = 2 x 10 x 64 = 1280 cycles at 1,562,500 baud.
    let errs = errors_after(|t| set(t, &["uart"], "rx_timeout", 1000));
    assert_rejects(&errs, "rx_timeout 1000 must be >= 2 byte times");
}

#[test]
fn allowed_bauds_are_accepted() {
    // D-020: 115,200 (KG2 fallback, divider 54, 0.47 % error) and the exact
    // dividers 1,562,500 (4) and 3,125,000 (2) must all pass validation, with
    // the committed RX_TIMEOUT unchanged.
    for baud in [115_200i64, 1_562_500, 3_125_000] {
        let mut t = table();
        set(&mut t, &["uart"], "baud", baud);
        let text = toml::to_string(&t).expect("serialize");
        let s = Spec::from_toml_str(&text).unwrap_or_else(|e| panic!("baud {baud}: {e}"));
        let want = match baud {
            115_200 => 54,
            1_562_500 => 4,
            _ => 2,
        };
        assert_eq!(s.baud_div(), want, "baud {baud}");
    }
}

#[test]
fn rejects_duplicate_or_missing_counter() {
    let errs = errors_after(|t| {
        let mut order = t["counters"]["order"].as_array().unwrap().clone();
        order[11] = "cycles_total".into(); // replace tiles with a duplicate
        tbl(t, &["counters"]).insert("order".into(), toml::Value::Array(order));
    });
    assert_rejects(&errs, "counter cycles_total listed twice");
    assert_rejects(&errs, "required counter tiles missing");
}

#[test]
fn rejects_wrong_mode_encoding() {
    let errs = errors_after(|t| {
        set(t, &["modes"], "STRIP", 2);
        set(t, &["modes"], "INDEP", 1);
    });
    assert_rejects(&errs, "modes.STRIP must be 1");
}

#[test]
fn rejects_wrong_drain_lengths() {
    let errs = errors_after(|t| {
        set(t, &["drain"], "t_drain_strip", 8);
        set(t, &["drain"], "t_drain_full_int32", 32);
    });
    assert_rejects(&errs, "t_drain_strip 8 must equal ROWS/2");
    assert_rejects(&errs, "t_drain_full_int32 32 must equal");
}

#[test]
fn rejects_unknown_keys() {
    let mut t = table();
    set(&mut t, &["array"], "colz", 8);
    let text = toml::to_string(&t).unwrap();
    match Spec::from_toml_str(&text) {
        Err(SpecError::Parse(msg)) => assert!(msg.contains("colz"), "{msg}"),
        other => panic!("expected parse error for unknown key, got {other:?}"),
    }
}

#[test]
fn reports_all_problems_at_once() {
    let errs = errors_after(|t| {
        set(t, &["array"], "cols", 6);
        set(t, &["latency", "L_RD"], "value", -2);
        set(t, &["buffers", "WBUF"], "word_bytes", 8);
    });
    assert!(errs.len() >= 3, "{errs:#?}");
}

#[test]
fn drain_lengths_per_mode_and_format() {
    // D-004: FULL int32 drains 8 rows x 2 writes, STRIP int32 4 rows x 4 writes.
    assert_eq!((ss::T_DRAIN_FULL, ss::T_DRAIN_STRIP), (8, 4));
    assert_eq!((ss::T_DRAIN_FULL_INT32, ss::T_DRAIN_STRIP_INT32), (16, 16));
    assert_eq!((ss::INT32_WRITES_PER_ROW_FULL, ss::INT32_WRITES_PER_ROW_STRIP), (2, 4));
    assert_eq!(ss::T_DRAIN_FULL_INT32, ss::FULL_TILE_M as u64 * ss::INT32_WRITES_PER_ROW_FULL);
    assert_eq!(ss::T_DRAIN_STRIP_INT32, ss::STRIP_TILE_M as u64 * ss::INT32_WRITES_PER_ROW_STRIP);
    assert_eq!(ss::INT32_WORDS_PER_ROW, 4);
    assert_eq!((ss::FLAGS_INT8, ss::FLAGS_INT32), (ss::FLAG_REQUANT_EN, ss::FLAG_OUT_INT32));
}

#[test]
fn bench_shapes_match_spec_section_1_1() {
    let got: Vec<(&str, usize, usize)> = ss::BENCH_SHAPES.iter().map(|s| (s.name, s.k, s.n)).collect();
    assert_eq!(got, [("qkv", 64, 192), ("attn_out", 64, 64), ("ffn_up", 64, 256), ("ffn_down", 256, 64)]);
    assert_eq!(ss::BENCH_M_SWEEP, [1, 2, 4, 8, 16, 32, 64]);
    assert_eq!((ss::BENCH_M_DECODE, ss::BENCH_M_PREFILL), (1, 64));
}

#[test]
fn rejects_bench_shape_out_of_range_or_duplicate() {
    let errs = errors_after(|t| {
        let mut shapes = t["bench"]["shapes"].as_array().unwrap().clone();
        shapes[0].as_table_mut().unwrap().insert("k".into(), 300.into());
        shapes[1].as_table_mut().unwrap().insert("name".into(), "ffn_up".into());
        tbl(t, &["bench"]).insert("shapes".into(), toml::Value::Array(shapes));
        set(t, &["bench"], "m_prefill", 128);
    });
    assert_rejects(&errs, "bench shape qkv (K=300");
    assert_rejects(&errs, "\"ffn_up\" must be a unique");
    assert_rejects(&errs, "bench M 128 outside");
}

/// Scalar constant names emitted in SV (localparams and enum members).
fn sv_names(sv: &str) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for line in sv.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("localparam ") {
            // localparam <type...> NAME = value;
            let lhs = rest.split('=').next().unwrap().trim();
            out.insert(lhs.rsplit(' ').next().unwrap().to_string());
        } else if t.contains(" = 8'h") && !t.starts_with("//") {
            out.insert(t.split(' ').next().unwrap().to_string());
        }
    }
    out
}

/// Constant names emitted in Rust (`pub const NAME:`).
fn rs_names(rs: &str) -> std::collections::BTreeSet<String> {
    rs.lines()
        .filter_map(|l| l.strip_prefix("pub const "))
        .map(|r| r.split(':').next().unwrap().trim().to_string())
        .collect()
}

#[test]
fn sv_and_rust_names_match() {
    // D-015/D-018: identical names in both languages; documented Rust-only extras.
    const RUST_ONLY: [&str; 11] = [
        "SPEC_VERSION",
        "PING_MAGIC",
        "LATENCIES",
        "REGIONS",
        "OPCODES",
        "STATUSES",
        "COUNTER_NAMES",
        "BENCH_SHAPES",
        "BENCH_M_SWEEP",
        "BENCH_M_DECODE",
        "BENCH_M_PREFILL",
    ];
    let s = Spec::load_default().unwrap();
    let sv = sv_names(&s.render_sv());
    let rs = rs_names(&s.render_rust());
    assert!(sv.len() > 100, "SV name scan found only {}", sv.len());
    let sv_only: Vec<&String> = sv.difference(&rs).collect();
    assert!(sv_only.is_empty(), "SV names missing from Rust: {sv_only:?}");
    let rs_only: Vec<&String> = rs.difference(&sv).filter(|n| !RUST_ONLY.contains(&n.as_str())).collect();
    assert!(rs_only.is_empty(), "Rust names missing from SV (not in the documented Rust-only list): {rs_only:?}");
    for n in RUST_ONLY {
        assert!(rs.contains(n), "documented Rust-only name {n} no longer exists; update D-018");
    }
}

#[test]
fn fletcher_constants_match_reference() {
    assert_eq!(ss::FLETCHER32_CHECK_ABCDE, ss::fletcher32(b"abcde"));
    assert_eq!(ss::FLETCHER32_CHECK_ABCDEF, ss::fletcher32(b"abcdef"));
    assert_eq!(ss::FLETCHER32_CHECK_ABCDEFGH, ss::fletcher32(b"abcdefgh"));
    assert_eq!(ss::FLETCHER32_CHECK_ABCDEFGH, 0xEBE1_9591);
    assert_eq!(ss::PING_MAGIC_LE.to_le_bytes(), ss::PING_MAGIC);
    assert_eq!((ss::WBUF_AW, ss::ABUF_AW, ss::OBUF_AW, ss::RQ_AW), (12, 11, 10, 8));
}
