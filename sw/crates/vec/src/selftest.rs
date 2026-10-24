//! Self-test ROM image (SPEC §3.9). Format: see `sw/crates/vec/README.md`.
//!
//! The ROM is a flat array of 32-bit words (`$readmemh`, 8 hex digits per line).
//! ROM byte address `4*i + b` is bits `[8b+7:8b]` of word i (little-endian).
//!
//! ```text
//! word 0                 MAGIC = 0x53535431
//! word 1                 VERSION = 1
//! word 2                 N_JOBS (= 2: FULL then STRIP)
//! word 3                 N_LOADS
//! word 4                 TOTAL_WORDS (ROM length)
//! word 5                 JOB_TBL  (word offset of the job table, = 8)
//! word 6                 LOAD_TBL (word offset of the load table)
//! word 7                 0 (reserved)
//! JOB_TBL + 8*j + 0..5   descriptor j, 24 bytes little-endian (desc byte 4w+b = word w bits 8b+7:8b)
//! JOB_TBL + 8*j + 6      expected Fletcher-32 of job j's checked OBUF bytes
//! JOB_TBL + 8*j + 7      CHECK_WORDS: number of 128-bit OBUF words checked, from desc.out_base
//! LOAD_TBL + 4*l + 0     region code (0 WBUF, 1 ABUF, 2 OBUF, 3 RQPARAM)
//! LOAD_TBL + 4*l + 1     destination byte address in that region
//! LOAD_TBL + 4*l + 2     byte length (multiple of 4)
//! LOAD_TBL + 4*l + 3     source ROM word offset
//! ```
//!
//! Procedure: perform every load (copy `len/4` ROM words to the region, word k going to
//! byte address `dst + 4k`, little-endian), then for each job in order: run it, compute
//! Fletcher-32 over OBUF bytes `[out_base*16, (out_base + CHECK_WORDS)*16)` in address
//! order, pass iff equal to the stored value. Each job writes every byte it checks, so
//! the result does not depend on prior OBUF contents.

use crate::jobs::{job_json, mode_name};
use crate::{Meta, OutFile};
use serde_json::json;
use splitsys_golden::consts::*;
use splitsys_golden::pack::{a_words, c_words, rq_entries, w_words};
use splitsys_golden::rng::Rng;
use splitsys_golden::{choose_rq_param, fletcher32, gemm_i32, requant_is_tie, Bases, Job, MatI8};

pub const SELFTEST_SEED: u64 = 0x5E1F_7E57;
pub const ROM_MAGIC: u32 = 0x5353_5431;
pub const ROM_VERSION: u32 = 1;
pub const ROM_HDR_WORDS: usize = 8;
pub const ROM_JOB_WORDS: usize = 8;
pub const ROM_LOAD_WORDS: usize = 4;
/// ≤ 4 BRAM36 (4 × 4 KiB data) per SPEC §3.4.
pub const ROM_MAX_WORDS: usize = 4096;

/// Self-test job shapes: (mode, M, K, N). STRIP uses M = 6 so that its two 4-row
/// tiles cover the whole M_pad = 8 block (every checked byte is written) while still
/// exercising a partial STRIP tile and zero padding rows.
pub const SELFTEST_SHAPES: [(u8, usize, usize, usize); 2] =
    [(MODE_FULL, 8, 32, 16), (MODE_STRIP, 6, 32, 16)];

/// One load record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Load {
    pub region: u8,
    pub dst_byte_addr: usize,
    pub bytes: Vec<u8>,
    pub src_word: usize,
}

/// The ROM and everything needed to explain it.
#[derive(Clone, Debug)]
pub struct SelftestRom {
    pub words: Vec<u32>,
    pub jobs: Vec<Job>,
    pub job_seeds: Vec<u64>,
    pub fletcher: Vec<u32>,
    pub check_words: Vec<usize>,
    pub loads: Vec<Load>,
    pub load_tbl: usize,
}

/// Coverage statistics of one requantized job.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub sat_pos: usize,
    pub sat_neg: usize,
    pub zero: usize,
    pub mid: usize,
    pub ties: usize,
}

pub fn stats(job: &Job) -> Stats {
    let c = job.c.as_ref().expect("requant job");
    let mut s = Stats::default();
    for i in 0..c.rows() {
        for j in 0..c.cols() {
            match c.get(i, j) {
                127 => s.sat_pos += 1,
                -128 => s.sat_neg += 1,
                0 => s.zero += 1,
                _ => s.mid += 1,
            }
            let (m, sh) = job.rq[j];
            if requant_is_tie(job.acc.get(i, j), m, sh) {
                s.ties += 1;
            }
        }
    }
    s
}

fn candidate(seed: u64, mode: u8, m: usize, k: usize, n: usize, bases: Bases) -> Job {
    let mut r = Rng::new(seed);
    // Rows with different magnitudes: small rows make ties and mid-range values likely,
    // large rows make saturation likely.
    let a = MatI8::from_fn(m, k, |i, _| {
        let lim = [2i64, 5, 127, 30, 127, 9, 64, 127][i % 8];
        r.range_i64(-lim.min(128), lim) as i8
    });
    let w = MatI8::from_fn(k, n, |_, _| r.i8());
    let acc = gemm_i32(&a, &w);
    let rq: Vec<(u16, u8)> = (0..n)
        .map(|j| match j % 4 {
            0 => (r.u16_in(1, 3), r.u8_in(1, 3)), // small shifts: ties, saturation
            _ => {
                let max_abs = (0..m)
                    .map(|i| acc.get(i, j).unsigned_abs())
                    .max()
                    .unwrap_or(0);
                choose_rq_param(u64::from(max_abs), r.range_i64(60, 300) as u64)
            }
        })
        .collect();
    Job::new(mode, a, w, Some(rq), bases).expect("self-test job valid")
}

fn good(job: &Job) -> bool {
    let s = stats(job);
    let total = job.desc.m as usize * job.desc.n as usize;
    job.writes_whole_region() && s.sat_pos > 0 && s.sat_neg > 0 && s.ties > 0 && s.mid * 4 >= total
}

/// Build the ROM. Deterministic: each job's data comes from the first seed at or
/// after `SELFTEST_SEED + 1000*j` whose job meets the coverage requirements.
pub fn build_rom() -> SelftestRom {
    let mut jobs = Vec::new();
    let mut job_seeds = Vec::new();
    let mut next = Bases::default();
    for (j, &(mode, m, k, n)) in SELFTEST_SHAPES.iter().enumerate() {
        let bases = next;
        let mut found = None;
        for attempt in 0..1000u64 {
            let s = SELFTEST_SEED + 1000 * j as u64 + attempt;
            let job = candidate(s, mode, m, k, n, bases);
            if good(&job) {
                found = Some((job, s));
                break;
            }
        }
        let (job, s) = found.expect("no self-test candidate met the coverage requirements");
        next = Bases {
            a: bases.a + a_words(m, k) as u32,
            w: bases.w + w_words(k, n) as u32,
            out: bases.out + c_words(m, n, false) as u32,
            rq: bases.rq + rq_entries(n) as u16,
        };
        jobs.push(job);
        job_seeds.push(s);
    }

    let n_jobs = jobs.len();
    let mut loads = Vec::new();
    for job in &jobs {
        loads.push((REGION_ABUF, job.a_byte_addr(), job.a_image.clone()));
        loads.push((REGION_WBUF, job.w_byte_addr(), job.w_image.clone()));
        loads.push((REGION_RQPARAM, job.rq_byte_addr(), job.rq_image.clone()));
    }
    let job_tbl = ROM_HDR_WORDS;
    let load_tbl = job_tbl + ROM_JOB_WORDS * n_jobs;
    let mut data_off = load_tbl + ROM_LOAD_WORDS * loads.len();
    let loads: Vec<Load> = loads
        .into_iter()
        .map(|(region, dst, bytes)| {
            assert!(bytes.len() % 4 == 0 && dst % 4 == 0);
            let l = Load {
                region,
                dst_byte_addr: dst,
                src_word: data_off,
                bytes,
            };
            data_off += l.bytes.len() / 4;
            l
        })
        .collect();
    let total = data_off;
    assert!(
        total <= ROM_MAX_WORDS,
        "self-test ROM too large: {total} words"
    );

    let fletcher: Vec<u32> = jobs.iter().map(|j| fletcher32(&j.obuf_image)).collect();
    let check_words: Vec<usize> = jobs
        .iter()
        .map(|j| j.obuf_image.len() / OBUF_WORD_BYTES)
        .collect();

    let mut words = vec![
        ROM_MAGIC,
        ROM_VERSION,
        n_jobs as u32,
        loads.len() as u32,
        total as u32,
        job_tbl as u32,
        load_tbl as u32,
        0,
    ];
    for (i, job) in jobs.iter().enumerate() {
        let d = job.desc.to_bytes();
        for c in d.chunks(4) {
            words.push(u32::from_le_bytes([c[0], c[1], c[2], c[3]]));
        }
        words.push(fletcher[i]);
        words.push(check_words[i] as u32);
    }
    for l in &loads {
        words.extend_from_slice(&[
            u32::from(l.region),
            l.dst_byte_addr as u32,
            l.bytes.len() as u32,
            l.src_word as u32,
        ]);
    }
    for l in &loads {
        for c in l.bytes.chunks(4) {
            words.push(u32::from_le_bytes([c[0], c[1], c[2], c[3]]));
        }
    }
    assert_eq!(words.len(), total);
    SelftestRom {
        words,
        jobs,
        job_seeds,
        fletcher,
        check_words,
        loads,
        load_tbl,
    }
}

/// `vec selftest`: `selftest_rom.mem` and `selftest_expected.json`.
pub fn selftest(meta: &Meta) -> Vec<OutFile> {
    let rom = build_rom();
    let mut mem = meta.mem_header(
        SELFTEST_SEED,
        "selftest_rom",
        &[
            format!(
                "{} words x 32 bit, little-endian bytes within a word",
                rom.words.len()
            ),
            "format: sw/crates/vec/README.md (header, job table, load table, data)".into(),
        ],
    );
    for w in &rom.words {
        mem.push_str(&format!("{w:08x}\n"));
    }
    let jobs: Vec<_> = rom
        .jobs
        .iter()
        .enumerate()
        .map(|(i, job)| {
            let s = stats(job);
            json!({
                "index": i,
                "mode": mode_name(job.desc.mode),
                "data_seed": rom.job_seeds[i],
                "rom_job_word_offset": ROM_HDR_WORDS + ROM_JOB_WORDS * i,
                "fletcher32": rom.fletcher[i],
                "fletcher32_hex": format!("{:08x}", rom.fletcher[i]),
                "check_words": rom.check_words[i],
                "check_byte_addr": job.out_byte_addr(),
                "check_bytes": job.obuf_image.len(),
                "stats": {"sat_pos": s.sat_pos, "sat_neg": s.sat_neg, "zero": s.zero, "mid": s.mid, "ties": s.ties},
                "job": job_json(&format!("selftest_{}", mode_name(job.desc.mode)), job),
            })
        })
        .collect();
    let loads: Vec<_> = rom
        .loads
        .iter()
        .map(|l| json!({"region": l.region, "dst_byte_addr": l.dst_byte_addr, "byte_len": l.bytes.len(), "src_word": l.src_word}))
        .collect();
    let doc = json!({
        "meta": meta.json(SELFTEST_SEED, "selftest"),
        "rom_file": "selftest_rom.mem",
        "rom_words": rom.words.len(),
        "rom_word_bits": 32,
        "format": {
            "byte_order": "ROM byte 4i+b = word i bits [8b+7:8b] (little-endian)",
            "header": ["MAGIC", "VERSION", "N_JOBS", "N_LOADS", "TOTAL_WORDS", "JOB_TBL", "LOAD_TBL", "RESERVED"],
            "magic": ROM_MAGIC,
            "version": ROM_VERSION,
            "job_table_offset": ROM_HDR_WORDS,
            "job_record_words": ROM_JOB_WORDS,
            "job_record": ["DESC0", "DESC1", "DESC2", "DESC3", "DESC4", "DESC5", "FLETCHER32", "CHECK_WORDS"],
            "load_table_offset": rom.load_tbl,
            "load_record_words": ROM_LOAD_WORDS,
            "load_record": ["REGION", "DST_BYTE_ADDR", "BYTE_LEN", "SRC_WORD"],
            "checksum": "Fletcher-32 over OBUF bytes [out_base*16, (out_base+CHECK_WORDS)*16) in address order; \
    16-bit LE words; sums init 0, mod 65535; result (sum2<<16)|sum1",
        },
        "loads": loads,
        "jobs": jobs,
        "rom_hex_words": rom.words.iter().map(|w| format!("{w:08x}")).collect::<Vec<_>>().join(""),
    });
    vec![
        OutFile::text("selftest_rom.mem", mem),
        OutFile::json_pretty("selftest_expected.json", &doc),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use splitsys_golden::pack::{unpack_a_blocks8, unpack_rqparams, unpack_w_blocks16};
    use splitsys_golden::{pack_c_i8_blocks16, requant_mat, Descriptor};

    /// Interpret the ROM like the RTL would, with independent memories, and check that
    /// the stored checksums are reproduced.
    #[test]
    fn rom_emulation_reproduces_checksums() {
        let rom = build_rom();
        let w = &rom.words;
        assert_eq!(w[0], ROM_MAGIC);
        assert_eq!(w[4] as usize, w.len());
        let (n_jobs, n_loads) = (w[2] as usize, w[3] as usize);
        let (job_tbl, load_tbl) = (w[5] as usize, w[6] as usize);
        let mut mem = [
            vec![0u8; WBUF_WORDS * WBUF_WORD_BYTES],
            vec![0u8; ABUF_WORDS * ABUF_WORD_BYTES],
            vec![0xEEu8; OBUF_WORDS * OBUF_WORD_BYTES], // garbage: must not matter
            vec![0u8; RQ_ENTRIES * RQ_WORD_BYTES],
        ];
        for l in 0..n_loads {
            let rec = &w[load_tbl + 4 * l..load_tbl + 4 * l + 4];
            let (region, dst, len, src) = (
                rec[0] as usize,
                rec[1] as usize,
                rec[2] as usize,
                rec[3] as usize,
            );
            for k in 0..len / 4 {
                mem[region][dst + 4 * k..dst + 4 * k + 4]
                    .copy_from_slice(&w[src + k].to_le_bytes());
            }
        }
        for j in 0..n_jobs {
            let rec = &w[job_tbl + 8 * j..job_tbl + 8 * j + 8];
            let db: Vec<u8> = rec[..6].iter().flat_map(|x| x.to_le_bytes()).collect();
            let d = Descriptor::from_bytes(&db).unwrap();
            d.validate().unwrap();
            let (m, k, n) = d.dims();
            let a = unpack_a_blocks8(&mem[1][d.a_base as usize * 8..], m, k);
            let wm = unpack_w_blocks16(&mem[0][d.w_base as usize * 16..], k, n);
            let rq = unpack_rqparams(&mem[3][usize::from(d.rq_base) * 4..], n);
            let img = pack_c_i8_blocks16(&requant_mat(&gemm_i32(&a, &wm), &rq));
            let out = d.out_base as usize * 16;
            // Write only the bytes this mode writes (the rest of OBUF stays garbage).
            let mask = splitsys_golden::obuf_written_mask(&d);
            for (i, (&b, &wr)) in img.iter().zip(&mask).enumerate() {
                if wr {
                    mem[2][out + i] = b;
                }
            }
            let check = rec[7] as usize * 16;
            assert_eq!(fletcher32(&mem[2][out..out + check]), rec[6], "job {j}");
        }
        assert_eq!(rom.jobs[0].desc.mode, MODE_FULL);
        assert_eq!(rom.jobs[1].desc.mode, MODE_STRIP);
        assert!(rom.words.len() <= ROM_MAX_WORDS);
    }
}
