//! GEMM runner: build a job with `splitsys_golden`, load it, run it, read back the
//! output and counters, check bit-exactly against golden and the SPEC §3.7
//! invariants, attach the `splitsys_cycle` prediction, and produce a run record.

use crate::client::{Board, HostError};
use crate::record::{
    iso_utc, now_unix, run_file_name, runs_dir, write_json, Check, Derived, GemmInfo, ModelInfo,
    Provenance, RunRecord,
};
use splitsys_cycle as cycle;
use splitsys_golden as golden;
use splitsys_spec as spec;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// One GEMM request.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct GemmSpec {
    pub name: String,
    #[serde(rename = "M")]
    pub m: usize,
    #[serde(rename = "K")]
    pub k: usize,
    #[serde(rename = "N")]
    pub n: usize,
    /// FULL or STRIP
    pub mode: String,
    #[serde(default = "yes")]
    pub requant: bool,
    #[serde(default = "default_seed")]
    pub seed: u64,
}

fn yes() -> bool {
    true
}
fn default_seed() -> u64 {
    1
}

pub fn mode_code(mode: &str) -> Result<u8, String> {
    match mode.to_ascii_uppercase().as_str() {
        "FULL" => Ok(spec::MODE_FULL),
        "STRIP" => Ok(spec::MODE_STRIP),
        m => Err(format!("mode '{m}' must be FULL or STRIP")),
    }
}

/// `(K, N)` of a SPEC §1.1 benchmark.
pub fn benchmark_kn(name: &str) -> Option<(usize, usize)> {
    spec::BENCH_SHAPES
        .iter()
        .find(|b| b.name == name)
        .map(|b| (b.k, b.n))
}

impl GemmSpec {
    pub fn named(name: &str, m: usize, mode: &str, seed: u64) -> Result<Self, String> {
        let (k, n) = benchmark_kn(name).ok_or_else(|| {
            let names: Vec<&str> = spec::BENCH_SHAPES.iter().map(|b| b.name).collect();
            format!("unknown benchmark '{name}' (known: {})", names.join(", "))
        })?;
        mode_code(mode)?;
        Ok(Self {
            name: name.into(),
            m,
            k,
            n,
            mode: mode.to_ascii_uppercase(),
            requant: true,
            seed,
        })
    }
}

fn name_salt(name: &str) -> u64 {
    // FNV-1a: a stable per-name salt for deterministic operand generation.
    name.bytes()
        .fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// Deterministic operands: W depends on (seed, name) only, so a benchmark's
/// weights are loaded once and reused across M and modes; A also depends on M.
pub fn build_job(g: &GemmSpec) -> Result<golden::Job, String> {
    let mode = mode_code(&g.mode)?;
    let mut root = golden::rng::Rng::new(g.seed);
    let mut wr = root.fork(name_salt(&g.name));
    let mut ar = root.fork(name_salt(&g.name) ^ (g.m as u64).wrapping_mul(0x9E37_79B9));
    let w = golden::MatI8::from_fn(g.k, g.n, |_, _| wr.i8());
    let a = golden::MatI8::from_fn(g.m, g.k, |_, _| ar.i8());
    let rq = if g.requant {
        let acc = golden::gemm_i32(&a, &w);
        Some(
            (0..g.n)
                .map(|c| {
                    let max = (0..g.m).map(|r| acc.get(r, c).unsigned_abs()).max().unwrap_or(1);
                    golden::choose_rq_param(u64::from(max), 127)
                })
                .collect(),
        )
    } else {
        None
    };
    golden::Job::new(mode, a, w, rq, golden::Bases::default()).map_err(|e| e.to_string())
}

/// What the host believes is already in board memory (to skip reloads).
#[derive(Default, Debug)]
pub struct LoadCache {
    w: Option<(String, u64, usize, usize)>,
    a: Option<(String, u64, usize, usize)>,
    rq: Option<Vec<u8>>,
}

impl LoadCache {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Process-wide provenance strings.
#[derive(Clone, Debug)]
pub struct Context {
    pub results: PathBuf,
    pub git_hash: String,
    pub model_git_hash: String,
    pub build_id: Option<String>,
    /// Overwrite OBUF with the bitwise complement of the expected image before
    /// each run, so a stale or unwritten byte can never pass the compare.
    pub poison: bool,
    pub write_files: bool,
}

/// The host binary's own git hash, embedded at build time by build.rs (HEAD,
/// `-dirty` if sw/crates/host or demo/ had uncommitted changes).
pub const HOST_GIT_HASH: &str = env!("SPLITSYS_HOST_GIT_HASH");

impl Context {
    pub fn new(results: PathBuf, _root: &Path) -> Self {
        Self {
            results,
            git_hash: HOST_GIT_HASH.to_string(),
            // Identifies the MODEL: last commit touching the model inputs (cycle, spec),
            // `-dirty`, SPLITSYS_MODEL_GIT_HASH override — splitsys_cycle's own rule.
            model_git_hash: cycle::report::model_git_hash(),
            build_id: None,
            poison: true,
            write_files: true,
        }
    }
}

#[derive(Debug)]
pub enum RunError {
    Host(HostError),
    Job(String),
}
impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Host(e) => write!(f, "{e}"),
            RunError::Job(e) => write!(f, "job error: {e}"),
        }
    }
}
impl From<HostError> for RunError {
    fn from(e: HostError) -> Self {
        RunError::Host(e)
    }
}

/// Model prediction + timeline for a spec (dashboard animation, labelled model).
pub fn model_for(g: &GemmSpec) -> Result<(cycle::Descriptor, cycle::Counters), String> {
    let mode = cycle::Mode::from_code(mode_code(&g.mode)?).ok_or("mode")?;
    let flags = golden::Descriptor::flags_for(g.requant);
    let d = cycle::Descriptor::new(mode, flags, g.m as u64, g.k as u64, g.n as u64);
    let c = cycle::predict(&d, &cycle::Latencies::default()).map_err(|e| format!("{e:?}"))?;
    Ok((d, c))
}

/// The `source` tag for a run. A board that answers PING with the mock build id
/// is the mock (e.g. `mock-bridge` over tcp:), so its numbers are MODEL, never SIM/HW.
pub fn effective_source(t: crate::transport::Source, build_id_hex: Option<&str>) -> &'static str {
    if build_id_hex == Some(MOCK_BUILD_ID_HEX) {
        "model"
    } else {
        t.as_str()
    }
}

/// `MOCK_BUILD_ID` as the 8-hex-digit string used in records.
pub const MOCK_BUILD_ID_HEX: &str = "4d4f434b";

/// Every counter (all 12, READ_COUNTERS order) that differs from the model.
pub fn counter_diffs(meas: &cycle::Counters, model: &cycle::Counters) -> Vec<String> {
    meas.readback()
        .into_iter()
        .zip(model.readback())
        .filter(|((_, a), (_, b))| a != b)
        .map(|((name, a), (_, b))| {
            let e = if b == 0 { f64::INFINITY } else { (a as f64 - b as f64) / b as f64 * 100.0 };
            format!("{name}: {a} vs model {b} ({e:+.2}%)")
        })
        .collect()
}

/// Measured utilization in permille for SET_DISPLAY: round-half-up of
/// `1000 · mac_useful / (64 · cycles_total)`, integer-only, clamped to 0..=1000.
pub fn util_permille(c: &cycle::Counters) -> u16 {
    let den = u128::from(c.cycles_total) * spec::N_PES as u128;
    if den == 0 {
        return 0;
    }
    let p = (u128::from(c.mac_useful) * 2000 + den) / (2 * den);
    p.min(1000) as u16
}

/// Run one GEMM end to end and return its record (written to disk if enabled).
pub fn run_gemm(
    board: &mut Board,
    ctx: &mut Context,
    cache: &mut LoadCache,
    g: &GemmSpec,
) -> Result<(RunRecord, Option<PathBuf>), RunError> {
    let job = build_job(g).map_err(RunError::Job)?;
    run_job(board, ctx, cache, g, &job, true)
}

/// Run a prebuilt golden job. `g` supplies the record's name/seed and must match
/// the job's shape, mode and requant. With `cacheable = false` (e.g. random jobs
/// at random bases) every image is written and the load cache is invalidated
/// afterwards, since the job may overwrite what the cache believes is resident.
pub fn run_job(
    board: &mut Board,
    ctx: &mut Context,
    cache: &mut LoadCache,
    g: &GemmSpec,
    job: &golden::Job,
    cacheable: bool,
) -> Result<(RunRecord, Option<PathBuf>), RunError> {
    if !cacheable {
        cache.clear();
    }
    let r = run_job_inner(board, ctx, cache, g, job);
    if !cacheable {
        cache.clear();
    }
    r
}

fn run_job_inner(
    board: &mut Board,
    ctx: &mut Context,
    cache: &mut LoadCache,
    g: &GemmSpec,
    job: &golden::Job,
) -> Result<(RunRecord, Option<PathBuf>), RunError> {
    let t0 = Instant::now();
    if job.desc.dims() != (g.m, g.k, g.n)
        || job.desc.mode != mode_code(&g.mode).map_err(RunError::Job)?
        || job.c.is_some() != g.requant
    {
        return Err(RunError::Job(format!("spec {g:?} does not match job {:?}", job.desc)));
    }
    let (cd, model) = model_for(g).map_err(RunError::Job)?;
    if ctx.build_id.is_none() {
        let p = board.ping()?;
        ctx.build_id = Some(format!("{:08x}", p.build_id));
    }
    // Make sure no job is running (BUSY otherwise).
    if board.poll()? == spec::POLL_BUSY {
        board.wait_done(Duration::from_secs(5))?;
    }
    // Load operands, skipping what is already resident.
    let wkey = (g.name.clone(), g.seed, g.k, g.n);
    if cache.w.as_ref() != Some(&wkey) {
        board.write_mem(spec::REGION_WBUF, job.w_byte_addr(), &job.w_image)?;
        cache.w = Some(wkey);
    }
    let akey = (g.name.clone(), g.seed, g.m, g.k);
    if cache.a.as_ref() != Some(&akey) {
        board.write_mem(spec::REGION_ABUF, job.a_byte_addr(), &job.a_image)?;
        cache.a = Some(akey);
    }
    if !job.rq_image.is_empty() && cache.rq.as_ref() != Some(&job.rq_image) {
        board.write_mem(spec::REGION_RQPARAM, job.rq_byte_addr(), &job.rq_image)?;
        cache.rq = Some(job.rq_image.clone());
    }
    if ctx.poison {
        let poison: Vec<u8> = job.obuf_image.iter().map(|b| !b).collect();
        board.write_mem(spec::REGION_OBUF, job.out_byte_addr(), &poison)?;
    }
    board.run_gemm(&job.desc.to_bytes())?;
    board.wait_done(Duration::from_secs(10))?;
    let counters = board.read_counters()?;
    let readback = board.read_mem(spec::REGION_OBUF, job.out_byte_addr(), job.obuf_image.len())?;
    // The 7-segment display has no divider: the host sends what it shows (e.g. `S 21.4`).
    board.set_display(util_permille(&counters), job.desc.mode)?;

    // Bit-exact compare of every in-bounds output element (D-006: padding is unspecified).
    let (m, n) = (g.m, g.n);
    let mut mismatches = 0u64;
    let mut first_bad: Option<u64> = None;
    for i in 0..m {
        for j in 0..n {
            let ok = if g.requant {
                let off = golden::pack::c_i8_offset(m, i, j);
                readback[off] == job.obuf_image[off]
            } else {
                let off = golden::pack::c_i32_offset(m, i, j);
                readback[off..off + 4] == job.obuf_image[off..off + 4]
            };
            if !ok {
                mismatches += 1;
                first_bad.get_or_insert((i * n + j) as u64);
            }
        }
    }
    let inv = counters.check_invariants(&cd, &cycle::Latencies::default());

    let bpc = cd.mode.operand_bytes_per_cycle() as f64;
    let streamed = cycle::operand_bytes_streamed(&cd, &counters) as f64;
    let ct = counters.cycles_total as f64;
    let now = now_unix();
    let src = effective_source(board.source(), ctx.build_id.as_deref());
    let mut run_id = format!(
        "{}-{}-m{}-{}",
        iso_utc(now),
        g.name,
        g.m,
        g.mode.to_ascii_lowercase()
    );
    let dir = runs_dir(&ctx.results, src);
    if ctx.write_files {
        let mut i = 2;
        while dir.join(run_file_name(&run_id)).exists() {
            run_id = format!(
                "{}-{}-m{}-{}-{i}",
                iso_utc(now),
                g.name,
                g.m,
                g.mode.to_ascii_lowercase()
            );
            i += 1;
        }
    }
    let rec = RunRecord {
        mock: ctx.build_id.as_deref() == Some(MOCK_BUILD_ID_HEX),
        run_id: run_id.clone(),
        source: src.into(),
        build_id: ctx.build_id.clone().unwrap_or_default(),
        git_hash: ctx.git_hash.clone(),
        clock_mhz: spec::CLK_HZ / 1_000_000,
        gemm: GemmInfo {
            name: g.name.clone(),
            m: g.m as u64,
            k: g.k as u64,
            n: g.n as u64,
            mode: g.mode.to_ascii_uppercase(),
            requant: g.requant,
        },
        counters,
        model: ModelInfo {
            counters: model,
            model_git_hash: ctx.model_git_hash.clone(),
        },
        derived: Derived {
            util: counters.utilization(),
            util_model: model.utilization(),
            operand_bytes_per_cycle: bpc,
            model_err_total: if model.cycles_total == 0 {
                0.0
            } else {
                (ct - model.cycles_total as f64) / model.cycles_total as f64
            },
            operand_bytes_per_cycle_avg: (ct > 0.0).then(|| streamed / ct),
            macs_per_cycle: Some(counters.macs_per_cycle()),
            arithmetic_intensity: (streamed > 0.0).then(|| counters.mac_useful as f64 / streamed),
        },
        check: Check {
            golden_match: mismatches == 0,
            outputs_compared: (m * n) as u64,
            mismatches,
            invariants_ok: inv.is_ok(),
            invariant_error: inv.err(),
            first_mismatch: first_bad,
            counter_diffs: counter_diffs(&counters, &model),
        },
        provenance: Provenance {
            transport: Some(board.describe()),
            seed: Some(g.seed),
            wall_ms_end_to_end: Some(t0.elapsed().as_secs_f64() * 1000.0),
            latencies_provisional: Some(cycle::spec::LATENCIES_PROVISIONAL),
        },
    };
    let path = if ctx.write_files {
        let p = dir.join(run_file_name(&run_id));
        write_json(&p, &rec).map_err(|e| RunError::Job(format!("write {}: {e}", p.display())))?;
        Some(p)
    } else {
        None
    };
    Ok((rec, path))
}

/// The benchmark suites. `bench`: 4 GEMMs × M∈{1,64} × both modes.
/// `sweep`: 4 GEMMs × M∈{1..64} × both modes. `smoke`: ffn_up M∈{1,64} × both.
pub fn suite(name: &str, seed: u64) -> Result<Vec<GemmSpec>, String> {
    let ms: Vec<usize> = match name {
        "bench" | "smoke" => vec![spec::BENCH_M_DECODE, spec::BENCH_M_PREFILL],
        "sweep" | "all" => spec::BENCH_M_SWEEP.to_vec(),
        s => return Err(format!("unknown suite '{s}' (bench | sweep | smoke)")),
    };
    let names: Vec<&str> = if name == "smoke" {
        vec!["ffn_up"]
    } else {
        spec::BENCH_SHAPES.iter().map(|b| b.name).collect()
    };
    let mut v = Vec::new();
    for n in names {
        for &m in &ms {
            for mode in ["FULL", "STRIP"] {
                v.push(GemmSpec::named(n, m, mode, seed)?);
            }
        }
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockConfig;
    use crate::transport::MockTransport;

    fn ctx() -> Context {
        Context {
            results: PathBuf::from("unused"),
            git_hash: "test".into(),
            model_git_hash: "test".into(),
            build_id: None,
            poison: true,
            write_files: false,
        }
    }

    #[test]
    fn mock_runs_every_benchmark_bit_exact_with_invariants() {
        let mut b = Board::new(Box::new(MockTransport::new(MockConfig::default())));
        let mut c = ctx();
        let mut cache = LoadCache::default();
        for g in suite("bench", 7).unwrap() {
            let (r, _) = run_gemm(&mut b, &mut c, &mut cache, &g).unwrap();
            assert!(r.verified(), "{}: {:?}", r.run_id, r.check);
            assert_eq!(r.counters, r.model.counters, "mock counters come from the model");
            assert_eq!(r.counters.mac_useful, (g.m * g.n * g.k) as u64);
            assert_eq!(r.source, "model");
            assert!(r.mock);
            assert_eq!(r.build_id, "4d4f434b");
            crate::record::validate_run_json(&serde_json::to_value(&r).unwrap()).unwrap();
        }
    }

    #[test]
    fn weights_loaded_once_per_benchmark() {
        let mut b = Board::new(Box::new(MockTransport::new(MockConfig::default())));
        let mut c = ctx();
        c.poison = false;
        let mut cache = LoadCache::default();
        let g = GemmSpec::named("ffn_up", 1, "FULL", 3).unwrap();
        run_gemm(&mut b, &mut c, &mut cache, &g).unwrap();
        let tx1 = b.stats.bytes_tx;
        let g2 = GemmSpec::named("ffn_up", 1, "STRIP", 3).unwrap();
        run_gemm(&mut b, &mut c, &mut cache, &g2).unwrap();
        let tx2 = b.stats.bytes_tx - tx1;
        // Second run sends no W/A/RQ data: only POLL/RUN/READ_COUNTERS/READ_MEM requests.
        assert!(tx2 < 200, "second run sent {tx2} bytes");
    }

    #[test]
    fn corrupted_obuf_is_detected() {
        // A board that drops the GEMM output: run, then overwrite OBUF behind the
        // host's back by poisoning via a second host write. Simulate by turning
        // poison on and making the mock never execute: use a descriptor mismatch.
        let mut b = Board::new(Box::new(MockTransport::new(MockConfig::default())));
        let mut c = ctx();
        let mut cache = LoadCache::default();
        let g = GemmSpec::named("attn_out", 4, "STRIP", 5).unwrap();
        let (r, _) = run_gemm(&mut b, &mut c, &mut cache, &g).unwrap();
        assert!(r.verified());
        // Same job but host thinks W is resident while the board has different W.
        let other = build_job(&GemmSpec::named("qkv", 4, "STRIP", 99).unwrap()).unwrap();
        b.write_mem(spec::REGION_WBUF, 0, &other.w_image).unwrap();
        let (r2, _) = run_gemm(&mut b, &mut c, &mut cache, &g).unwrap();
        assert!(!r2.check.golden_match);
        assert!(r2.check.mismatches > 0);
        assert!(!r2.verified());
    }

    #[test]
    fn mock_over_tcp_is_tagged_model_not_sim() {
        use crate::transport::{fake::spawn_tcp_bridge, open, TransportSpec};
        let addr = spawn_tcp_bridge(MockConfig::default(), 64);
        let t = open(&TransportSpec::Tcp(addr), 115200, Some(1), MockConfig::default()).unwrap();
        let mut b = Board::new(t);
        let mut c = ctx();
        let mut cache = LoadCache::default();
        let g = GemmSpec::named("attn_out", 1, "STRIP", 1).unwrap();
        let (r, _) = run_gemm(&mut b, &mut c, &mut cache, &g).unwrap();
        assert_eq!(r.source, "model");
        assert_eq!(MOCK_BUILD_ID_HEX, format!("{:08x}", crate::mock::MOCK_BUILD_ID));
    }

    #[test]
    fn util_permille_rounding() {
        let c = |mac: u64, cyc: u64| cycle::Counters { mac_useful: mac, cycles_total: cyc, ..Default::default() };
        assert_eq!(util_permille(&c(0, 0)), 0);
        assert_eq!(util_permille(&c(64, 1)), 1000);
        assert_eq!(util_permille(&c(16384, 1730)), 148); // 14.798% -> 148
        assert_eq!(util_permille(&c(1, 128)), 0); // 0.122 permille -> 0
        assert_eq!(util_permille(&c(32, 1000)), 1); // exactly 0.5 permille rounds up
    }

    #[test]
    fn json_gemm_spec_parses() {
        let g: GemmSpec =
            serde_json::from_str(r#"{"name":"custom","M":3,"K":17,"N":33,"mode":"STRIP"}"#).unwrap();
        assert!(g.requant);
        let j = build_job(&g).unwrap();
        assert_eq!(j.desc.dims(), (3, 17, 33));
    }

    #[test]
    fn int32_bypass_runs_bit_exact() {
        let mut b = Board::new(Box::new(MockTransport::new(MockConfig::default())));
        let mut c = ctx();
        let mut cache = LoadCache::default();
        let mut g = GemmSpec::named("attn_out", 8, "FULL", 2).unwrap();
        g.requant = false;
        let (r, _) = run_gemm(&mut b, &mut c, &mut cache, &g).unwrap();
        assert!(r.verified(), "{:?}", r.check);
    }
}

#[cfg(test)]
mod hash_tests {
    use super::*;

    #[test]
    fn model_hash_is_cycle_crate_rule_and_host_hash_is_separate() {
        let c = Context::new(PathBuf::from("unused"), Path::new("."));
        assert_eq!(c.model_git_hash, cycle::report::model_git_hash());
        assert_eq!(c.git_hash, HOST_GIT_HASH);
        assert!(!HOST_GIT_HASH.is_empty());
        // The model hash is the inputs-scoped 12-hex-digit form (or an override /
        // "unknown"), never the host's 7-digit HEAD hash.
        if std::env::var("SPLITSYS_MODEL_GIT_HASH").is_err() && c.model_git_hash != "unknown" {
            let h = c.model_git_hash.trim_end_matches("-dirty");
            assert_eq!(h.len(), 12, "{h}");
        }
    }
}
