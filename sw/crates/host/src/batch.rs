//! Random-GEMM batch (checklist H-5). Jobs come from `splitsys_vec::jobs::gemm`,
//! the generator behind `splitsys-vec gemm --seed S --n N --mode M --max-m ..`, so a
//! failing job is reproduced in simulation with the same command and job index.
//! The host rebuilds each `golden::Job` from the generator's operands and checks
//! that its descriptor bytes and expected OBUF image equal the generator's.

use crate::client::Board;
use crate::record::RunRecord;
use crate::runner::{run_job, Context, GemmSpec, LoadCache, RunError};
use serde_json::Value;
use splitsys_golden as golden;
use splitsys_spec as spec;
use std::path::PathBuf;

/// One generated job with its provenance.
pub struct BatchJob {
    /// e.g. `gemm_full_s1_7`: job 7 of `splitsys-vec gemm --seed 1 --mode full`.
    pub name: String,
    pub index: usize,
    pub spec: GemmSpec,
    pub job: golden::Job,
    pub desc_hex: String,
}

fn unhex(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err("odd hex length".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

fn u(v: &Value, k: &str) -> Result<usize, String> {
    v[k].as_u64().map(|x| x as usize).ok_or_else(|| format!("job field {k} missing"))
}

/// The `splitsys-vec gemm` command line that regenerates a batch in sim.
pub fn vec_command(seed: u64, n: usize, mode: &str, max: (usize, usize, usize)) -> String {
    format!(
        "splitsys-vec gemm --seed {seed} --n {n} --mode {} --max-m {} --max-k {} --max-n {}",
        mode.to_ascii_lowercase(),
        max.0,
        max.1,
        max.2
    )
}

/// Generate `n` jobs in one mode with the vec generator and rebuild them.
pub fn generate(seed: u64, n: usize, mode: u8, max: (usize, usize, usize)) -> Result<Vec<BatchJob>, String> {
    let meta = splitsys_vec::Meta::test();
    let files = splitsys_vec::jobs::gemm(seed, n, mode, max.0, max.1, max.2, &meta)?;
    let doc: Value = serde_json::from_slice(&files.first().ok_or("vec gemm produced no file")?.bytes)
        .map_err(|e| e.to_string())?;
    let jobs = doc["jobs"].as_array().ok_or("vec gemm: no jobs array")?;
    let mut out = Vec::with_capacity(jobs.len());
    for (i, j) in jobs.iter().enumerate() {
        let (m, k, nn) = (u(j, "M")?, u(j, "K")?, u(j, "N")?);
        let requant = j["requant"].as_bool().ok_or("requant")?;
        let mut a_img = None;
        let mut w_img = None;
        for l in j["load"].as_array().ok_or("load")? {
            let bytes = unhex(l["hex"].as_str().ok_or("hex")?)?;
            match l["region"].as_u64() {
                Some(r) if r == u64::from(spec::REGION_ABUF) => a_img = Some(bytes),
                Some(r) if r == u64::from(spec::REGION_WBUF) => w_img = Some(bytes),
                _ => {}
            }
        }
        let a = golden::unpack_a_blocks8(&a_img.ok_or("no ABUF load")?, m, k);
        let w = golden::unpack_w_blocks16(&w_img.ok_or("no WBUF load")?, k, nn);
        let rq = if requant {
            Some(
                j["rq"]
                    .as_array()
                    .ok_or("rq")?
                    .iter()
                    .map(|p| (p[0].as_u64().unwrap_or(0) as u16, p[1].as_u64().unwrap_or(0) as u8))
                    .collect(),
            )
        } else {
            None
        };
        let d = &j["desc"];
        let bases = golden::Bases {
            a: u(d, "a_base")? as u32,
            w: u(d, "w_base")? as u32,
            out: u(d, "out_base")? as u32,
            rq: u(d, "rq_base")? as u16,
        };
        let job = golden::Job::new(mode, a, w, rq, bases).map_err(|e| format!("job {i}: {e}"))?;
        let desc_hex = j["desc_hex"].as_str().unwrap_or("").to_string();
        // The rebuilt job must be the generator's job, byte for byte.
        if unhex(&desc_hex)? != job.desc.to_bytes() {
            return Err(format!("job {i}: rebuilt descriptor differs from splitsys-vec"));
        }
        if unhex(j["expected"]["obuf_hex"].as_str().unwrap_or(""))? != job.obuf_image {
            return Err(format!("job {i}: rebuilt expected OBUF differs from splitsys-vec"));
        }
        let name = j["name"].as_str().unwrap_or("gemm").to_string();
        let mode_s = if mode == spec::MODE_STRIP { "STRIP" } else { "FULL" };
        out.push(BatchJob {
            spec: GemmSpec {
                name: name.clone(),
                m,
                k,
                n: nn,
                mode: mode_s.into(),
                requant,
                seed,
                mode_select: None,
            },
            name,
            index: i,
            job,
            desc_hex,
        });
    }
    Ok(out)
}

/// `run --random N --mode auto`: the FULL-mode generator's jobs (operands, rq
/// params, bases), each re-targeted to the mode `splitsys_cycle::choose_mode`
/// picks for its shape. Jobs whose pick is FULL are the generator's job byte for
/// byte; STRIP picks are rebuilt with golden from the same operands and bases.
pub fn generate_auto(seed: u64, n: usize, max: (usize, usize, usize)) -> Result<Vec<BatchJob>, String> {
    let src_cmd = vec_command(seed, n, "full", max);
    let mut out = generate(seed, n, spec::MODE_FULL, max)?;
    for bj in &mut out {
        let mut sel = crate::runner::select_mode(bj.spec.m, bj.spec.k, bj.spec.n, bj.spec.requant)?;
        sel.operands_from = Some(format!("{} (job {} of `{src_cmd}`)", bj.name, bj.index));
        if sel.chosen == "STRIP" {
            let d = &bj.job.desc;
            let bases = golden::Bases { a: d.a_base, w: d.w_base, out: d.out_base, rq: d.rq_base };
            let rq = (!bj.job.rq.is_empty()).then(|| bj.job.rq.clone());
            let job = golden::Job::new(spec::MODE_STRIP, bj.job.a.clone(), bj.job.w.clone(), rq, bases)
                .map_err(|e| format!("auto job {}: STRIP rebuild: {e}", bj.index))?;
            bj.desc_hex = job.desc.to_bytes().iter().map(|b| format!("{b:02x}")).collect();
            bj.job = job;
        }
        bj.name = format!("gemm_auto_s{seed}_{}", bj.index);
        bj.spec.name = bj.name.clone();
        bj.spec.mode = sel.chosen.clone();
        bj.spec.mode_select = Some(sel);
    }
    Ok(out)
}

pub const AUTO_POLICY_SHORT: &str = "model-scored: choose_mode, min cycles_total, ties -> FULL";

/// Model-scored totals for a set of auto runs (`--mode auto`). Only the chosen
/// mode was run: `measured_cycles` sums its measured `cycles_total`; the
/// always-FULL / always-STRIP totals are MODEL predictions, never measurements.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AutoTally {
    pub runs: u64,
    pub picked_full: u64,
    pub picked_strip: u64,
    /// Auto runs where the model says STRIP needs more cycles than FULL (auto ran FULL).
    pub strip_slower: u64,
    /// Of those, how many have more STRIP tiles than FULL tiles.
    pub strip_slower_more_tiles: u64,
    /// Auto runs where the model predicts equal cycles (auto ran FULL: ties -> FULL).
    pub ties: u64,
    /// Sum of measured cycles_total of the runs actually made (chosen mode).
    pub measured_cycles: u64,
    /// Runs whose measured cycles_total equals the model's prediction for the chosen mode.
    pub measured_eq_model: u64,
    pub model_cycles_auto: u64,
    pub model_cycles_full: u64,
    pub model_cycles_strip: u64,
}

impl AutoTally {
    pub fn add(&mut self, r: &RunRecord) {
        let Some(sel) = &r.mode_select else { return };
        self.runs += 1;
        if sel.chosen == "STRIP" {
            self.picked_strip += 1;
        } else {
            self.picked_full += 1;
        }
        if sel.model_cycles_strip > sel.model_cycles_full {
            self.strip_slower += 1;
            self.strip_slower_more_tiles += u64::from(sel.model_tiles_strip > sel.model_tiles_full);
        }
        self.ties += u64::from(sel.model_cycles_strip == sel.model_cycles_full);
        self.measured_cycles += r.counters.cycles_total;
        self.measured_eq_model += u64::from(r.counters.cycles_total == sel.model_cycles_chosen());
        self.model_cycles_auto += sel.model_cycles_chosen();
        self.model_cycles_full += sel.model_cycles_full;
        self.model_cycles_strip += sel.model_cycles_strip;
    }

    /// The printed summary. `src` labels the measured line (HW / SIM / MODEL).
    pub fn lines(&self, src: &str) -> Vec<String> {
        let lab = src.to_ascii_uppercase();
        let x = |t: u64| if self.model_cycles_auto == 0 { 0.0 } else { t as f64 / self.model_cycles_auto as f64 };
        vec![
            format!(
                "AUTO: {} GEMMs, picked FULL {} / STRIP {} ({AUTO_POLICY_SHORT}); model says STRIP slower on {} ({} with more STRIP tiles) -> auto ran FULL there; {} ties -> FULL",
                self.runs, self.picked_full, self.picked_strip, self.strip_slower, self.strip_slower_more_tiles, self.ties
            ),
            format!(
                "AUTO cycles, measured ({lab}, chosen mode only): {} total; measured == model on {}/{} runs",
                self.measured_cycles, self.measured_eq_model, self.runs
            ),
            format!(
                "AUTO cycles, MODEL totals (un-run modes are never measured): auto {} | always-FULL {} ({:.3}x auto) | always-STRIP {} ({:.3}x auto)",
                self.model_cycles_auto,
                self.model_cycles_full,
                x(self.model_cycles_full),
                self.model_cycles_strip,
                x(self.model_cycles_strip)
            ),
        ]
    }
}

#[derive(Debug, Default, Clone)]
pub struct BatchSummary {
    pub seed: u64,
    pub runs: u64,
    /// Mismatching in-bounds outputs, summed over runs.
    pub mismatches: u64,
    /// Runs whose counters differ from the model in any of the 12 counters.
    pub counter_mismatches: u64,
    pub invariant_failures: u64,
    pub errors: u64,
    /// First failing run: description (name, descriptor, seed, sim command, reason).
    pub first_failure: Option<String>,
}

impl BatchSummary {
    pub fn ok(&self) -> bool {
        self.mismatches == 0 && self.counter_mismatches == 0 && self.invariant_failures == 0 && self.errors == 0
    }
    pub fn line(&self) -> String {
        format!(
            "BATCH {}: seed {} runs {} mismatches {} counter_mismatches {} invariant_failures {} errors {}",
            if self.ok() { "PASS" } else { "FAIL" },
            self.seed,
            self.runs,
            self.mismatches,
            self.counter_mismatches,
            self.invariant_failures,
            self.errors
        )
    }
}

/// Run a batch. `modes` in run order per index (e.g. FULL then STRIP).
/// `each` sees every finished run (record or error) for printing.
#[allow(clippy::too_many_arguments)]
pub fn run_batch(
    board: &mut Board,
    ctx: &mut Context,
    seed: u64,
    n: usize,
    modes: &[u8],
    max: (usize, usize, usize),
    mut each: impl FnMut(&BatchJob, &Result<(RunRecord, Option<PathBuf>), RunError>),
) -> Result<BatchSummary, String> {
    let lists: Vec<(u8, Vec<BatchJob>)> = modes
        .iter()
        .map(|&m| generate(seed, n, m, max).map(|v| (m, v)))
        .collect::<Result<_, _>>()?;
    run_lists(board, ctx, seed, n, max, &lists, &mut each)
}

/// `run --random N --mode auto`: one run per job, in the mode the model picks.
pub fn run_batch_auto(
    board: &mut Board,
    ctx: &mut Context,
    seed: u64,
    n: usize,
    max: (usize, usize, usize),
    mut each: impl FnMut(&BatchJob, &Result<(RunRecord, Option<PathBuf>), RunError>),
) -> Result<(BatchSummary, AutoTally), String> {
    // The reproduction command names the FULL generator (the operands' source).
    let lists = vec![(spec::MODE_FULL, generate_auto(seed, n, max)?)];
    let mut tally = AutoTally::default();
    let s = run_lists(board, ctx, seed, n, max, &lists, &mut |bj, r| {
        if let Ok((rec, _)) = r {
            tally.add(rec);
        }
        each(bj, r)
    })?;
    Ok((s, tally))
}

type Each<'a> = dyn FnMut(&BatchJob, &Result<(RunRecord, Option<PathBuf>), RunError>) + 'a;

fn run_lists(
    board: &mut Board,
    ctx: &mut Context,
    seed: u64,
    n: usize,
    max: (usize, usize, usize),
    lists: &[(u8, Vec<BatchJob>)],
    each: &mut Each<'_>,
) -> Result<BatchSummary, String> {
    let mut s = BatchSummary {
        seed,
        ..Default::default()
    };
    let mut cache = LoadCache::default();
    for i in 0..n {
        for (mode, list) in lists {
            let Some(bj) = list.get(i) else { continue };
            let res = run_job(board, ctx, &mut cache, &bj.spec, &bj.job, false);
            s.runs += 1;
            let fail = match &res {
                Ok((r, _)) => {
                    s.mismatches += r.check.mismatches;
                    let cm = !r.check.counter_diffs.is_empty();
                    s.counter_mismatches += u64::from(cm);
                    s.invariant_failures += u64::from(!r.check.invariants_ok);
                    let mut why = Vec::new();
                    if r.check.mismatches > 0 {
                        let at = r.check.first_mismatch.map(|idx| {
                            let (m, n) = ((idx as usize) / bj.spec.n, (idx as usize) % bj.spec.n);
                            let off = if bj.spec.requant {
                                golden::pack::c_i8_offset(bj.spec.m, m, n)
                            } else {
                                golden::pack::c_i32_offset(bj.spec.m, m, n)
                            };
                            crate::diag::locate(&bj.job.desc, off).to_string()
                        });
                        why.push(format!(
                            "{} output mismatches (first: {})",
                            r.check.mismatches,
                            at.unwrap_or_else(|| "?".into())
                        ));
                    }
                    if cm {
                        why.push(format!("counters: {}", r.check.counter_diffs.join("; ")));
                    }
                    if let Some(e) = &r.check.invariant_error {
                        why.push(format!("invariant: {e}"));
                    }
                    (!why.is_empty()).then(|| why.join(" | "))
                }
                Err(e) => {
                    s.errors += 1;
                    Some(format!("error: {e}"))
                }
            };
            if let (Some(why), None) = (fail, &s.first_failure) {
                let ms = if *mode == spec::MODE_STRIP { "strip" } else { "full" };
                let auto = bj.spec.mode_select.as_ref().map_or(String::new(), |_| {
                    format!(" [AUTO chose {}; operands and bases from that FULL job, descriptor re-targeted]", bj.spec.mode)
                });
                s.first_failure = Some(format!(
                    "{} (job {} of `{}`){auto}: M={} K={} N={} {} requant={} desc_hex {} seed {}: {why}",
                    bj.name,
                    bj.index,
                    vec_command(seed, n, ms, max),
                    bj.spec.m,
                    bj.spec.k,
                    bj.spec.n,
                    bj.spec.mode,
                    bj.spec.requant,
                    bj.desc_hex,
                    seed
                ));
            }
            each(bj, &res);
        }
    }
    Ok(s)
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
    fn generated_jobs_match_vec_generator_and_cover_edges() {
        let v = generate(1, 12, spec::MODE_STRIP, (64, 256, 256)).unwrap();
        assert_eq!(v.len(), 12);
        assert_eq!(v[0].spec.m, 1);
        assert_eq!((v[1].spec.m, v[1].spec.k, v[1].spec.n), (64, 256, 256));
        assert!(v.iter().any(|b| b.job.desc.a_base != 0 || b.job.desc.out_base != 0), "random bases used");
    }

    #[test]
    fn mock_batch_both_modes_passes() {
        let mut b = Board::new(Box::new(MockTransport::new(MockConfig::default())));
        let mut c = ctx();
        let mut seen = 0;
        let s = run_batch(&mut b, &mut c, 3, 12, &[spec::MODE_FULL, spec::MODE_STRIP], (64, 256, 256), |_, r| {
            assert!(r.is_ok());
            seen += 1;
        })
        .unwrap();
        assert!(s.ok(), "{s:?}");
        assert_eq!(s.runs, 24);
        assert_eq!(seen, 24);
        assert!(s.line().starts_with("BATCH PASS: seed 3 runs 24"));
    }

    #[test]
    fn mock_auto_batch_runs_each_job_once_in_the_model_pick() {
        let mut b = Board::new(Box::new(MockTransport::new(MockConfig::default())));
        let mut c = ctx();
        let full = generate(1, 16, spec::MODE_FULL, (64, 256, 256)).unwrap();
        let auto = generate_auto(1, 16, (64, 256, 256)).unwrap();
        for (f, a) in full.iter().zip(&auto) {
            let sel = a.spec.mode_select.as_ref().unwrap();
            assert_eq!(
                (a.spec.m, a.spec.k, a.spec.n, a.spec.requant),
                (f.spec.m, f.spec.k, f.spec.n, f.spec.requant)
            );
            assert_eq!(a.job.a_image, f.job.a_image);
            assert_eq!(a.job.w_image, f.job.w_image);
            assert_eq!(a.job.obuf_image, f.job.obuf_image, "output layout is mode-independent");
            assert!(sel.operands_from.as_ref().unwrap().contains(&f.name));
            if sel.chosen == "FULL" {
                assert_eq!(a.desc_hex, f.desc_hex, "FULL picks are the generator's job byte for byte");
            } else {
                assert_eq!(a.job.desc.mode, spec::MODE_STRIP);
                assert_eq!((a.job.desc.a_base, a.job.desc.out_base), (f.job.desc.a_base, f.job.desc.out_base));
            }
        }
        let mut seen = 0;
        let (s, t) = run_batch_auto(&mut b, &mut c, 1, 16, (64, 256, 256), |bj, r| {
            let (rec, _) = r.as_ref().unwrap();
            assert_eq!(rec.mode_select.as_ref(), bj.spec.mode_select.as_ref());
            assert_eq!(rec.gemm.mode, bj.spec.mode);
            seen += 1;
        })
        .unwrap();
        assert!(s.ok(), "{s:?}");
        assert_eq!((s.runs, seen, t.runs), (16, 16, 16), "one run per job, not one per mode");
        assert_eq!(t.picked_full + t.picked_strip, 16);
        assert!(t.picked_full > 0 && t.picked_strip > 0, "{t:?}");
        assert_eq!(t.measured_eq_model, 16);
        assert_eq!(t.measured_cycles, t.model_cycles_auto);
        assert!(t.model_cycles_auto <= t.model_cycles_full && t.model_cycles_auto <= t.model_cycles_strip);
        assert_eq!(t.strip_slower, t.strip_slower_more_tiles);
        let l = t.lines("model");
        assert!(l[0].starts_with("AUTO: 16 GEMMs, picked FULL"), "{}", l[0]);
        assert!(l[1].contains("measured (MODEL, chosen mode only)"), "{}", l[1]);
        assert!(l[2].contains("MODEL totals"), "{}", l[2]);
    }

    #[test]
    fn fault_injected_mock_fails_with_reproduction_info() {
        // A memory fault: one bit flipped in the middle of every OBUF read-back.
        // (Job 0 is 1x1x1, whose middle byte is padding: correctly not counted.)
        let mut c = ctx();
        let mut b = Board::new(Box::new(MockTransport::new(MockConfig {
            flip_read_byte: 1000,
            ..Default::default()
        })));
        let s = run_batch(&mut b, &mut c, 5, 4, &[spec::MODE_FULL], (16, 32, 32), |_, _| {}).unwrap();
        assert!(!s.ok());
        assert!(s.mismatches >= 1, "{s:?}");
        let f = s.first_failure.clone().unwrap();
        assert!(f.contains("gemm_full_s5_1 (job 1 of"), "{f}");
        assert!(f.contains("splitsys-vec gemm --seed 5 --n 4 --mode full"), "{f}");
        assert!(f.contains("desc_hex"), "{f}");
        assert!(s.line().starts_with("BATCH FAIL"));
    }

    #[test]
    fn counter_diffs_are_reported() {
        let m = splitsys_cycle::Counters { cycles_total: 100, tiles: 1, ..Default::default() };
        let mut h = m;
        h.cycles_total = 101;
        let d = crate::runner::counter_diffs(&h, &m);
        assert_eq!(d, vec!["cycles_total: 101 vs model 100 (+1.00%)".to_string()]);
        assert!(crate::runner::counter_diffs(&m, &m).is_empty());
    }
}
