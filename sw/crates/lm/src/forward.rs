//! Forward pass (`run.c forward` @ 350e04fe) shared by the f32 reference and the
//! int8/accelerator path. Only the linear layers differ: the f32 path runs
//! `run.c matmul`; the int8 path builds one SplitSys GEMM job per projection and
//! hands it to the caller's `exec`.

use crate::model::{Config, Model, NonLinear};
use crate::quant::{dequantize, quantize_activation, QLinear, QModel, CLS_NAMES, LAYER_CALL_ORDER};
use crate::tokenizer::BOS;
use splitsys_golden::consts::MODE_FULL;
use splitsys_golden::{gemm_i32, pack_c_i32, unpack_a_blocks8, unpack_c_i32, unpack_w_blocks16};
use splitsys_golden::{Bases, Job, MatI8};

/// Which projection a linear call is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proj {
    Wq,
    Wk,
    Wv,
    Wo,
    W1,
    W3,
    W2,
    /// Full classifier (the int8 path splits it into `cls0`, `cls1`, ...).
    Cls,
}

impl Proj {
    /// Index into `LAYER_CALL_ORDER` / `QModel::layers[l]`.
    fn slot(self) -> usize {
        match self {
            Proj::Wq => 0,
            Proj::Wk => 1,
            Proj::Wv => 2,
            Proj::Wo => 3,
            Proj::W1 => 4,
            Proj::W3 => 5,
            Proj::W2 => 6,
            Proj::Cls => usize::MAX,
        }
    }
}

/// Key/value cache: `n_layers × seq_len × kv_dim` each (`run.c RunState`).
#[derive(Clone, Debug)]
pub struct KvCache {
    key: Vec<f32>,
    value: Vec<f32>,
    kv_dim: usize,
    seq_len: usize,
}

impl KvCache {
    pub fn new(c: &Config) -> Self {
        let n = c.n_layers * c.seq_len * c.kv_dim();
        Self {
            key: vec![0.0; n],
            value: vec![0.0; n],
            kv_dim: c.kv_dim(),
            seq_len: c.seq_len,
        }
    }
}

fn rmsnorm(o: &mut [f32], x: &[f32], weight: &[f32]) {
    let size = x.len();
    let mut ss = 0.0f32;
    for &v in x {
        ss += v * v;
    }
    ss /= size as f32;
    ss += 1e-5f32;
    ss = 1.0f32 / ss.sqrt();
    for j in 0..size {
        o[j] = weight[j] * (ss * x[j]);
    }
}

fn softmax(x: &mut [f32]) {
    let mut max_val = x[0];
    for &v in &x[1..] {
        if v > max_val {
            max_val = v;
        }
    }
    let mut sum = 0.0f32;
    for v in x.iter_mut() {
        *v = (*v - max_val).exp();
        sum += *v;
    }
    for v in x.iter_mut() {
        *v /= sum;
    }
}

/// `run.c matmul`: W is `d × n` row-major, `xout[i] = Σ_j w[i*n+j] x[j]` (f32,
/// sequential accumulation).
fn matmul(x: &[f32], w: &[f32], d: usize) -> Vec<f32> {
    let n = x.len();
    (0..d)
        .map(|i| {
            let mut val = 0.0f32;
            for j in 0..n {
                val += w[i * n + j] * x[j];
            }
            val
        })
        .collect()
}

type LinFn<'a> = dyn FnMut(Option<usize>, Proj, &[f32]) -> Result<Vec<f32>, String> + 'a;

/// One `run.c forward(token, pos)` with pluggable linear layers.
fn forward_core(
    p: &Config,
    nl: &NonLinear,
    kv: &mut KvCache,
    token: u32,
    pos: usize,
    lin: &mut LinFn<'_>,
) -> Result<Vec<f32>, String> {
    let (dim, hidden) = (p.dim, p.hidden_dim);
    let kv_dim = p.kv_dim();
    let kv_mul = p.kv_mul();
    let head_size = p.head_size();
    let t = token as usize;
    if t >= p.vocab_size {
        return Err(format!("token {token} >= vocab {}", p.vocab_size));
    }
    if pos >= p.seq_len || kv.seq_len != p.seq_len || kv.kv_dim != kv_dim {
        return Err(format!(
            "position {pos} outside the sequence length {}",
            p.seq_len
        ));
    }
    let mut x = nl.token_embedding[t * dim..(t + 1) * dim].to_vec();
    let mut xb = vec![0.0f32; dim];
    let mut att = vec![0.0f32; pos + 1];
    let want = |v: Vec<f32>, n: usize, what: &str| -> Result<Vec<f32>, String> {
        if v.len() == n {
            Ok(v)
        } else {
            Err(format!(
                "{what}: linear returned {} values, expected {n}",
                v.len()
            ))
        }
    };

    for l in 0..p.n_layers {
        rmsnorm(&mut xb, &x, &nl.rms_att[l]);
        let loff = l * p.seq_len * kv_dim;
        let mut q = want(lin(Some(l), Proj::Wq, &xb)?, dim, "wq")?;
        let mut k = want(lin(Some(l), Proj::Wk, &xb)?, kv_dim, "wk")?;
        let v = want(lin(Some(l), Proj::Wv, &xb)?, kv_dim, "wv")?;

        // RoPE
        let mut i = 0;
        while i < dim {
            let head_dim = i % head_size;
            let freq = 1.0f32 / 10000.0f32.powf(head_dim as f32 / head_size as f32);
            let val = pos as f32 * freq;
            let fcr = val.cos();
            let fci = val.sin();
            let rotn = if i < kv_dim { 2 } else { 1 };
            for vi in 0..rotn {
                let vec: &mut [f32] = if vi == 0 { &mut q } else { &mut k };
                let v0 = vec[i];
                let v1 = vec[i + 1];
                vec[i] = v0 * fcr - v1 * fci;
                vec[i + 1] = v0 * fci + v1 * fcr;
            }
            i += 2;
        }
        let at = loff + pos * kv_dim;
        kv.key[at..at + kv_dim].copy_from_slice(&k);
        kv.value[at..at + kv_dim].copy_from_slice(&v);

        // multi-head attention
        for h in 0..p.n_heads {
            let qh = &q[h * head_size..(h + 1) * head_size];
            for (tt, a) in att.iter_mut().enumerate() {
                let ko = loff + tt * kv_dim + (h / kv_mul) * head_size;
                let kh = &kv.key[ko..ko + head_size];
                let mut score = 0.0f32;
                for i in 0..head_size {
                    score += qh[i] * kh[i];
                }
                score /= (head_size as f32).sqrt();
                *a = score;
            }
            softmax(&mut att);
            let xbh = &mut xb[h * head_size..(h + 1) * head_size];
            xbh.iter_mut().for_each(|e| *e = 0.0);
            for (tt, &a) in att.iter().enumerate() {
                let vo = loff + tt * kv_dim + (h / kv_mul) * head_size;
                let vh = &kv.value[vo..vo + head_size];
                for i in 0..head_size {
                    xbh[i] += a * vh[i];
                }
            }
        }

        let xb2 = want(lin(Some(l), Proj::Wo, &xb)?, dim, "wo")?;
        for i in 0..dim {
            x[i] += xb2[i];
        }
        rmsnorm(&mut xb, &x, &nl.rms_ffn[l]);
        let mut hb = want(lin(Some(l), Proj::W1, &xb)?, hidden, "w1")?;
        let hb2 = want(lin(Some(l), Proj::W3, &xb)?, hidden, "w3")?;
        for i in 0..hidden {
            let mut val = hb[i];
            val *= 1.0f32 / (1.0f32 + (-val).exp());
            val *= hb2[i];
            hb[i] = val;
        }
        let xo = want(lin(Some(l), Proj::W2, &hb)?, dim, "w2")?;
        for i in 0..dim {
            x[i] += xo[i];
        }
    }
    let xin = x.clone();
    rmsnorm(&mut x, &xin, &nl.rms_final);
    want(lin(None, Proj::Cls, &x)?, p.vocab_size, "cls")
}

/// f32 reference step (`run.c forward`), at position `pos` with cache `kv`.
pub fn reference_step_f32(
    m: &Model,
    kv: &mut KvCache,
    pos: usize,
    token: u32,
) -> Result<Vec<f32>, String> {
    let c = m.cfg;
    let mut lin = |layer: Option<usize>, proj: Proj, x: &[f32]| -> Result<Vec<f32>, String> {
        Ok(match (layer, proj) {
            (None, Proj::Cls) => matmul(x, &m.wcls, c.vocab_size),
            (Some(l), pr) => {
                let w = &m.layers[l];
                match pr {
                    Proj::Wq => matmul(x, &w.wq, c.dim),
                    Proj::Wk => matmul(x, &w.wk, c.kv_dim()),
                    Proj::Wv => matmul(x, &w.wv, c.kv_dim()),
                    Proj::Wo => matmul(x, &w.wo, c.dim),
                    Proj::W1 => matmul(x, &w.w1, c.hidden_dim),
                    Proj::W3 => matmul(x, &w.w3, c.hidden_dim),
                    Proj::W2 => matmul(x, &w.w2, c.dim),
                    Proj::Cls => return Err("classifier inside a layer".into()),
                }
            }
            (None, _) => return Err("layer projection without a layer".into()),
        })
    };
    forward_core(&c, &m.nl, kv, token, pos, &mut lin)
}

/// Stateful f32 reference decoder.
pub struct F32Generator<'a> {
    pub model: &'a Model,
    kv: KvCache,
    pub pos: usize,
}

impl<'a> F32Generator<'a> {
    pub fn new(model: &'a Model) -> Self {
        Self {
            model,
            kv: KvCache::new(&model.cfg),
            pos: 0,
        }
    }
    pub fn step(&mut self, token: u32) -> Result<Vec<f32>, String> {
        let r = reference_step_f32(self.model, &mut self.kv, self.pos, token)?;
        self.pos += 1;
        Ok(r)
    }
}

/// One linear layer call to run on the accelerator: x (int8, per-tensor scale) ·
/// W (int8) -> int32. `job` is M=1, K=in, N=out (≤ N_MAX), flags = FLAGS_INT32,
/// with packed A/W images and the expected OBUF image from golden;
/// `expected_acc` is golden's N int32 accumulators.
#[derive(Clone, Debug)]
pub struct LinearCall {
    /// `Some(l)` for layer projections, `None` for classifier chunks.
    pub layer: Option<usize>,
    /// "wq","wk","wv","wo","w1","w3","w2" or "cls0","cls1",...
    pub name: &'static str,
    pub job: Job,
    pub expected_acc: Vec<i32>,
}

impl LinearCall {
    /// Check returned accumulators against golden.
    pub fn check(&self, got: &[i32]) -> Result<(), String> {
        if got == self.expected_acc.as_slice() {
            return Ok(());
        }
        if got.len() != self.expected_acc.len() {
            return Err(format!(
                "{}: {} values, expected {}",
                self.label(),
                got.len(),
                self.expected_acc.len()
            ));
        }
        let i = got
            .iter()
            .zip(&self.expected_acc)
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        Err(format!(
            "{}: acc[{i}] = {} but golden = {}",
            self.label(),
            got[i],
            self.expected_acc[i]
        ))
    }

    /// "L3.wq" / "cls1".
    pub fn label(&self) -> String {
        match self.layer {
            Some(l) => format!("L{l}.{}", self.name),
            None => self.name.to_string(),
        }
    }
}

/// Extract the N int32 accumulators of row 0 from an OBUF read-back that starts at
/// `call.job.out_byte_addr()` (int32 bypass layout, SPEC §1.3). Only the in-bounds
/// bytes are read; `readback` must cover at least `call.job.obuf_image.len()` bytes
/// up to the last in-bounds byte.
pub fn acc_from_obuf(call: &LinearCall, readback: &[u8]) -> Result<Vec<i32>, String> {
    let (m, _k, n) = call.job.desc.dims();
    let need = call
        .job
        .compare
        .iter()
        .rposition(|&b| b)
        .map_or(0, |i| i + 1);
    if readback.len() < need {
        return Err(format!(
            "{}: read-back {} bytes < {need}",
            call.label(),
            readback.len()
        ));
    }
    let mut buf = readback.to_vec();
    buf.resize(call.job.obuf_image.len().max(buf.len()), 0);
    Ok(unpack_c_i32(&buf, m, n).row(0).to_vec())
}

/// A software accelerator: unpack the job's A and W images, multiply with golden,
/// pack to the int32 OBUF layout and read row 0 back. Independent of
/// `expected_acc` (it exercises the packing round trip).
pub fn golden_exec(call: &LinearCall) -> Result<Vec<i32>, String> {
    let (m, k, n) = call.job.desc.dims();
    let a = unpack_a_blocks8(&call.job.a_image, m, k);
    let w = unpack_w_blocks16(&call.job.w_image, k, n);
    let img = pack_c_i32(&gemm_i32(&a, &w));
    acc_from_obuf(call, &img)
}

/// Build the bypass job for `ql` applied to the quantized activation `xq`
/// (A at ABUF word 0, W at `ql.w_base`, OBUF word 0).
fn make_call(
    mode: u8,
    layer: Option<usize>,
    name: &'static str,
    ql: &QLinear,
    xq: &[i8],
) -> Result<LinearCall, String> {
    let a = MatI8::from_vec(1, xq.len(), xq.to_vec());
    let bases = Bases {
        a: 0,
        w: ql.w_base,
        out: 0,
        rq: 0,
    };
    let job = Job::new(mode, a, ql.w.clone(), None, bases)
        .map_err(|e| format!("{name} (layer {layer:?}): invalid job: {e}"))?;
    let expected_acc = job.acc.row(0).to_vec();
    Ok(LinearCall {
        layer,
        name,
        job,
        expected_acc,
    })
}

/// int8 decoder whose linear layers run through a caller-supplied executor.
pub struct Generator<'a> {
    pub q: &'a QModel,
    kv: KvCache,
    pub pos: usize,
    /// Descriptor mode of the jobs (MODE_FULL by default; the result is mode-independent).
    pub mode: u8,
}

impl<'a> Generator<'a> {
    pub fn new(q: &'a QModel) -> Self {
        Self {
            q,
            kv: KvCache::new(&q.cfg),
            pos: 0,
            mode: MODE_FULL,
        }
    }

    /// Run one decode step at the current position. `exec` must execute the call
    /// (on the board or in golden) and return the N int32 accumulators.
    pub fn step(
        &mut self,
        token: u32,
        exec: &mut dyn FnMut(&LinearCall) -> Result<Vec<i32>, String>,
    ) -> Result<Vec<f32>, String> {
        let q = self.q;
        let mode = self.mode;
        let mut run = |layer: Option<usize>,
                       name: &'static str,
                       ql: &QLinear,
                       xq: &[i8],
                       sx: f32|
         -> Result<Vec<f32>, String> {
            let call = make_call(mode, layer, name, ql, xq)?;
            let acc = exec(&call)?;
            if acc.len() != ql.n() {
                return Err(format!(
                    "{}: exec returned {} values, expected {}",
                    call.label(),
                    acc.len(),
                    ql.n()
                ));
            }
            Ok(dequantize(&acc, sx, &ql.scale))
        };
        let mut lin = |layer: Option<usize>, proj: Proj, x: &[f32]| -> Result<Vec<f32>, String> {
            let (xq, sx) = quantize_activation(x)?;
            match (layer, proj) {
                (None, Proj::Cls) => {
                    let mut out = Vec::with_capacity(q.cfg.vocab_size);
                    for (i, ql) in q.cls.iter().enumerate() {
                        out.extend(run(None, CLS_NAMES[i], ql, &xq, sx)?);
                    }
                    Ok(out)
                }
                (Some(l), pr) if pr != Proj::Cls => {
                    let s = pr.slot();
                    run(Some(l), LAYER_CALL_ORDER[s], &q.layers[l][s], &xq, sx)
                }
                _ => Err(format!("bad linear request {layer:?} {proj:?}")),
            }
        };
        let logits = forward_core(&q.cfg, &q.nl, &mut self.kv, token, self.pos, &mut lin)?;
        self.pos += 1;
        Ok(logits)
    }
}

/// `run.c sample_argmax`: first index of the maximum (strict `>`).
pub fn argmax(logits: &[f32]) -> u32 {
    let mut max_i = 0usize;
    let mut max_p = logits[0];
    for (i, &p) in logits.iter().enumerate().skip(1) {
        if p > max_p {
            max_i = i;
            max_p = p;
        }
    }
    max_i as u32
}

/// `run.c generate` with greedy sampling. Starting from `prompt[0]` at position 0,
/// calls `step(token)` up to `steps` times, forcing the prompt tokens and then
/// taking the argmax; stops early when the next token is BOS. Returns the token
/// sequence, starting with `prompt[0]` (decode pairs `(seq[i], seq[i+1])`).
pub fn generate_greedy(
    prompt: &[u32],
    steps: usize,
    step: &mut dyn FnMut(u32) -> Result<Vec<f32>, String>,
) -> Result<Vec<u32>, String> {
    let first = *prompt
        .first()
        .ok_or("empty prompt (expected at least BOS)")?;
    let mut seq = vec![first];
    let mut token = first;
    for pos in 0..steps {
        let logits = step(token)?;
        let next = if pos + 1 < prompt.len() {
            prompt[pos + 1]
        } else {
            argmax(&logits)
        };
        if next == BOS {
            break;
        }
        seq.push(next);
        token = next;
    }
    Ok(seq)
}
