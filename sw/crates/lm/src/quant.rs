//! int8 quantization of the linear layers and the WBUF layout of one layer.
//! See the crate docs for the exact numerics contract.

use crate::model::{Config, Model, NonLinear};
use splitsys_golden::consts::{K_MAX, K_MIN, N_MAX, N_MIN, WBUF_WORDS};
use splitsys_golden::pack::w_words;
use splitsys_golden::MatI8;

/// The 7 projections of a layer in execution order (`run.c forward`): q, k, v,
/// o, then w1, w3, w2.
pub const LAYER_CALL_ORDER: [&str; 7] = ["wq", "wk", "wv", "wo", "w1", "w3", "w2"];

/// Names of the classifier chunks (N_MAX columns each).
pub const CLS_NAMES: [&str; 8] = [
    "cls0", "cls1", "cls2", "cls3", "cls4", "cls5", "cls6", "cls7",
];

/// A quantized linear layer: GEMM operand `W` is `K × N` (K = in, N = out),
/// `scale[n]` is the per-output-column dequantization scale.
#[derive(Clone, Debug, PartialEq)]
pub struct QLinear {
    pub w: MatI8,
    pub scale: Vec<f32>,
    /// WBUF word base this matrix is assigned (distinct within a layer / the classifier).
    pub w_base: u32,
}

impl QLinear {
    pub fn k(&self) -> usize {
        self.w.rows()
    }
    pub fn n(&self) -> usize {
        self.w.cols()
    }

    /// Quantize rows `[n0, n1)` of an `[out][in]` f32 matrix with `inp` columns.
    pub fn quantize_rows(w_out_in: &[f32], inp: usize, n0: usize, n1: usize) -> QLinear {
        let n = n1 - n0;
        let mut scale = Vec::with_capacity(n);
        let mut q = MatI8::zeros(inp, n);
        for (j, row) in w_out_in.chunks_exact(inp).skip(n0).take(n).enumerate() {
            let amax = row.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
            let s = if amax > 0.0 { amax / 127.0 } else { 1.0 };
            for (k, &v) in row.iter().enumerate() {
                q.set(k, j, quant_one(v, s));
            }
            scale.push(s);
        }
        QLinear {
            w: q,
            scale,
            w_base: 0,
        }
    }
}

/// `clamp(round_half_away(v / s), -127, 127)`.
fn quant_one(v: f32, s: f32) -> i8 {
    (v / s).round().clamp(-127.0, 127.0) as i8
}

/// Symmetric per-tensor int8 activation quantization: returns (q, sx).
pub fn quantize_activation(x: &[f32]) -> Result<(Vec<i8>, f32), String> {
    let mut amax = 0.0f32;
    for &v in x {
        if !v.is_finite() {
            return Err(format!("non-finite activation {v}"));
        }
        amax = amax.max(v.abs());
    }
    let s = if amax > 0.0 { amax / 127.0 } else { 1.0 };
    Ok((x.iter().map(|&v| quant_one(v, s)).collect(), s))
}

/// `y[n] = (acc[n] as f32) * (sx * sw[n])`.
pub fn dequantize(acc: &[i32], sx: f32, sw: &[f32]) -> Vec<f32> {
    acc.iter()
        .zip(sw)
        .map(|(&a, &s)| a as f32 * (sx * s))
        .collect()
}

/// Where one projection of a layer lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerSlot {
    pub name: &'static str,
    pub k: usize,
    pub n: usize,
    pub w_base: u32,
    pub w_words: usize,
}

/// The (K, N) of each projection in `LAYER_CALL_ORDER`.
fn layer_shapes(c: &Config) -> [(usize, usize); 7] {
    let (d, kv, h) = (c.dim, c.kv_dim(), c.hidden_dim);
    [(d, d), (d, kv), (d, kv), (d, d), (d, h), (d, h), (h, d)]
}

/// WBUF layout of one layer's 7 matrices, packed back to back from word 0 in
/// call order. Errors if any shape is outside the descriptor ranges or the layer
/// does not fit WBUF.
pub fn layer_layout(c: &Config) -> Result<Vec<LayerSlot>, String> {
    let mut base = 0usize;
    let mut out = Vec::new();
    for (name, (k, n)) in LAYER_CALL_ORDER.iter().zip(layer_shapes(c)) {
        if !(K_MIN..=K_MAX).contains(&k) || !(N_MIN..=N_MAX).contains(&n) {
            return Err(format!(
                "{name}: K={k} N={n} outside K {K_MIN}..={K_MAX}, N {N_MIN}..={N_MAX}"
            ));
        }
        let ww = w_words(k, n);
        out.push(LayerSlot {
            name,
            k,
            n,
            w_base: base as u32,
            w_words: ww,
        });
        base += ww;
    }
    if base > WBUF_WORDS {
        return Err(format!("one layer needs {base} WBUF words > {WBUF_WORDS}"));
    }
    Ok(out)
}

/// Classifier chunks: (name, n0, n1, w_base), packed from word 0.
fn cls_layout(c: &Config) -> Result<Vec<(&'static str, usize, usize, u32)>, String> {
    if !(K_MIN..=K_MAX).contains(&c.dim) {
        return Err(format!("classifier K={} outside {K_MIN}..={K_MAX}", c.dim));
    }
    let chunks = c.vocab_size.div_ceil(N_MAX);
    if chunks > CLS_NAMES.len() {
        return Err(format!(
            "vocab {} needs {chunks} classifier chunks > {}",
            c.vocab_size,
            CLS_NAMES.len()
        ));
    }
    let mut base = 0usize;
    let mut out = Vec::new();
    for (i, name) in CLS_NAMES.iter().enumerate().take(chunks) {
        let (n0, n1) = (i * N_MAX, ((i + 1) * N_MAX).min(c.vocab_size));
        out.push((*name, n0, n1, base as u32));
        base += w_words(c.dim, n1 - n0);
    }
    if base > WBUF_WORDS {
        return Err(format!("classifier needs {base} WBUF words > {WBUF_WORDS}"));
    }
    Ok(out)
}

/// Does this config map onto the accelerator (all shapes legal, buffers fit)?
pub fn check_fits(c: &Config) -> Result<(), String> {
    layer_layout(c)?;
    cls_layout(c)?;
    Ok(())
}

/// A quantized model: per layer the 7 projections in `LAYER_CALL_ORDER`, the
/// classifier chunks, and the f32 non-linear parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct QModel {
    pub cfg: Config,
    pub nl: NonLinear,
    /// `layers[l][i]` is projection `LAYER_CALL_ORDER[i]` of layer `l`.
    pub layers: Vec<[QLinear; 7]>,
    /// Classifier chunks, `CLS_NAMES[i]` covers vocab columns `[i*N_MAX, ...)`.
    pub cls: Vec<QLinear>,
}

impl QModel {
    /// Quantize. Panics if the model does not fit the accelerator (use
    /// [`check_fits`] first to get an error instead).
    pub fn quantize(m: &Model) -> QModel {
        let c = &m.cfg;
        let slots = layer_layout(c).expect("QModel::quantize: layer does not fit");
        let cls = cls_layout(c).expect("QModel::quantize: classifier does not fit");
        let layers = m
            .layers
            .iter()
            .map(|lw| {
                let src: [&[f32]; 7] = [&lw.wq, &lw.wk, &lw.wv, &lw.wo, &lw.w1, &lw.w3, &lw.w2];
                std::array::from_fn(|i| {
                    let s = slots[i];
                    let mut q = QLinear::quantize_rows(src[i], s.k, 0, s.n);
                    q.w_base = s.w_base;
                    q
                })
            })
            .collect();
        let cls = cls
            .iter()
            .map(|&(_, n0, n1, base)| {
                let mut q = QLinear::quantize_rows(&m.wcls, c.dim, n0, n1);
                q.w_base = base;
                q
            })
            .collect();
        QModel {
            cfg: *c,
            nl: m.nl.clone(),
            layers,
            cls,
        }
    }

    /// Total int8 weight bytes as packed WBUF images (one full pass).
    pub fn packed_weight_bytes(&self) -> usize {
        let wb = splitsys_golden::consts::WBUF_WORD_BYTES;
        self.layers
            .iter()
            .flat_map(|l| l.iter())
            .chain(self.cls.iter())
            .map(|q| w_words(q.k(), q.n()) * wb)
            .sum()
    }
}
