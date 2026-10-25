//! G5 stretch: run the public tiny LM (karpathy/tinyllamas stories260K) with every
//! linear layer executed on the board as an int8 GEMM (M = 1, int32 bypass).
//!
//! Split of work (stated in every record and on Scene 4): the linear layers
//! (q, k, v, o, w1, w2, w3 per layer and the classifier) run on the FPGA; token
//! embedding, RMSNorm, RoPE, attention scores and softmax, SwiGLU, dequant and
//! sampling run on the laptop (crate `splitsys_lm`).
//!
//! Every call is checked like `run`: the int32 OBUF read-back must equal the
//! call's `expected_acc` bit for bit (abort on the first mismatch), the 12
//! counters are compared with `splitsys_cycle::predict`, and the SPEC §3.7
//! invariants are checked. The board's output (not the expectation) is what
//! the generator consumes.
//!
//! WBUF plan: the whole model (~260 KB of int8 weights) does not fit in the
//! 64 KB WBUF, and every token walks the matrices in the same order, so an LRU
//! would never hit. Instead a fixed subset is *pinned* (written once, verified
//! by content hash on every use) and the rest stream through one scratch slot.
//! ABUF is reused when the activation image is unchanged (q/k/v and w1/w3 share
//! an input).
//!
//! Records go to `results/<source>/lm/` (a subdirectory, so the per-GEMM run
//! records and session totals are unaffected): one JSON per generation,
//! rewritten after every token.

use crate::client::Board;
use crate::record::{iso_utc, now_unix, runs_dir, write_json};
use crate::runner::{self, effective_source, Context, GemmSpec, MOCK_BUILD_ID_HEX};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use splitsys_cycle as cycle;
use splitsys_golden as golden;
use splitsys_spec as spec;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// What runs where. Shown verbatim on Scene 4 and stored in every record.
pub const SPLIT_FPGA: &str =
    "linear layers on the FPGA: q, k, v, o, w1, w2, w3 in every layer and the classifier (int8 x int8 -> int32, M = 1)";
pub const SPLIT_LAPTOP: &str =
    "on the laptop: token embedding, RMSNorm, RoPE, attention scores + softmax, SwiGLU, int8 quantize/dequantize, greedy sampling";
/// The hardware wall-time caveat, with the UART rate the record was made at.
pub fn wall_note_hw(transport: &str) -> String {
    let rate = crate::transport::baud_of(transport).map_or_else(String::new, |b| format!(" at {b} baud"));
    format!("laptop wall time including UART{rate}; NOT accelerator speed (on-chip cycles come from the counters)")
}
pub const WALL_NOTE_MOCK: &str =
    "laptop wall time with the in-process mock board (no UART); NOT accelerator speed (on-chip cycles come from the model counters)";
pub const WALL_NOTE_SIM: &str =
    "laptop wall time against the RTL co-simulation (tcp); NOT accelerator speed (on-chip cycles come from the counters)";

/// The wall-time caveat for a record's source; `transport` as in the record
/// (`serial:COM3@1562500`) supplies the baud for hardware records.
pub fn wall_note(source: &str, mock: bool, transport: &str) -> String {
    match (source, mock) {
        (_, true) | ("model", _) => WALL_NOTE_MOCK.into(),
        ("sim", _) => WALL_NOTE_SIM.into(),
        _ => wall_note_hw(transport),
    }
}
pub const NUMERICS: &str =
    "int8 weights per output column, int8 activations per call (per tensor), int32 bypass accumulators, host dequant + nonlinearities in f32, greedy decoding";

/// The record format version (bump on breaking changes).
pub const LM_SCHEMA: u32 = 1;
pub const LM_KIND: &str = "splitsys-lm-generation";

/// One linear call as the generator hands it to the executor.
#[derive(Clone, Copy, Debug)]
pub struct CallView<'a> {
    pub layer: Option<usize>,
    pub name: &'a str,
    pub job: &'a golden::Job,
    /// Row-major M x N int32 accumulators the board must reproduce exactly.
    pub expected_acc: &'a [i32],
}

/// A weight matrix identity: (layer, name). Content is verified by hash on use.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WKey {
    pub layer: Option<usize>,
    pub name: String,
}

impl WKey {
    pub fn label(&self) -> String {
        match self.layer {
            Some(l) => format!("L{l}.{}", self.name),
            None => self.name.clone(),
        }
    }
}

pub fn fnv1a64(b: &[u8]) -> u64 {
    b.iter()
        .fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ u64::from(x)).wrapping_mul(0x100_0000_01b3))
}

// ---------------------------------------------------------------- WBUF plan

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedSlot {
    pub key: WKey,
    /// WBUF word base (16-byte words).
    pub w_base: u32,
    pub bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WbufPlan {
    pub pinned: Vec<PinnedSlot>,
    pub scratch_base: u32,
    pub scratch_bytes: usize,
    pub pinned_bytes: usize,
    /// Weight bytes that must be sent every step (the non-pinned matrices).
    pub streamed_bytes_per_step: usize,
    /// All weight bytes one step uses (pinned + streamed).
    pub total_bytes_per_step: usize,
    pub capacity_bytes: usize,
    pub rule: String,
}

impl WbufPlan {
    /// Choose the pinned set for a step's matrices (call order, duplicates
    /// allowed). For each candidate scratch size (every distinct matrix size),
    /// matrices larger than the scratch must be pinned; the rest are pinned
    /// largest-first while they fit. The plan pinning the most bytes wins
    /// (ties: the earlier candidate, i.e. the smaller scratch).
    pub fn new(items: &[(WKey, usize)], capacity: usize) -> Result<Self, String> {
        let mut uniq: Vec<(WKey, usize)> = Vec::new();
        for (k, b) in items {
            match uniq.iter().find(|(u, _)| u == k) {
                Some((_, ub)) if ub != b => return Err(format!("{} appears with sizes {ub} and {b}", k.label())),
                Some(_) => {}
                None => uniq.push((k.clone(), *b)),
            }
        }
        if uniq.iter().any(|(_, b)| b % spec::WBUF_WORD_BYTES != 0) {
            return Err("W images must be whole WBUF words".into());
        }
        let total: usize = uniq.iter().map(|(_, b)| b).sum();
        let mut sizes: Vec<usize> = uniq.iter().map(|(_, b)| *b).collect();
        sizes.push(0);
        sizes.sort_unstable();
        sizes.dedup();
        let mut best: Option<(usize, usize, Vec<usize>)> = None; // (pinned bytes, scratch, idx)
        for &s in &sizes {
            if s > capacity {
                continue;
            }
            let mut room = capacity - s;
            let mut pin: Vec<usize> = Vec::new();
            let mut ok = true;
            for (i, (_, b)) in uniq.iter().enumerate() {
                if *b > s {
                    if *b > room {
                        ok = false;
                        break;
                    }
                    room -= b;
                    pin.push(i);
                }
            }
            if !ok {
                continue;
            }
            let mut rest: Vec<usize> = (0..uniq.len()).filter(|i| uniq[*i].1 <= s).collect();
            rest.sort_by(|a, b| uniq[*b].1.cmp(&uniq[*a].1).then(a.cmp(b)));
            for i in rest {
                if uniq[i].1 <= room {
                    room -= uniq[i].1;
                    pin.push(i);
                }
            }
            // The scratch must still hold every non-pinned matrix.
            let max_unpinned = (0..uniq.len()).filter(|i| !pin.contains(i)).map(|i| uniq[i].1).max().unwrap_or(0);
            let scratch = max_unpinned;
            let pinned: usize = pin.iter().map(|&i| uniq[i].1).sum();
            if pinned + scratch > capacity {
                continue;
            }
            if best.as_ref().is_none_or(|(p, _, _)| pinned > *p) {
                pin.sort_unstable();
                best = Some((pinned, scratch, pin));
            }
        }
        let (pinned_bytes, scratch_bytes, pin) =
            best.ok_or_else(|| format!("no WBUF plan: a matrix exceeds the {capacity}-byte WBUF"))?;
        let mut base = 0usize;
        let mut pinned = Vec::new();
        for &i in &pin {
            pinned.push(PinnedSlot { key: uniq[i].0.clone(), w_base: (base / spec::WBUF_WORD_BYTES) as u32, bytes: uniq[i].1 });
            base += uniq[i].1;
        }
        // Streamed bytes per step count every occurrence (a repeated unpinned
        // matrix is resent unless it is still in the scratch slot).
        let mut streamed = 0usize;
        let mut last: Option<&WKey> = None;
        for (k, b) in items {
            if !pin.iter().any(|&i| &uniq[i].0 == k) && last != Some(k) {
                streamed += b;
            }
            last = Some(k);
        }
        Ok(Self {
            pinned,
            scratch_base: (base / spec::WBUF_WORD_BYTES) as u32,
            scratch_bytes,
            pinned_bytes,
            streamed_bytes_per_step: streamed,
            total_bytes_per_step: total,
            capacity_bytes: capacity,
            rule: "pin the matrix set that maximises resident bytes, leaving one scratch slot as large as the largest unpinned matrix; pinned matrices are written once and content-hash checked on every use".into(),
        })
    }

    fn slot(&self, k: &WKey) -> Option<&PinnedSlot> {
        self.pinned.iter().find(|p| &p.key == k)
    }
}

// ---------------------------------------------------------------- per-call checks

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CallCheck {
    /// Call index within the step.
    pub idx: usize,
    pub layer: Option<usize>,
    pub name: String,
    #[serde(rename = "M")]
    pub m: usize,
    #[serde(rename = "K")]
    pub k: usize,
    #[serde(rename = "N")]
    pub n: usize,
    /// FULL | STRIP (as run).
    pub mode: String,
    /// The mode was picked by the cycle model (`--mode auto`).
    pub auto: bool,
    pub bit_exact: bool,
    pub outputs_compared: u64,
    pub mismatches: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_mismatch: Option<String>,
    pub counters_eq_model: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub counter_diffs: Vec<String>,
    pub invariants_ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invariant_error: Option<String>,
    /// All 12 counters as read from the board.
    pub counters: cycle::Counters,
    pub model_cycles_total: u64,
    /// The weight image was sent for this call (false: resident in WBUF).
    pub w_sent: bool,
    pub a_sent: bool,
    pub bytes_tx: u64,
    pub bytes_rx: u64,
    pub wall_ms: f64,
}

impl CallCheck {
    pub fn ok(&self) -> bool {
        self.bit_exact && self.counters_eq_model && self.invariants_ok
    }
}

/// `--mode` for the LM: one fixed mode, or the cycle model's pick per call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LmMode {
    Full,
    Strip,
    Auto,
}

impl LmMode {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "full" => Ok(Self::Full),
            "strip" => Ok(Self::Strip),
            "auto" => Ok(Self::Auto),
            m => Err(format!("--mode {m}: expected full | strip | auto")),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "FULL",
            Self::Strip => "STRIP",
            Self::Auto => "AUTO",
        }
    }
}

/// Runs linear calls on a board with the WBUF plan, checking each one.
pub struct Executor {
    pub mode: LmMode,
    pub plan: WbufPlan,
    /// Pinned matrices written so far, with the content hash written.
    pinned_written: HashMap<WKey, u64>,
    scratch: Option<(WKey, u64)>,
    a_resident: Option<Vec<u8>>,
    pub poison: bool,
    /// Step (token) index and call index, for error messages.
    pub step: usize,
    pub calls: Vec<CallCheck>,
    pub w_bytes_sent: u64,
    pub w_bytes_resident: u64,
    checked_idle: bool,
}

impl Executor {
    pub fn new(mode: LmMode, plan: WbufPlan) -> Self {
        Self {
            mode,
            plan,
            pinned_written: HashMap::new(),
            scratch: None,
            a_resident: None,
            poison: true,
            step: 0,
            calls: Vec::new(),
            w_bytes_sent: 0,
            w_bytes_resident: 0,
            checked_idle: false,
        }
    }

    /// Forget what is resident (e.g. after a reconnect).
    pub fn invalidate(&mut self) {
        self.pinned_written.clear();
        self.scratch = None;
        self.a_resident = None;
        self.checked_idle = false;
    }

    fn resolve_mode(&self, m: usize, k: usize, n: usize) -> Result<(u8, bool), String> {
        Ok(match self.mode {
            LmMode::Full => (spec::MODE_FULL, false),
            LmMode::Strip => (spec::MODE_STRIP, false),
            LmMode::Auto => {
                let s = runner::select_mode(m, k, n, false)?;
                (runner::mode_code(&s.chosen)?, true)
            }
        })
    }

    /// Run one call on the board. Returns the board's int32 outputs (row-major
    /// M x N). Any mismatch, counter difference or invariant failure is an error
    /// naming the step, call, layer and matrix.
    pub fn exec(&mut self, board: &mut Board, call: &CallView<'_>) -> Result<Vec<i32>, String> {
        let t0 = Instant::now();
        let (tx0, rx0) = (board.stats.bytes_tx, board.stats.bytes_rx);
        let idx = self.calls.len();
        let key = WKey { layer: call.layer, name: call.name.to_string() };
        let at = |what: &str| format!("step {} call #{idx} {} ({what})", self.step, key.label());
        let src = call.job;
        let (m, k, n) = src.desc.dims();
        if src.c.is_some() {
            return Err(at("expected an int32 bypass job"));
        }
        if call.expected_acc.len() != m * n {
            return Err(at(&format!("expected_acc has {} values, want M*N = {}", call.expected_acc.len(), m * n)));
        }
        // The lm crate's expectation must agree with the golden GEMM of its operands.
        for i in 0..m {
            for j in 0..n {
                if src.acc.get(i, j) != call.expected_acc[i * n + j] {
                    return Err(at(&format!(
                        "expected_acc[{i},{j}] = {} disagrees with golden gemm_i32 = {}",
                        call.expected_acc[i * n + j],
                        src.acc.get(i, j)
                    )));
                }
            }
        }
        let (mode, auto) = self.resolve_mode(m, k, n)?;
        // Re-target the job at our WBUF slot (images are base-independent).
        let wh = fnv1a64(&src.w_image);
        let (w_base, pinned) = match self.plan.slot(&key) {
            Some(s) if s.bytes == src.w_image.len() => (s.w_base, true),
            _ => (self.plan.scratch_base, false),
        };
        if !pinned && src.w_image.len() > self.plan.scratch_bytes {
            return Err(at(&format!(
                "W image {} B exceeds the scratch slot {} B (plan built from another model?)",
                src.w_image.len(),
                self.plan.scratch_bytes
            )));
        }
        let job = golden::Job::new(
            mode,
            src.a.clone(),
            src.w.clone(),
            None,
            golden::Bases { a: 0, w: w_base, out: 0, rq: 0 },
        )
        .map_err(|e| at(&format!("descriptor: {e}")))?;
        let e = |x: crate::client::HostError| at(&x.to_string());
        if !self.checked_idle {
            if board.poll().map_err(e)? == spec::POLL_BUSY {
                board.wait_done(Duration::from_secs(5)).map_err(e)?;
            }
            self.checked_idle = true;
        }
        // W: skip if this exact image is resident at this base.
        let resident = if pinned {
            self.pinned_written.get(&key) == Some(&wh)
        } else {
            self.scratch.as_ref() == Some(&(key.clone(), wh))
        };
        let w_sent = !resident;
        if w_sent {
            board.write_mem(spec::REGION_WBUF, job.w_byte_addr(), &job.w_image).map_err(e)?;
            self.w_bytes_sent += job.w_image.len() as u64;
            if pinned {
                self.pinned_written.insert(key.clone(), wh);
            } else {
                self.scratch = Some((key.clone(), wh));
            }
        } else {
            self.w_bytes_resident += job.w_image.len() as u64;
        }
        let a_sent = self.a_resident.as_deref() != Some(&job.a_image[..]);
        if a_sent {
            board.write_mem(spec::REGION_ABUF, job.a_byte_addr(), &job.a_image).map_err(e)?;
            self.a_resident = Some(job.a_image.clone());
        }
        // Expected OBUF bytes come from the lm crate's expected_acc.
        let mut want = vec![0u8; job.obuf_image.len()];
        for i in 0..m {
            for j in 0..n {
                let off = golden::pack::c_i32_offset(m, i, j);
                want[off..off + 4].copy_from_slice(&call.expected_acc[i * n + j].to_le_bytes());
            }
        }
        let ranges = job.compare_ranges();
        if self.poison {
            // Complement of the expectation on exactly the compared bytes, so a stale
            // or unwritten byte can never pass.
            for &(o, l) in &ranges {
                let p: Vec<u8> = want[o..o + l].iter().map(|b| !b).collect();
                board.write_mem(spec::REGION_OBUF, job.out_byte_addr() + o, &p).map_err(e)?;
            }
        }
        board.run_gemm(&job.desc.to_bytes()).map_err(e)?;
        board.wait_done(Duration::from_secs(10)).map_err(e)?;
        let counters = board.read_counters().map_err(e)?;
        let mut got = want.iter().map(|b| !b).collect::<Vec<u8>>();
        for &(o, l) in &ranges {
            let r = board.read_mem(spec::REGION_OBUF, job.out_byte_addr() + o, l).map_err(e)?;
            got[o..o + l].copy_from_slice(&r);
        }
        let mut out = vec![0i32; m * n];
        let mut mismatches = 0u64;
        let mut first: Option<String> = None;
        for i in 0..m {
            for j in 0..n {
                let off = golden::pack::c_i32_offset(m, i, j);
                let v = i32::from_le_bytes([got[off], got[off + 1], got[off + 2], got[off + 3]]);
                out[i * n + j] = v;
                let w = call.expected_acc[i * n + j];
                if v != w {
                    mismatches += 1;
                    first.get_or_insert_with(|| format!("C[{i},{j}] board {v} expected {w}"));
                }
            }
        }
        let mode_s = if mode == spec::MODE_STRIP { "STRIP" } else { "FULL" };
        let g = GemmSpec {
            name: key.label(),
            m,
            k,
            n,
            mode: mode_s.into(),
            requant: false,
            seed: 0,
            mode_select: None,
        };
        let (cd, model) = runner::model_for(&g)?;
        let inv = counters.check_invariants(&cd, &cycle::Latencies::default());
        let diffs = runner::counter_diffs(&counters, &model);
        let chk = CallCheck {
            idx,
            layer: call.layer,
            name: call.name.to_string(),
            m,
            k,
            n,
            mode: mode_s.into(),
            auto,
            bit_exact: mismatches == 0,
            outputs_compared: (m * n) as u64,
            mismatches,
            first_mismatch: first.clone(),
            counters_eq_model: diffs.is_empty(),
            counter_diffs: diffs.clone(),
            invariants_ok: inv.is_ok(),
            invariant_error: inv.clone().err(),
            counters,
            model_cycles_total: model.cycles_total,
            w_sent,
            a_sent,
            bytes_tx: board.stats.bytes_tx - tx0,
            bytes_rx: board.stats.bytes_rx - rx0,
            wall_ms: t0.elapsed().as_secs_f64() * 1000.0,
        };
        self.calls.push(chk);
        if mismatches > 0 {
            return Err(at(&format!(
                "{m}x{k}x{n} {mode_s}: OUTPUT MISMATCH, {mismatches} of {} int32 outputs differ; first {}",
                m * n,
                first.unwrap_or_default()
            )));
        }
        if let Err(ie) = inv {
            return Err(at(&format!("{m}x{k}x{n} {mode_s}: invariant failed: {ie}")));
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- generation

/// A tiny LM the host can drive. Implemented for `splitsys_lm` (stories260K) and
/// by a small fake in tests.
pub trait LmBackend {
    fn info(&self) -> LmModelInfo;
    /// Prompt tokens including BOS.
    fn encode(&self, prompt: &str) -> Vec<u32>;
    fn decode(&self, prev: u32, tok: u32) -> String;
    fn new_state(&self) -> Result<Box<dyn LmState + '_>, String>;
}

pub trait LmState {
    /// One forward step at the next position; every linear goes through `exec`.
    fn step(
        &mut self,
        token: u32,
        exec: &mut dyn FnMut(&CallView<'_>) -> Result<Vec<i32>, String>,
    ) -> Result<Vec<f32>, String>;
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LmModelInfo {
    pub name: String,
    pub dim: usize,
    pub hidden_dim: usize,
    pub n_layers: usize,
    pub n_heads: usize,
    pub n_kv_heads: usize,
    pub vocab_size: usize,
    pub seq_len: usize,
    /// Model files with size and FNV-1a 64 content hash (provenance).
    pub files: Vec<FileInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: String,
    pub bytes: u64,
    pub fnv1a64: String,
}

impl FileInfo {
    pub fn of(path: &Path, display: &str) -> Result<Self, String> {
        let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Self { path: display.into(), bytes: b.len() as u64, fnv1a64: format!("{:016x}", fnv1a64(&b)) })
    }
}

/// Greedy sampling: `splitsys_lm::argmax` (run.c `sample_argmax`).
pub fn argmax(v: &[f32]) -> u32 {
    splitsys_lm::argmax(v)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StepSummary {
    pub gemms: u64,
    pub bit_exact: u64,
    pub counters_eq_model: u64,
    pub invariants_ok: u64,
    pub cycles_total: u64,
    pub mac_useful: u64,
    pub modes: HashMap<String, u64>,
    pub w_sent: u64,
    pub w_resident: u64,
    pub bytes_tx: u64,
    pub bytes_rx: u64,
    pub wall_ms: f64,
}

impl StepSummary {
    fn from_calls(c: &[CallCheck], wall_ms: f64) -> Self {
        let mut s = Self { wall_ms, ..Default::default() };
        for x in c {
            s.gemms += 1;
            s.bit_exact += u64::from(x.bit_exact);
            s.counters_eq_model += u64::from(x.counters_eq_model);
            s.invariants_ok += u64::from(x.invariants_ok);
            s.cycles_total += x.counters.cycles_total;
            s.mac_useful += x.counters.mac_useful;
            *s.modes.entry(x.mode.clone()).or_default() += 1;
            s.w_sent += u64::from(x.w_sent);
            s.w_resident += u64::from(!x.w_sent);
            s.bytes_tx += x.bytes_tx;
            s.bytes_rx += x.bytes_rx;
        }
        s
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepRecord {
    /// Step index = position in the sequence.
    pub index: usize,
    /// "prompt" (consumes a prompt token; output forced) | "generate".
    pub phase: String,
    pub input_token: u32,
    /// The token chosen after this step (prompt: the next prompt token).
    pub output_token: u32,
    /// Decoded text of `output_token` (empty for prompt steps' forced tokens is not used).
    pub piece: String,
    /// Prompt plus generated text after this step.
    pub text_so_far: String,
    pub utc: String,
    pub summary: StepSummary,
    pub calls: Vec<CallCheck>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GenSummary {
    pub steps: usize,
    pub prompt_steps: usize,
    pub tokens_generated: usize,
    pub gemms: u64,
    pub bit_exact: u64,
    pub counters_eq_model: u64,
    pub invariants_ok: u64,
    pub mac_useful: u64,
    pub cycles_total: u64,
    pub bytes_tx: u64,
    pub bytes_rx: u64,
    pub w_bytes_sent: u64,
    pub w_bytes_resident: u64,
    pub wall_s: f64,
    /// Mean over all steps (prompt and generated).
    pub wall_s_per_step: f64,
    /// Mean on-chip cycles per step, from the counters.
    pub cycles_per_step: f64,
    /// Mean bytes sent per step (excluding the first step, which loads the pinned set).
    pub bytes_tx_per_step_steady: f64,
    pub wall_note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LmRecord {
    pub kind: String,
    pub schema: u32,
    pub gen_id: String,
    /// hw | sim | model (mock board)
    pub source: String,
    pub mock: bool,
    pub build_id: String,
    /// splitsys-host build (HEAD at build time; the lm crate is compiled in).
    pub git_hash: String,
    pub model_git_hash: String,
    pub transport: String,
    pub started_utc: String,
    pub updated_utc: String,
    /// running | done | aborted
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub prompt: String,
    pub prompt_tokens: Vec<u32>,
    pub max_new_tokens: usize,
    pub mode_requested: String,
    pub model: LmModelInfo,
    pub numerics: String,
    pub split: Value,
    pub wbuf_plan: WbufPlan,
    /// Generated continuation only.
    pub generated_text: String,
    /// Prompt + generated text.
    pub text: String,
    pub summary: GenSummary,
    /// Stopped early by the operator (not an error).
    #[serde(default)]
    pub stopped: bool,
    pub steps: Vec<StepRecord>,
}

impl LmRecord {
    pub fn verified(&self) -> bool {
        self.status == "done"
            && self.summary.gemms > 0
            && self.summary.bit_exact == self.summary.gemms
            && self.summary.counters_eq_model == self.summary.gemms
            && self.summary.invariants_ok == self.summary.gemms
    }
}

/// Where LM records live for a source: `results/<source dir>/lm/`.
pub fn lm_dir(results: &Path, source: &str) -> PathBuf {
    runs_dir(results, source).join("lm")
}

/// Required keys and types of an LM record.
pub fn validate_lm_json(v: &Value) -> Result<(), String> {
    let need = |o: &Value, k: &str| o.get(k).cloned().ok_or_else(|| format!("missing {k}"));
    if v["kind"] != LM_KIND {
        return Err(format!("kind must be {LM_KIND}"));
    }
    for k in ["gen_id", "source", "build_id", "git_hash", "model_git_hash", "status", "prompt", "text", "generated_text", "mode_requested", "numerics"] {
        need(v, k)?.as_str().ok_or_else(|| format!("{k} must be a string"))?;
    }
    if !["hw", "sim", "model"].contains(&v["source"].as_str().unwrap_or("")) {
        return Err("source must be hw | sim | model".into());
    }
    if !["running", "done", "aborted"].contains(&v["status"].as_str().unwrap_or("")) {
        return Err("status must be running | done | aborted".into());
    }
    need(v, "mock")?.as_bool().ok_or("mock must be a bool")?;
    need(v, "prompt_tokens")?.as_array().ok_or("prompt_tokens must be an array")?;
    for k in ["fpga", "laptop"] {
        v["split"][k].as_str().ok_or_else(|| format!("split.{k} must be a string"))?;
    }
    for k in ["gemms", "bit_exact", "counters_eq_model", "bytes_tx", "cycles_total"] {
        v["summary"][k].as_u64().ok_or_else(|| format!("summary.{k} must be an integer"))?;
    }
    v["summary"]["wall_note"].as_str().ok_or("summary.wall_note must be a string")?;
    let steps = need(v, "steps")?;
    let steps = steps.as_array().ok_or("steps must be an array")?;
    let mut gemms = 0u64;
    for (i, s) in steps.iter().enumerate() {
        let calls = s["calls"].as_array().ok_or_else(|| format!("steps[{i}].calls must be an array"))?;
        if s["summary"]["gemms"].as_u64() != Some(calls.len() as u64) {
            return Err(format!("steps[{i}].summary.gemms != calls"));
        }
        s["output_token"].as_u64().ok_or_else(|| format!("steps[{i}].output_token"))?;
        s["text_so_far"].as_str().ok_or_else(|| format!("steps[{i}].text_so_far"))?;
        for (j, c) in calls.iter().enumerate() {
            for k in ["bit_exact", "counters_eq_model", "invariants_ok", "w_sent"] {
                c[k].as_bool().ok_or_else(|| format!("steps[{i}].calls[{j}].{k} must be a bool"))?;
            }
            for k in ["M", "K", "N"] {
                c[k].as_u64().ok_or_else(|| format!("steps[{i}].calls[{j}].{k}"))?;
            }
            c["counters"]["cycles_total"].as_u64().ok_or_else(|| format!("steps[{i}].calls[{j}].counters"))?;
            if !["FULL", "STRIP"].contains(&c["mode"].as_str().unwrap_or("")) {
                return Err(format!("steps[{i}].calls[{j}].mode must be FULL | STRIP"));
            }
        }
        gemms += calls.len() as u64;
    }
    // An aborted step may hold the failing call without being in the totals.
    if v["status"] != "aborted" && v["summary"]["gemms"].as_u64() != Some(gemms) {
        return Err("summary.gemms != sum of step calls".into());
    }
    Ok(())
}

/// stories260K (or any llama2.c checkpoint that fits) via crate `splitsys_lm`.
pub struct Stories {
    pub q: splitsys_lm::QModel,
    pub tok: splitsys_lm::Tokenizer,
    info: LmModelInfo,
}

/// Default model directory, relative to the repo root.
pub const DEFAULT_MODEL_DIR: &str = "models/stories260K";
pub const MODEL_ORIGIN: &str =
    "karpathy/tinyllamas stories260K (MIT), Hugging Face revision 0bd21da7, fetched and sha256-checked by models/fetch_stories260k.sh";

impl Stories {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let bin = dir.join("stories260K.bin");
        let tokp = dir.join("tok512.bin");
        for p in [&bin, &tokp] {
            if !p.is_file() {
                return Err(format!(
                    "{} not found: run `sh models/fetch_stories260k.sh` (or pass --model-dir)",
                    p.display()
                ));
            }
        }
        let m = splitsys_lm::Model::load(&bin)?;
        splitsys_lm::check_fits(&m.cfg)?;
        let tok = splitsys_lm::Tokenizer::load(&tokp, m.cfg.vocab_size)?;
        let c = m.cfg;
        let info = LmModelInfo {
            name: "stories260K".into(),
            dim: c.dim,
            hidden_dim: c.hidden_dim,
            n_layers: c.n_layers,
            n_heads: c.n_heads,
            n_kv_heads: c.n_kv_heads,
            vocab_size: c.vocab_size,
            seq_len: c.seq_len,
            files: vec![
                FileInfo::of(&bin, &format!("{DEFAULT_MODEL_DIR}/stories260K.bin"))?,
                FileInfo::of(&tokp, &format!("{DEFAULT_MODEL_DIR}/tok512.bin"))?,
            ],
            origin: Some(MODEL_ORIGIN.into()),
        };
        Ok(Self { q: splitsys_lm::QModel::quantize(&m), tok, info })
    }
}

struct StoriesState<'a> {
    g: splitsys_lm::Generator<'a>,
}

impl LmBackend for Stories {
    fn info(&self) -> LmModelInfo {
        self.info.clone()
    }
    fn encode(&self, prompt: &str) -> Vec<u32> {
        self.tok.encode(prompt, true)
    }
    fn decode(&self, prev: u32, tok: u32) -> String {
        String::from_utf8_lossy(&self.tok.decode_safe(prev, tok)).into_owned()
    }
    fn new_state(&self) -> Result<Box<dyn LmState + '_>, String> {
        Ok(Box::new(StoriesState { g: splitsys_lm::Generator::new(&self.q) }))
    }
}

impl LmState for StoriesState<'_> {
    fn step(
        &mut self,
        token: u32,
        exec: &mut dyn FnMut(&CallView<'_>) -> Result<Vec<i32>, String>,
    ) -> Result<Vec<f32>, String> {
        self.g.step(token, &mut |c: &splitsys_lm::LinearCall| {
            exec(&CallView { layer: c.layer, name: c.name, job: &c.job, expected_acc: &c.expected_acc })
        })
    }
}

/// Generation options.
#[derive(Clone, Debug)]
pub struct GenOpts {
    pub prompt: String,
    pub max_new_tokens: usize,
    pub mode: LmMode,
    pub poison: bool,
    /// Checked before every step (dashboard stop); the record ends `done` with `stopped: true`.
    pub stop: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

/// Events for the CLI printer and the dashboard.
pub enum LmEvent<'a> {
    Started(&'a LmRecord),
    Call { step: usize, call: &'a CallCheck },
    Step { rec: &'a LmRecord, step: &'a StepRecord },
    Finished(&'a LmRecord),
}

/// Build the WBUF plan from one dry-run step on the host (golden outputs, no board).
pub fn plan_for(be: &dyn LmBackend, first_token: u32) -> Result<WbufPlan, String> {
    let mut st = be.new_state()?;
    let mut items: Vec<(WKey, usize)> = Vec::new();
    st.step(first_token, &mut |c: &CallView<'_>| {
        items.push((WKey { layer: c.layer, name: c.name.to_string() }, c.job.w_image.len()));
        Ok(c.expected_acc.to_vec())
    })?;
    WbufPlan::new(&items, spec::WBUF_BYTES)
}

fn new_gen_id(now: f64, results: &Path, src: &str) -> String {
    let base = format!("{}-lm", iso_utc(now).replace(':', "-"));
    let dir = lm_dir(results, src);
    let mut id = base.clone();
    let mut i = 2;
    while dir.join(format!("{id}.json")).exists() {
        id = format!("{base}-{i}");
        i += 1;
    }
    id
}

/// Run a generation on `board`. Writes `results/<source>/lm/<gen_id>.json` after
/// every step when `ctx.write_files`. Returns the record (status done/aborted)
/// and the error that aborted it, if any.
pub fn generate(
    board: &mut Board,
    ctx: &mut Context,
    be: &dyn LmBackend,
    o: &GenOpts,
    on_event: &mut dyn FnMut(LmEvent<'_>),
) -> (LmRecord, Option<String>, Option<PathBuf>) {
    let t0 = Instant::now();
    let now = now_unix();
    if ctx.build_id.is_none() {
        if let Ok(p) = board.ping() {
            ctx.build_id = Some(format!("{:08x}", p.build_id));
        }
    }
    let src = effective_source(board.source(), ctx.build_id.as_deref()).to_string();
    let prompt_tokens = be.encode(&o.prompt);
    let plan_r = match prompt_tokens.first() {
        Some(&t) => plan_for(be, t),
        None => Err("empty prompt encoding".into()),
    };
    let gen_id = new_gen_id(now, &ctx.results, &src);
    let mut rec = LmRecord {
        kind: LM_KIND.into(),
        schema: LM_SCHEMA,
        gen_id: gen_id.clone(),
        source: src.clone(),
        mock: ctx.build_id.as_deref() == Some(MOCK_BUILD_ID_HEX),
        build_id: ctx.build_id.clone().unwrap_or_default(),
        git_hash: ctx.git_hash.clone(),
        model_git_hash: ctx.model_git_hash.clone(),
        transport: board.describe(),
        started_utc: iso_utc(now),
        updated_utc: iso_utc(now),
        status: "running".into(),
        error: None,
        prompt: o.prompt.clone(),
        prompt_tokens: prompt_tokens.clone(),
        max_new_tokens: o.max_new_tokens,
        mode_requested: o.mode.as_str().into(),
        model: be.info(),
        numerics: NUMERICS.into(),
        split: serde_json::json!({"fpga": SPLIT_FPGA, "laptop": SPLIT_LAPTOP}),
        wbuf_plan: plan_r.clone().unwrap_or_else(|_| WbufPlan {
            pinned: vec![],
            scratch_base: 0,
            scratch_bytes: 0,
            pinned_bytes: 0,
            streamed_bytes_per_step: 0,
            total_bytes_per_step: 0,
            capacity_bytes: spec::WBUF_BYTES,
            rule: String::new(),
        }),
        generated_text: String::new(),
        text: String::new(),
        summary: GenSummary { wall_note: wall_note(&src, ctx.build_id.as_deref() == Some(MOCK_BUILD_ID_HEX), &board.describe()), ..Default::default() },
        stopped: false,
        steps: vec![],
    };
    let path = ctx.write_files.then(|| lm_dir(&ctx.results, &src).join(format!("{gen_id}.json")));
    let save = |rec: &LmRecord| {
        if let Some(p) = &path {
            if let Err(e) = write_json(p, rec) {
                eprintln!("splitsys-host lm: cannot write {}: {e}", p.display());
            }
        }
    };
    let finish = |rec: &mut LmRecord, err: Option<String>, on_event: &mut dyn FnMut(LmEvent<'_>)| {
        rec.status = if err.is_some() { "aborted" } else { "done" }.into();
        rec.error = err.clone();
        rec.updated_utc = iso_utc(now_unix());
        rec.summary.wall_s = t0.elapsed().as_secs_f64();
        save(rec);
        on_event(LmEvent::Finished(rec));
        err
    };
    let plan = match plan_r {
        Ok(p) => p,
        Err(e) => {
            let err = finish(&mut rec, Some(format!("WBUF plan: {e}")), on_event);
            return (rec, err, path);
        }
    };
    let mut st = match be.new_state() {
        Ok(s) => s,
        Err(e) => {
            let err = finish(&mut rec, Some(e), on_event);
            return (rec, err, path);
        }
    };
    save(&rec);
    on_event(LmEvent::Started(&rec));
    let mut ex = Executor::new(o.mode, plan);
    ex.poison = o.poison;
    let mut text = String::new();
    // The prompt text as the tokenizer renders it (BOS skipped, leading space stripped).
    for w in prompt_tokens.windows(2) {
        text.push_str(&be.decode(w[0], w[1]));
    }
    rec.text = text.clone();
    let mut tok = prompt_tokens[0];
    let n_prompt = prompt_tokens.len();
    let total_steps = (n_prompt - 1 + o.max_new_tokens).min(rec.model.seq_len.max(1));
    let mut generated = 0usize;
    let mut err: Option<String> = None;
    let mut steady_tx = 0u64;
    for pos in 0..total_steps {
        if o.stop.as_ref().is_some_and(|f| f.load(std::sync::atomic::Ordering::Relaxed)) {
            rec.stopped = true;
            break;
        }
        let ts = Instant::now();
        ex.step = pos;
        ex.calls.clear();
        let mut on_call = |c: &CallCheck| on_event(LmEvent::Call { step: pos, call: c });
        let r = {
            let exb = &mut ex;
            let bb = &mut *board;
            st.step(tok, &mut |c: &CallView<'_>| {
                let before = exb.calls.len();
                let out = exb.exec(bb, c);
                if exb.calls.len() > before {
                    on_call(exb.calls.last().expect("call"));
                }
                out
            })
        };
        let calls = std::mem::take(&mut ex.calls);
        let ss = StepSummary::from_calls(&calls, ts.elapsed().as_secs_f64() * 1000.0);
        let logits = match r {
            Ok(l) => l,
            Err(e) => {
                // Keep the failing step's calls (the last one is the culprit).
                rec.steps.push(StepRecord {
                    index: pos,
                    phase: if pos + 1 < n_prompt { "prompt" } else { "generate" }.into(),
                    input_token: tok,
                    output_token: tok,
                    piece: String::new(),
                    text_so_far: text.clone(),
                    utc: iso_utc(now_unix()),
                    summary: ss,
                    calls,
                });
                err = Some(e);
                break;
            }
        };
        let (phase, next) = if pos + 1 < n_prompt { ("prompt", prompt_tokens[pos + 1]) } else { ("generate", argmax(&logits)) };
        // llama2.c stops on BOS (token 1) after the prompt.
        if phase == "generate" && next == 1 {
            push_step(&mut rec, pos, phase, tok, next, String::new(), &text, ss, calls);
            accumulate(&mut rec, &ex, &mut steady_tx);
            on_event(LmEvent::Step { rec: &rec, step: rec.steps.last().expect("step") });
            save(&rec);
            break;
        }
        let piece = be.decode(tok, next);
        if phase == "generate" {
            text.push_str(&piece);
            rec.generated_text.push_str(&piece);
            generated += 1;
        }
        rec.text = text.clone();
        rec.summary.tokens_generated = generated;
        push_step(&mut rec, pos, phase, tok, next, piece, &text, ss, calls);
        accumulate(&mut rec, &ex, &mut steady_tx);
        // 7-segment: this step's on-chip utilization (from the counters).
        let s = &rec.steps.last().expect("step").summary;
        let agg = cycle::Counters { mac_useful: s.mac_useful, cycles_total: s.cycles_total, ..Default::default() };
        let mode = if s.modes.get("STRIP").copied().unwrap_or(0) >= s.modes.get("FULL").copied().unwrap_or(0) {
            spec::MODE_STRIP
        } else {
            spec::MODE_FULL
        };
        let _ = board.set_display(runner::util_permille(&agg), mode);
        rec.updated_utc = iso_utc(now_unix());
        rec.summary.wall_s = t0.elapsed().as_secs_f64();
        save(&rec);
        on_event(LmEvent::Step { rec: &rec, step: rec.steps.last().expect("step") });
        tok = next;
    }
    let err = finish(&mut rec, err, on_event);
    (rec, err, path)
}

#[allow(clippy::too_many_arguments)]
fn push_step(
    rec: &mut LmRecord,
    pos: usize,
    phase: &str,
    input: u32,
    out: u32,
    piece: String,
    text: &str,
    ss: StepSummary,
    calls: Vec<CallCheck>,
) {
    rec.steps.push(StepRecord {
        index: pos,
        phase: phase.into(),
        input_token: input,
        output_token: out,
        piece,
        text_so_far: text.to_string(),
        utc: iso_utc(now_unix()),
        summary: ss,
        calls,
    });
}

fn accumulate(rec: &mut LmRecord, ex: &Executor, steady_tx: &mut u64) {
    let s = &rec.steps.last().expect("step").summary;
    let g = &mut rec.summary;
    g.steps += 1;
    if rec.steps.last().expect("step").phase == "prompt" {
        g.prompt_steps += 1;
    }
    g.gemms += s.gemms;
    g.bit_exact += s.bit_exact;
    g.counters_eq_model += s.counters_eq_model;
    g.invariants_ok += s.invariants_ok;
    g.mac_useful += s.mac_useful;
    g.cycles_total += s.cycles_total;
    g.bytes_tx += s.bytes_tx;
    g.bytes_rx += s.bytes_rx;
    if g.steps > 1 {
        *steady_tx += s.bytes_tx;
        g.bytes_tx_per_step_steady = *steady_tx as f64 / (g.steps - 1) as f64;
    }
    g.w_bytes_sent = ex.w_bytes_sent;
    g.w_bytes_resident = ex.w_bytes_resident;
    let wall: f64 = rec.steps.iter().map(|s| s.summary.wall_ms).sum::<f64>() / 1000.0;
    g.wall_s_per_step = wall / g.steps as f64;
    g.cycles_per_step = g.cycles_total as f64 / g.steps as f64;
}

/// `splitsys-host lm`: stream tokens to stdout, print a summary. Ok(false) on abort.
pub fn cli_lm(board: &mut Board, ctx: &mut Context, model_dir: &Path, o: &GenOpts) -> Result<bool, String> {
    use std::io::Write;
    let t_load = Instant::now();
    let be = Stories::load(model_dir)?;
    let info = be.info();
    eprintln!(
        "lm: {} (dim {}, hidden {}, {} layers, vocab {}) loaded + quantized in {:.2} s; {} via {}",
        info.name,
        info.dim,
        info.hidden_dim,
        info.n_layers,
        info.vocab_size,
        t_load.elapsed().as_secs_f64(),
        o.mode.as_str(),
        board.describe()
    );
    eprintln!("lm: {SPLIT_FPGA}");
    eprintln!("lm: {SPLIT_LAPTOP}");
    let mut printed_prompt = false;
    let (rec, err, path) = generate(board, ctx, &be, o, &mut |e| match e {
        LmEvent::Started(r) => {
            let p = &r.wbuf_plan;
            eprintln!(
                "lm: source {} build {} | WBUF plan: {} matrices pinned ({} B), scratch {} B, {} of {} weight B streamed per step",
                r.source,
                r.build_id,
                p.pinned.len(),
                p.pinned_bytes,
                p.scratch_bytes,
                p.streamed_bytes_per_step,
                p.total_bytes_per_step
            );
        }
        LmEvent::Step { rec, step } => {
            let mut out = std::io::stdout();
            if !printed_prompt && step.phase == "generate" {
                let _ = write!(out, "{}", rec.text.strip_suffix(step.piece.as_str()).unwrap_or(&rec.text));
                printed_prompt = true;
            }
            if step.phase == "generate" {
                let _ = write!(out, "{}", step.piece);
            }
            let _ = out.flush();
            let s = &step.summary;
            eprintln!(
                "  [step {:>2} {:<8} tok {:>3}] {} GEMMs, {} bit-exact, {} counters==model, {} cycles on chip, {} B tx, {:.2} s wall",
                step.index, step.phase, step.output_token, s.gemms, s.bit_exact, s.counters_eq_model, s.cycles_total, s.bytes_tx, s.wall_ms / 1000.0
            );
        }
        LmEvent::Call { .. } | LmEvent::Finished(_) => {}
    });
    println!();
    let s = &rec.summary;
    println!("---- lm summary ({}) ----", if err.is_none() { "done" } else { "ABORTED" });
    println!("source            {} (build {}, transport {})", rec.source, rec.build_id, rec.transport);
    println!("prompt            {:?} ({} tokens incl. BOS)", rec.prompt, rec.prompt_tokens.len());
    println!("text              {:?}", rec.text);
    println!("steps             {} ({} prompt, {} generated tokens)", s.steps, s.prompt_steps, s.tokens_generated);
    println!("GEMMs on board    {}", s.gemms);
    println!("bit-exact         {} / {}", s.bit_exact, s.gemms);
    println!("counters == model {} / {} (all 12 counters per GEMM)", s.counters_eq_model, s.gemms);
    println!("invariants ok     {} / {}", s.invariants_ok, s.gemms);
    println!("on-chip cycles    {:.0} per step ({:.1} us at {} MHz), from the counters", s.cycles_per_step, s.cycles_per_step / (spec::CLK_HZ as f64 / 1e6), spec::CLK_HZ / 1_000_000);
    println!("bytes sent        {} total, {:.0} per step steady state; weights {} B sent, {} B reused from WBUF", s.bytes_tx, s.bytes_tx_per_step_steady, s.w_bytes_sent, s.w_bytes_resident);
    println!("bytes received    {}", s.bytes_rx);
    println!("wall time         {:.2} s total, {:.2} s per step: {}", s.wall_s, s.wall_s_per_step, s.wall_note);
    if rec.mock {
        println!("NOTE              mock board: its outputs and counters come from golden + the cycle model, so these checks are circular (not hardware evidence)");
    }
    if let Some(p) = &path {
        println!("record            {}", p.display());
    }
    if let Some(e) = &err {
        eprintln!("splitsys-host lm: ABORTED: {e}");
    }
    Ok(err.is_none() && rec.verified())
}

/// Load every LM record in a directory (sorted by gen_id), newest last.
pub fn load_lm_records(dir: &Path) -> Vec<LmRecord> {
    let mut v: Vec<LmRecord> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|p| std::fs::read(&p).ok())
        .filter_map(|b| serde_json::from_slice::<LmRecord>(&b).ok())
        .filter(|r| r.kind == LM_KIND)
        .collect();
    v.sort_by_key(|r| gen_order_key(&r.gen_id));
    v
}

/// Chronological order of gen ids `<utc>-lm` < `<utc>-lm-2` < ... < `<utc>-lm-10`.
pub fn gen_order_key(id: &str) -> (String, u64) {
    match id.rsplit_once("-lm-") {
        Some((t, n)) if n.chars().all(|c| c.is_ascii_digit()) => (t.to_string(), n.parse().unwrap_or(0)),
        _ => (id.trim_end_matches("-lm").to_string(), 1),
    }
}

/// Dashboard view of a record: the per-call counter values are dropped (the
/// Scene 4 grid needs each call's mode, shape and check results only).
pub fn record_view(r: &LmRecord) -> Value {
    let mut v = serde_json::to_value(r).unwrap_or(Value::Null);
    if let Some(steps) = v.get_mut("steps").and_then(|s| s.as_array_mut()) {
        for s in steps.iter_mut() {
            if let Some(calls) = s.get_mut("calls").and_then(|c| c.as_array_mut()) {
                for c in calls.iter_mut().filter_map(|c| c.as_object_mut()) {
                    c.remove("counters");
                }
            }
        }
    }
    v
}

#[cfg(test)]
pub mod fake {
    //! A tiny deterministic stand-in for `splitsys_lm` (same call pattern and
    //! stories260K shapes), so the executor, plan and records are tested even
    //! without model files.
    use super::*;

    pub struct FakeLm {
        pub layers: usize,
        pub dim: usize,
        pub hidden: usize,
        pub kv: usize,
        pub vocab: usize,
        pub w: Vec<(Option<usize>, &'static str, golden::MatI8)>,
    }

    impl FakeLm {
        pub fn stories_like(layers: usize) -> Self {
            let (dim, hidden, kv, vocab) = (64usize, 172usize, 32usize, 512usize);
            let mut rng = golden::rng::Rng::new(260);
            let mut mat = |k: usize, n: usize| golden::MatI8::from_fn(k, n, |_, _| rng.i8());
            let mut w = Vec::new();
            for l in 0..layers {
                for (name, k, n) in
                    [("wq", dim, dim), ("wk", dim, kv), ("wv", dim, kv), ("wo", dim, dim), ("w1", dim, hidden), ("w3", dim, hidden), ("w2", hidden, dim)]
                {
                    w.push((Some(l), name, mat(k, n)));
                }
            }
            w.push((None, "cls0", mat(dim, 256)));
            w.push((None, "cls1", mat(dim, vocab - 256)));
            Self { layers, dim, hidden, kv, vocab, w }
        }
    }

    pub struct FakeState<'a> {
        lm: &'a FakeLm,
        pos: usize,
    }

    impl LmBackend for FakeLm {
        fn info(&self) -> LmModelInfo {
            LmModelInfo {
                name: "fake-stories-shapes".into(),
                dim: self.dim,
                hidden_dim: self.hidden,
                n_layers: self.layers,
                n_heads: 8,
                n_kv_heads: 4,
                vocab_size: self.vocab,
                seq_len: 64,
                files: vec![],
                origin: Some("test fake".into()),
            }
        }
        fn encode(&self, prompt: &str) -> Vec<u32> {
            std::iter::once(1).chain(prompt.bytes().map(|b| 3 + u32::from(b) % 200)).collect()
        }
        fn decode(&self, _prev: u32, tok: u32) -> String {
            format!("<{tok}>")
        }
        fn new_state(&self) -> Result<Box<dyn LmState + '_>, String> {
            Ok(Box::new(FakeState { lm: self, pos: 0 }))
        }
    }

    impl LmState for FakeState<'_> {
        fn step(
            &mut self,
            token: u32,
            exec: &mut dyn FnMut(&CallView<'_>) -> Result<Vec<i32>, String>,
        ) -> Result<Vec<f32>, String> {
            let mut x: Vec<i8> = (0..self.lm.dim).map(|i| ((token as usize * 31 + i * 7 + self.pos) % 255) as i8).collect();
            let mut logits = Vec::new();
            let mut ffn_in = x.clone();
            for (layer, name, w) in &self.lm.w {
                let input = match *name {
                    "w2" => {
                        // fake SwiGLU output: hidden-sized
                        (0..self.lm.hidden).map(|i| (i as i8).wrapping_mul(3) ^ x[i % x.len()]).collect::<Vec<i8>>()
                    }
                    "w3" => ffn_in.clone(),
                    _ => x.clone(),
                };
                if *name == "w1" {
                    ffn_in = input.clone();
                }
                let a = golden::MatI8::from_fn(1, input.len(), |_, k| input[k]);
                let job = golden::Job::new(spec::MODE_FULL, a, w.clone(), None, golden::Bases::default())
                    .map_err(|e| e.to_string())?;
                let expected: Vec<i32> = (0..job.desc.dims().2).map(|j| job.acc.get(0, j)).collect();
                let out = exec(&CallView { layer: *layer, name, job: &job, expected_acc: &expected })?;
                if name.starts_with("cls") {
                    logits.extend(out.iter().map(|&v| v as f32));
                } else if *name == "wo" || *name == "w2" {
                    x = out.iter().take(self.lm.dim).map(|&v| (v >> 10).clamp(-127, 127) as i8).collect();
                }
            }
            self.pos += 1;
            Ok(logits)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeLm;
    use super::*;
    use crate::mock::MockConfig;
    use crate::transport::MockTransport;

    fn ctx(dir: Option<PathBuf>) -> Context {
        Context {
            results: dir.clone().unwrap_or_else(|| PathBuf::from("unused")),
            git_hash: "test".into(),
            model_git_hash: "test".into(),
            build_id: None,
            poison: true,
            write_files: dir.is_some(),
        }
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("splitsys-lm-{tag}-{}-{}", std::process::id(), now_unix() as u64));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn opts(n: usize, mode: LmMode) -> GenOpts {
        GenOpts { prompt: "Hi".into(), max_new_tokens: n, mode, poison: true, stop: None }
    }

    #[test]
    fn plan_pins_most_bytes_and_leaves_a_scratch_slot() {
        let lm = FakeLm::stories_like(5);
        let p = plan_for(&lm, 1).unwrap();
        assert!(p.pinned_bytes + p.scratch_bytes <= spec::WBUF_BYTES, "{p:?}");
        assert!(p.pinned_bytes > 40_000, "pins a useful share: {}", p.pinned_bytes);
        assert_eq!(p.total_bytes_per_step, p.pinned_bytes + p.streamed_bytes_per_step);
        // No two pinned slots (or the scratch) overlap.
        let mut spans: Vec<(usize, usize)> =
            p.pinned.iter().map(|s| (s.w_base as usize * 16, s.w_base as usize * 16 + s.bytes)).collect();
        spans.push((p.scratch_base as usize * 16, p.scratch_base as usize * 16 + p.scratch_bytes));
        spans.sort_unstable();
        for w in spans.windows(2) {
            assert!(w[0].1 <= w[1].0, "{spans:?}");
        }
        // Everything fits when the model is small.
        let one = FakeLm::stories_like(1);
        let k: Vec<(WKey, usize)> = one.w.iter().map(|(l, n, w)| (WKey { layer: *l, name: n.to_string() }, golden::pack_w_blocks16(w).len())).collect();
        let total: usize = k.iter().map(|x| x.1).sum();
        let p1 = WbufPlan::new(&k, 1 << 20).unwrap();
        assert_eq!(p1.pinned_bytes, total);
        assert_eq!(p1.streamed_bytes_per_step, 0);
        assert!(WbufPlan::new(&k, 1000).is_err());
    }

    #[test]
    fn mock_generation_is_bit_exact_and_counters_match_and_resends_only_unpinned() {
        let lm = FakeLm::stories_like(5);
        let mut b = Board::new(Box::new(MockTransport::new(MockConfig::default())));
        let dir = tmp("e2e");
        let mut c = ctx(Some(dir.clone()));
        let mut calls_seen = 0usize;
        let (rec, err, path) = generate(&mut b, &mut c, &lm, &opts(3, LmMode::Auto), &mut |e| {
            if let LmEvent::Call { .. } = e {
                calls_seen += 1;
            }
        });
        assert!(err.is_none(), "{err:?}");
        assert_eq!(rec.status, "done");
        assert!(rec.verified());
        let per_step = lm.w.len() as u64;
        assert_eq!(rec.summary.gemms, per_step * rec.steps.len() as u64);
        assert_eq!(calls_seen as u64, rec.summary.gemms);
        assert_eq!(rec.summary.tokens_generated + rec.summary.prompt_steps, rec.steps.len());
        assert_eq!(rec.source, "model");
        assert!(rec.mock);
        // Auto at M = 1 mostly picks STRIP; every call says which mode ran.
        assert!(rec.steps[0].summary.modes.get("STRIP").copied().unwrap_or(0) > 0);
        // Steady state: only unpinned weights are re-sent.
        let s1 = &rec.steps[1].summary;
        assert_eq!(s1.w_resident as usize, rec.wbuf_plan.pinned.len());
        let w_tx1: usize = rec.steps[1].calls.iter().filter(|c| c.w_sent).map(|c| c.k * c.n.div_ceil(16) * 16).sum();
        assert_eq!(w_tx1, rec.wbuf_plan.streamed_bytes_per_step);
        assert!(rec.summary.bytes_tx_per_step_steady > rec.wbuf_plan.streamed_bytes_per_step as f64);
        // Record on disk, under results/mock/lm/, valid schema, round trips.
        let p = path.unwrap();
        assert!(p.starts_with(dir.join("mock").join("lm")), "{}", p.display());
        let v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        validate_lm_json(&v).unwrap();
        let back: LmRecord = serde_json::from_value(v).unwrap();
        // serde_json float parsing may differ by an ulp: compare with wall times zeroed.
        let zero = |mut r: LmRecord| {
            r.summary.wall_s = 0.0;
            r.summary.wall_s_per_step = 0.0;
            r.summary.cycles_per_step = 0.0;
            r.summary.bytes_tx_per_step_steady = 0.0;
            for s in &mut r.steps {
                s.summary.wall_ms = 0.0;
                s.calls.iter_mut().for_each(|c| c.wall_ms = 0.0);
            }
            r
        };
        assert_eq!(zero(back), zero(rec.clone()));
        assert_eq!(load_lm_records(&dir.join("mock").join("lm")).len(), 1);
        // The per-GEMM run records are untouched (no top-level files).
        assert!(crate::record::load_runs(&dir.join("mock")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn flipped_read_byte_aborts_loudly_with_step_call_and_layer() {
        let lm = FakeLm::stories_like(2);
        let mut b = Board::new(Box::new(MockTransport::new(MockConfig::default())));
        let dir = tmp("flip");
        let mut c = ctx(Some(dir.clone()));
        let (rec, err, _) = generate(&mut b, &mut c, &lm, &opts(4, LmMode::Strip), &mut |_| {});
        assert!(err.is_none(), "baseline passes: {err:?}");
        assert!(rec.verified());
        // Now with a fault: the mock flips one bit in the next READ_MEM response.
        let mut t = MockTransport::new(MockConfig::default());
        t.board.cfg.flip_read_byte = 1;
        let mut b = Board::new(Box::new(t));
        let (rec, err, path) = generate(&mut b, &mut c, &lm, &opts(4, LmMode::Strip), &mut |_| {});
        let err = err.expect("must abort");
        assert!(err.contains("OUTPUT MISMATCH"), "{err}");
        assert!(err.contains("step 0 call #0 L0.wq"), "{err}");
        assert_eq!(rec.status, "aborted");
        assert!(!rec.verified());
        assert_eq!(rec.steps.len(), 1, "stops at the failing step");
        let last = rec.steps[0].calls.last().unwrap();
        assert!(!last.bit_exact && last.mismatches > 0);
        // Saved as aborted and still schema-valid.
        let v: Value = serde_json::from_slice(&std::fs::read(path.unwrap()).unwrap()).unwrap();
        validate_lm_json(&v).unwrap();
        assert_eq!(v["status"], "aborted");
        assert!(v["error"].as_str().unwrap().contains("OUTPUT MISMATCH"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_expected_acc_is_rejected_before_running() {
        let lm = FakeLm::stories_like(1);
        let mut b = Board::new(Box::new(MockTransport::new(MockConfig::default())));
        let plan = plan_for(&lm, 1).unwrap();
        let mut ex = Executor::new(LmMode::Full, plan);
        let (l, n, w) = &lm.w[0];
        let a = golden::MatI8::from_fn(1, 64, |_, k| k as i8);
        let job = golden::Job::new(spec::MODE_FULL, a, w.clone(), None, golden::Bases::default()).unwrap();
        let mut exp: Vec<i32> = (0..64).map(|j| job.acc.get(0, j)).collect();
        let ok = ex.exec(&mut b, &CallView { layer: *l, name: n, job: &job, expected_acc: &exp }).unwrap();
        assert_eq!(ok, exp);
        exp[5] ^= 1;
        let e = ex.exec(&mut b, &CallView { layer: *l, name: n, job: &job, expected_acc: &exp }).unwrap_err();
        assert!(e.contains("disagrees with golden"), "{e}");
        // Second identical call: W and A are resident, nothing re-sent.
        exp[5] ^= 1;
        let tx = b.stats.bytes_tx;
        ex.exec(&mut b, &CallView { layer: *l, name: n, job: &job, expected_acc: &exp }).unwrap();
        let c = ex.calls.last().unwrap();
        assert!(!c.w_sent && !c.a_sent);
        assert!(b.stats.bytes_tx - tx < 600, "{}", b.stats.bytes_tx - tx);
    }

    /// The real stories260K through the mock board: the board-driven text equals the
    /// text the lm crate generates with its own golden executor. SKIP if the model
    /// files are absent (models/fetch_stories260k.sh), unless SPLITSYS_LM_REQUIRE_MODEL=1.
    #[test]
    fn stories260k_on_mock_matches_golden_generation() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../models/stories260K");
        let be = match Stories::load(&dir) {
            Ok(b) => b,
            Err(e) => {
                assert!(std::env::var("SPLITSYS_LM_REQUIRE_MODEL").as_deref() != Ok("1"), "{e}");
                eprintln!("SKIP stories260k_on_mock_matches_golden_generation: {e}");
                return;
            }
        };
        let prompt = "Once upon a time";
        let new = 6;
        // Reference: the lm crate alone (golden_exec), same greedy loop.
        let ptoks = be.encode(prompt);
        let mut g = splitsys_lm::Generator::new(&be.q);
        let seq = splitsys_lm::generate_greedy(&ptoks, ptoks.len() - 1 + new, &mut |t| {
            g.step(t, &mut |c: &splitsys_lm::LinearCall| splitsys_lm::golden_exec(c))
        })
        .unwrap();
        let mut want = String::new();
        for w in seq.windows(2) {
            want.push_str(&be.decode(w[0], w[1]));
        }
        for mode in [LmMode::Auto, LmMode::Full] {
            let mut b = Board::new(Box::new(MockTransport::new(MockConfig::default())));
            let mut c = ctx(None);
            let o = GenOpts { prompt: prompt.into(), max_new_tokens: new, mode, poison: true, stop: None };
            let (rec, err, _) = generate(&mut b, &mut c, &be, &o, &mut |_| {});
            assert!(err.is_none(), "{err:?}");
            assert!(rec.verified());
            assert_eq!(rec.text, want, "{mode:?}");
            assert_eq!(rec.summary.gemms, 37 * rec.steps.len() as u64);
            assert_eq!(rec.model.files.len(), 2);
            assert!(rec.steps.iter().flat_map(|s| &s.calls).all(|c| c.mode == if mode == LmMode::Full { "FULL" } else { "STRIP" }));
            validate_lm_json(&serde_json::to_value(&rec).unwrap()).unwrap();
        }
    }

    #[test]
    fn stop_flag_ends_the_generation_cleanly() {
        let lm = FakeLm::stories_like(1);
        let mut b = Board::new(Box::new(MockTransport::new(MockConfig::default())));
        let mut c = ctx(None);
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let f2 = flag.clone();
        let o = GenOpts { prompt: "Hi".into(), max_new_tokens: 10, mode: LmMode::Strip, poison: false, stop: Some(flag) };
        let (rec, err, _) = generate(&mut b, &mut c, &lm, &o, &mut |e| {
            if let LmEvent::Step { step, .. } = e {
                if step.index == 3 {
                    f2.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        });
        assert!(err.is_none());
        assert!(rec.stopped && rec.status == "done");
        assert_eq!(rec.steps.len(), 4);
        assert!(rec.verified());
    }

    #[test]
    fn schema_rejects_missing_and_bad_fields() {
        let lm = FakeLm::stories_like(1);
        let mut b = Board::new(Box::new(MockTransport::new(MockConfig::default())));
        let mut c = ctx(None);
        let (rec, err, _) = generate(&mut b, &mut c, &lm, &opts(1, LmMode::Full), &mut |_| {});
        assert!(err.is_none());
        let v = serde_json::to_value(&rec).unwrap();
        validate_lm_json(&v).unwrap();
        for (k, bad) in [("source", Value::from("replay")), ("status", Value::from("ok")), ("split", Value::Null)] {
            let mut w = v.clone();
            w[k] = bad;
            assert!(validate_lm_json(&w).is_err(), "{k}");
        }
        let mut w = v.clone();
        w["steps"][0]["calls"][0]["mode"] = Value::from("AUTO");
        assert!(validate_lm_json(&w).is_err());
        let mut w = v.clone();
        w["summary"]["gemms"] = Value::from(1);
        assert!(validate_lm_json(&w).is_err());
        // The dashboard view keeps every call's checks but not the raw counters.
        let view = record_view(&rec);
        let c0 = &view["steps"][0]["calls"][0];
        assert!(c0["bit_exact"].is_boolean() && c0["counters"].is_null());
    }
}
