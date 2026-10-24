//! Benchmark suite (SPEC §1.1): qkv, attn_out, ffn_up, ffn_down at M ∈ {1..64}.
//!
//! Values are synthetic but realistically scaled, from a fixed seed: weights are
//! per-output-column symmetric int8 (column max |w| = 127), activations per-tensor
//! symmetric int8 with a few outliers, and requant params calibrated per column on the
//! M = 64 activations so that the column's largest |acc| maps to about 127.
//! Smaller M use the first M rows of the M = 64 activation matrix.

use crate::jobs::job_json;
use crate::{hex, Meta, OutFile};
use serde_json::json;
use splitsys_golden::consts::{MODE_FULL, MODE_STRIP};
use splitsys_golden::rng::Rng;
use splitsys_golden::{choose_rq_param, gemm_i32, Bases, Job, MatI8};

pub const BENCH_SEED: u64 = 0xBE7C_2026;
/// Calibration / largest M: the prefill M from spec.toml `[bench]`.
const M_CAL: usize = splitsys_spec::BENCH_M_PREFILL;

/// Round-to-nearest signed division (ties away from zero), integer only.
fn round_div(x: i64, d: i64) -> i64 {
    assert!(d > 0);
    if x >= 0 {
        (x + d / 2) / d
    } else {
        -((-x + d / 2) / d)
    }
}

/// Symmetric int8 quantization of an integer "real-valued" matrix: per column when
/// `per_col`, else per tensor. Max |value| maps to 127.
fn quantize(raw: &[Vec<i64>], per_col: bool) -> MatI8 {
    let rows = raw.len();
    let cols = raw[0].len();
    let tensor_max = raw
        .iter()
        .flatten()
        .map(|v| v.abs())
        .max()
        .unwrap_or(1)
        .max(1);
    let col_max: Vec<i64> = (0..cols)
        .map(|j| raw.iter().map(|r| r[j].abs()).max().unwrap_or(1).max(1))
        .collect();
    MatI8::from_fn(rows, cols, |i, j| {
        let m = if per_col { col_max[j] } else { tensor_max };
        round_div(raw[i][j] * 127, m).clamp(-127, 127) as i8
    })
}

pub fn bench(meta: &Meta) -> Vec<OutFile> {
    let mut r = Rng::new(BENCH_SEED);
    let mut layers = Vec::new();
    let mut entries = Vec::new();
    for shape in splitsys_spec::BENCH_SHAPES {
        let (name, k, n) = (shape.name, shape.k, shape.n);
        // Weights ~ N(0, 1000) per element, with per-column scale variation.
        let col_scale: Vec<i64> = (0..n).map(|_| r.range_i64(500, 1500)).collect();
        let w_raw: Vec<Vec<i64>> = (0..k)
            .map(|_| (0..n).map(|j| r.normalish(col_scale[j])).collect())
            .collect();
        let w = quantize(&w_raw, true);
        // Activations ~ N(0, 1000) with ~1% outliers at 6x.
        let a_raw: Vec<Vec<i64>> = (0..M_CAL)
            .map(|_| {
                (0..k)
                    .map(|_| {
                        let v = r.normalish(1000);
                        if r.chance(1, 100) {
                            v * 6
                        } else {
                            v
                        }
                    })
                    .collect()
            })
            .collect();
        let a64 = quantize(&a_raw, false);
        let acc64 = gemm_i32(&a64, &w);
        let rq: Vec<(u16, u8)> = (0..n)
            .map(|j| {
                let max_abs = (0..M_CAL)
                    .map(|i| acc64.get(i, j).unsigned_abs())
                    .max()
                    .unwrap_or(0);
                choose_rq_param(u64::from(max_abs), 127)
            })
            .collect();
        layers.push(json!({
            "name": name, "K": k, "N": n,
            "w_rowmajor_hex": hex(&w.data().iter().map(|&v| v as u8).collect::<Vec<_>>()),
            "a64_rowmajor_hex": hex(&a64.data().iter().map(|&v| v as u8).collect::<Vec<_>>()),
            "rq": rq.iter().map(|(m, s)| json!([m, s])).collect::<Vec<_>>(),
        }));
        for m in splitsys_spec::BENCH_M_SWEEP {
            let a = a64.top_rows(m);
            let full = Job::new(MODE_FULL, a, w.clone(), Some(rq.clone()), Bases::default())
                .expect("bench job valid");
            let strip = full.with_mode(MODE_STRIP);
            let c = full.c.as_ref().expect("requant");
            entries.push(json!({
                "layer": name, "M": m, "K": k, "N": n, "macs": m * k * n,
                "c_rowmajor_hex": hex(&c.data().iter().map(|&v| v as u8).collect::<Vec<_>>()),
                "desc_full_hex": hex(&full.desc.to_bytes()),
                "desc_strip_hex": hex(&strip.desc.to_bytes()),
                "job": job_json(&format!("bench_{name}_m{m}"), &full),
            }));
        }
    }
    let doc = json!({
        "meta": meta.json(BENCH_SEED, "bench"),
        "description": "SPEC §1.1 benchmark suite. Each entry's `job` is the FULL-mode job (load list, descriptor, \
    expected OBUF image, compare_ranges); for STRIP use desc_strip_hex (same images, same expected bytes, same compare_ranges). Bases are 0; load each job before running it.",
        "ms": splitsys_spec::BENCH_M_SWEEP,
        "layers": layers,
        "entries": entries,
    });
    vec![OutFile::json_compact("bench/bench_suite.json", &doc)]
}
