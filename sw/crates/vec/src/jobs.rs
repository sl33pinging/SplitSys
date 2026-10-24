//! GEMM jobs: JSON encoding, random GEMMs (`vec gemm`), directed cases (`vec directed`).

use crate::{hex, unit, Meta, OutFile};
use serde_json::{json, Value};
use splitsys_golden::consts::*;
use splitsys_golden::pack::{a_words, c_words, w_words};
use splitsys_golden::rng::Rng;
use splitsys_golden::{choose_rq_param, gemm_i32, Bases, Job, MatI8};

pub fn mode_name(mode: u8) -> &'static str {
    match mode {
        MODE_FULL => "full",
        MODE_STRIP => "strip",
        _ => "indep",
    }
}

pub fn parse_mode(s: &str) -> Result<u8, String> {
    match s {
        "full" => Ok(MODE_FULL),
        "strip" => Ok(MODE_STRIP),
        _ => Err(format!("unknown mode '{s}' (full|strip)")),
    }
}

fn region_json(region: u8, name: &str, byte_addr: usize, bytes: &[u8]) -> Value {
    json!({"region": region, "region_name": name, "byte_addr": byte_addr, "len": bytes.len(), "hex": hex(bytes)})
}

/// JSON for one job. `load` lists WRITE_MEM operations (region, byte address, bytes);
/// `expected.obuf_hex` is the OBUF image from `expected.obuf_byte_addr`; compare only
/// `expected.compare_ranges` (offset, len relative to obuf_byte_addr): the in-bounds
/// bytes. Everything else in the C extent is unspecified (SPEC §1.3, D-006).
pub fn job_json(name: &str, job: &Job) -> Value {
    let d = &job.desc;
    let mut load = vec![
        region_json(REGION_ABUF, "ABUF", job.a_byte_addr(), &job.a_image),
        region_json(REGION_WBUF, "WBUF", job.w_byte_addr(), &job.w_image),
    ];
    if !job.rq_image.is_empty() {
        load.push(region_json(
            REGION_RQPARAM,
            "RQPARAM",
            job.rq_byte_addr(),
            &job.rq_image,
        ));
    }
    json!({
        "name": name,
        "mode": mode_name(d.mode),
        "M": d.m, "K": d.k, "N": d.n,
        "requant": job.c.is_some(),
        "desc": {
            "mode": d.mode, "flags": d.flags, "M": d.m, "K": d.k, "N": d.n,
            "a_base": d.a_base, "w_base": d.w_base, "out_base": d.out_base,
            "rq_base": d.rq_base, "reserved": d.reserved,
        },
        "desc_hex": hex(&d.to_bytes()),
        "rq": job.rq.iter().map(|(m, s)| json!([m, s])).collect::<Vec<_>>(),
        "load": load,
        "expected": {
            "obuf_byte_addr": job.out_byte_addr(),
            "obuf_len": job.obuf_image.len(),
            "obuf_hex": hex(&job.obuf_image),
            "compare_ranges": job.compare_ranges(),
            "c": job.c.as_ref().map(|c| c.to_rows()),
            "acc": job.acc.to_rows(),
        },
    })
}

// ------------------------------------------------------------------ random GEMM

fn rand_mat(r: &mut Rng, rows: usize, cols: usize) -> MatI8 {
    let style = r.below(6);
    MatI8::from_fn(rows, cols, |_, _| match style {
        0 => *r.pick(&[-128i8, 127, -1, 0, 1]),
        1 => r.range_i64(-4, 4) as i8,
        2 => {
            if r.chance(1, 4) {
                r.i8()
            } else {
                0
            }
        }
        3 => r.normalish(30).clamp(-128, 127) as i8,
        _ => r.i8(),
    })
}

/// Per-column params. Mostly "realistic" (column max |acc| maps near a target, some
/// columns saturate), sometimes uniform random, sometimes extreme.
pub fn rand_params(r: &mut Rng, a: &MatI8, w: &MatI8) -> Vec<(u16, u8)> {
    let acc = gemm_i32(a, w);
    let style = r.below(4);
    (0..w.cols())
        .map(|j| match style {
            0 => (
                r.u16_in(RQ_MULT_MIN, RQ_MULT_MAX),
                r.u8_in(RQ_SHIFT_MIN, RQ_SHIFT_MAX),
            ),
            1 => (
                *r.pick(&[RQ_MULT_MIN, RQ_MULT_MAX]),
                *r.pick(&[RQ_SHIFT_MIN, RQ_SHIFT_MAX, 8, 16]),
            ),
            _ => {
                let max_abs = (0..acc.rows())
                    .map(|i| acc.get(i, j).unsigned_abs())
                    .max()
                    .unwrap_or(0);
                let target = r.range_i64(40, 250) as u64;
                choose_rq_param(u64::from(max_abs), target)
            }
        })
        .collect()
}

fn rand_bases(r: &mut Rng, m: usize, k: usize, n: usize, int32: bool) -> Bases {
    if r.chance(1, 2) {
        return Bases::default();
    }
    let pick = |r: &mut Rng, used: usize, size: usize| r.range_usize(0, size - used) as u32;
    Bases {
        a: pick(r, a_words(m, k), ABUF_WORDS),
        w: pick(r, w_words(k, n), WBUF_WORDS),
        out: pick(r, c_words(m, n, int32), OBUF_WORDS),
        rq: pick(r, n, RQ_ENTRIES) as u16,
    }
}

/// `vec gemm`: `n` random jobs in one mode. The first jobs are edge shapes.
pub fn gemm(
    seed: u64,
    n: usize,
    mode: u8,
    max_m: usize,
    max_k: usize,
    max_n: usize,
    meta: &Meta,
) -> Result<Vec<OutFile>, String> {
    for (nm, v, lo, hi) in [
        ("m", max_m, M_MIN, M_MAX),
        ("k", max_k, K_MIN, K_MAX),
        ("n", max_n, N_MIN, N_MAX),
    ] {
        if !(lo..=hi).contains(&v) {
            return Err(format!("--max-{nm} {v} outside {lo}..={hi}"));
        }
    }
    let mut r = Rng::new(seed);
    let (tm, tn) = tile_shape(mode);
    let mut shapes: Vec<(usize, usize, usize)> = vec![
        (1, 1, 1),
        (max_m, max_k, max_n),
        (max_m, 1, max_n),
        (1, max_k, 1),
        (tm, max_k.min(17), tn),
        ((tm + 1).min(max_m), 3.min(max_k), (tn + 1).min(max_n)),
        ((tm - 1).min(max_m), 2.min(max_k), (tn - 1).min(max_n)),
        (max_m.min(9), max_k.min(33), max_n.min(17)),
    ];
    shapes.truncate(n);
    let mut jobs = Vec::new();
    for i in 0..n {
        let (m, k, nn) = if i < shapes.len() {
            shapes[i]
        } else {
            (
                r.range_usize(1, max_m),
                r.range_usize(1, max_k),
                r.range_usize(1, max_n),
            )
        };
        let a = rand_mat(&mut r, m, k);
        let w = rand_mat(&mut r, k, nn);
        let bypass_fits = c_words(m, nn, true) <= OBUF_WORDS;
        let requant = !bypass_fits || r.chance(17, 20);
        let rq = if requant {
            Some(rand_params(&mut r, &a, &w))
        } else {
            None
        };
        let bases = rand_bases(&mut r, m, k, nn, !requant);
        let job = Job::new(mode, a, w, rq, bases).map_err(|e| format!("job {i}: {e}"))?;
        jobs.push(job_json(
            &format!("gemm_{}_s{seed}_{i}", mode_name(mode)),
            &job,
        ));
    }
    let doc = json!({
        "meta": meta.json(seed, "gemm"),
        "mode": mode_name(mode),
        "limits": {"max_m": max_m, "max_k": max_k, "max_n": max_n},
        "jobs": jobs,
    });
    Ok(vec![OutFile::json_compact(
        format!("gemm/gemm_{}_s{seed}_n{n}.json", mode_name(mode)),
        &doc,
    )])
}

// --------------------------------------------------------------------- directed

/// Fixed seed for the random fill inside directed cases.
pub const DIRECTED_SEED: u64 = 0xD1EC_7ED0;

/// `vec directed`: SPEC §6 directed cases, both modes.
pub fn directed(meta: &Meta) -> Vec<OutFile> {
    let mut r = Rng::new(DIRECTED_SEED);
    let mut jobs: Vec<Value> = Vec::new();
    let z = Bases::default();
    let mut add = |name: String, job: Job| jobs.push(job_json(&name, &job));
    let mk = |mode, a: MatI8, w: MatI8, rq: Option<Vec<(u16, u8)>>| {
        Job::new(mode, a, w, rq, z).expect("directed job is valid")
    };
    let fill = |rows, cols, v: i8| MatI8::from_fn(rows, cols, |_, _| v);

    for mode in [MODE_FULL, MODE_STRIP] {
        let mn = mode_name(mode);
        let (tm, tn) = tile_shape(mode);
        let m = tm; // one row tile
        let n = 16; // one W/C block
                    // Extreme products at K = K_MAX, bypass and requant.
        for (tag, av, wv) in [
            ("neg128_neg128", -128i8, -128i8),
            ("neg128_pos127", -128, 127),
            ("pos127_pos127", 127, 127),
        ] {
            let (a, w) = (fill(m, K_MAX, av), fill(K_MAX, n, wv));
            add(
                format!("{mn}_{tag}_k{K_MAX}_i32"),
                mk(mode, a.clone(), w.clone(), None),
            );
            let rq: Vec<(u16, u8)> = (0..n)
                .map(|j| match j % 4 {
                    0 => (RQ_MULT_MAX, RQ_SHIFT_MAX),
                    1 => (RQ_MULT_MAX, RQ_SHIFT_MIN),
                    2 => (RQ_MULT_MIN, RQ_SHIFT_MAX),
                    _ => (RQ_MULT_MIN, 16),
                })
                .collect();
            add(format!("{mn}_{tag}_k{K_MAX}_rq"), mk(mode, a, w, Some(rq)));
        }
        // All zeros.
        for k in [1, K_MAX] {
            add(
                format!("{mn}_zeros_k{k}_i32"),
                mk(mode, fill(m, k, 0), fill(k, n, 0), None),
            );
            add(
                format!("{mn}_zeros_k{k}_rq"),
                mk(
                    mode,
                    fill(m, k, 0),
                    fill(k, n, 0),
                    Some(vec![(RQ_MULT_MAX, RQ_SHIFT_MIN); n]),
                ),
            );
        }
        // Identity W (K = N = 16): C = A.
        let a = MatI8::from_fn(m, n, |_, _| r.i8());
        let ident = MatI8::from_fn(n, n, |i, j| i8::from(i == j));
        add(format!("{mn}_identity_w_i32"), mk(mode, a, ident, None));
        // All ones, odd K.
        add(
            format!("{mn}_all_ones_k37_i32"),
            mk(mode, fill(m, 37, 1), fill(37, n, 1), None),
        );
        // Single nonzero output element at every (i, j) of the tile (+ the 2nd FULL half-block).
        let (pm, pn) = if mode == MODE_FULL {
            (tm, 2 * tn)
        } else {
            (tm, tn)
        };
        for i in 0..pm {
            for j in 0..pn {
                let a = MatI8::from_fn(
                    pm,
                    3,
                    |ii, kk| if ii == i && kk == 1 { 100 + i as i8 } else { 0 },
                );
                let w = MatI8::from_fn(3, pn, |kk, jj| {
                    if jj == j && kk == 1 {
                        -(50 + j as i8)
                    } else {
                        0
                    }
                });
                add(
                    format!("{mn}_single_nz_r{i}_c{j}_i32"),
                    mk(mode, a, w, None),
                );
            }
        }
        // Quadrant isolation: only one physical quadrant's PEs see nonzero products.
        for q in 0..4usize {
            let job = if mode == MODE_STRIP {
                // STRIP: PE(i,c) -> C[i,c], PE(4+i,c) -> C[i,8+c]. Quadrant q covers
                // C columns 4q..4q+3 (Q0: 0-3, Q1: 4-7, Q2: 8-11, Q3: 12-15).
                let a = MatI8::from_fn(tm, 8, |_, _| r.range_i64(1, 127) as i8);
                let w = MatI8::from_fn(8, 16, |_, j| {
                    if j / 4 == q {
                        r.range_i64(1, 127) as i8
                    } else {
                        0
                    }
                });
                mk(mode, a, w, None)
            } else {
                // FULL: quadrant q = rows 4*(q/2).., cols 4*(q%2)..
                let (r0, c0) = (4 * (q / 2), 4 * (q % 2));
                let a = MatI8::from_fn(8, 8, |i, _| {
                    if (r0..r0 + 4).contains(&i) {
                        r.range_i64(1, 127) as i8
                    } else {
                        0
                    }
                });
                let w = MatI8::from_fn(8, 8, |_, j| {
                    if (c0..c0 + 4).contains(&j) {
                        r.range_i64(1, 127) as i8
                    } else {
                        0
                    }
                });
                mk(mode, a, w, None)
            };
            add(format!("{mn}_quadrant_q{q}_only_i32"), job);
        }
        // Requant ties at GEMM level: K = 1, W = 1, so acc = A; params create .5 cases.
        let tie_vals: [i8; 8] = [-1, -3, 1, 3, -127, 127, -128, 5];
        let a = MatI8::from_fn(m, 1, |i, _| tie_vals[i % 8]);
        let w = fill(1, n, 1);
        let tie_params: Vec<(u16, u8)> = (0..n)
            .map(|j| {
                *[
                    (1u16, 1u8),
                    (3, 2),
                    (1, 2),
                    (5, 3),
                    (RQ_MULT_MAX, 15),
                    (1, 1),
                    (7, 4),
                    (RQ_MULT_MAX, 16),
                ]
                .get(j % 8)
                .unwrap_or(&(1, 1))
            })
            .collect();
        add(
            format!("{mn}_requant_ties"),
            mk(mode, a, w, Some(tie_params)),
        );
        // Saturation both ways with extreme params.
        let a = MatI8::from_fn(m, K_MAX, |i, _| if i % 2 == 0 { 127 } else { -128 });
        let w = MatI8::from_fn(K_MAX, n, |_, j| if j % 2 == 0 { 127 } else { -128 });
        let sat_params: Vec<(u16, u8)> = (0..n)
            .map(|j| match j % 4 {
                0 => (RQ_MULT_MAX, RQ_SHIFT_MIN),
                1 => (RQ_MULT_MIN, RQ_SHIFT_MIN),
                2 => (RQ_MULT_MAX, RQ_SHIFT_MAX),
                _ => (RQ_MULT_MIN, RQ_SHIFT_MAX),
            })
            .collect();
        add(
            format!("{mn}_requant_saturation"),
            mk(mode, a, w, Some(sat_params)),
        );
    }

    // Edge shapes (both modes, same data): M/N not multiples, K = 1, max sizes.
    let edge_shapes: [(usize, usize, usize); 9] = [
        (1, 1, 1),
        (1, K_MAX, 1),
        (3, 1, 7),
        (5, 2, 9),
        (7, 17, 15),
        (9, 33, 17),
        (4, 64, 256),
        (M_MAX, 5, 3),
        (M_MAX, K_MAX, N_MAX),
    ];
    let mut seq_names = Vec::new();
    for (m, k, n) in edge_shapes {
        let a = MatI8::from_fn(m, k, |_, _| r.i8());
        let w = MatI8::from_fn(k, n, |_, _| r.i8());
        let rq = rand_params(&mut r, &a, &w);
        let full = mk(MODE_FULL, a.clone(), w.clone(), Some(rq.clone()));
        let tag = format!("edge_{m}x{k}x{n}");
        add(format!("full_{tag}_rq"), full.clone());
        add(format!("strip_{tag}_rq"), full.with_mode(MODE_STRIP));
        if c_words(m, n, true) <= OBUF_WORDS {
            let b = mk(MODE_FULL, a, w, None);
            add(format!("full_{tag}_i32"), b.clone());
            add(format!("strip_{tag}_i32"), b.with_mode(MODE_STRIP));
        }
        if (m, k, n) == (9, 33, 17) {
            seq_names = vec![
                format!("full_{tag}_rq"),
                format!("strip_{tag}_rq"),
                format!("full_{tag}_rq"),
            ];
        }
    }
    let requant_cases: Vec<Value> = unit::requant_directed()
        .into_iter()
        .map(|(nm, a, m, s)| json!({"name": nm, "acc": a, "mult": m, "shift": s, "y": splitsys_golden::requant(a, m, s)}))
        .collect();
    let doc = json!({
        "meta": meta.json(DIRECTED_SEED, "directed"),
        "description": "SPEC §6 directed cases. Each job: write `load` regions, RUN_GEMM with desc_hex, \
    then compare OBUF from expected.obuf_byte_addr on expected.compare_ranges (in-bounds bytes only) against expected.obuf_hex. \
    `sequences` lists jobs to run back to back with no reset (mode switching).",
        "jobs": jobs,
        "sequences": [{"name": "mode_switch_full_strip_full", "jobs": seq_names}],
        "requant_cases": requant_cases,
    });
    vec![OutFile::json_compact("directed.json", &doc)]
}
