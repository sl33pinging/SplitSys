//! llama2.c checkpoint (`.bin`) loader.
//!
//! Layout (llama2.c `run.c` @ 350e04fe, `read_checkpoint` + `memory_map_weights`):
//! a 7 × int32 little-endian header `dim, hidden_dim, n_layers, n_heads,
//! n_kv_heads, vocab_size, seq_len` (a negative `vocab_size` means the classifier
//! is NOT shared with the token embedding), then f32 tensors in this order:
//!
//! | tensor                 | floats                                   |
//! |------------------------|------------------------------------------|
//! | token_embedding_table  | vocab · dim                              |
//! | rms_att_weight         | n_layers · dim                           |
//! | wq                     | n_layers · dim · (n_heads · head_size)   |
//! | wk                     | n_layers · dim · (n_kv_heads · head_size)|
//! | wv                     | n_layers · dim · (n_kv_heads · head_size)|
//! | wo                     | n_layers · (n_heads · head_size) · dim   |
//! | rms_ffn_weight         | n_layers · dim                           |
//! | w1                     | n_layers · dim · hidden_dim              |
//! | w2                     | n_layers · hidden_dim · dim              |
//! | w3                     | n_layers · dim · hidden_dim              |
//! | rms_final_weight       | dim                                      |
//! | (freq_cis_real, imag)  | 2 · seq_len · head_size / 2 (skipped)    |
//! | wcls (only if unshared)| vocab · dim                              |
//!
//! Every matrix is stored `[out][in]` row-major (`matmul(xout, x, w, n, d)`:
//! `xout[i] = Σ_j w[i*n + j] · x[j]`, W is `d × n`).
//!
//! The file is untrusted data: every size is computed with checked arithmetic and
//! the file length must match the header exactly before anything is allocated.

use std::path::Path;

/// Model hyper-parameters, exactly the llama2.c header (with `vocab_size` made positive).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub dim: usize,
    pub hidden_dim: usize,
    pub n_layers: usize,
    pub n_heads: usize,
    pub n_kv_heads: usize,
    pub vocab_size: usize,
    pub seq_len: usize,
}

impl Config {
    /// Header size in bytes (7 × int32).
    pub const HEADER_BYTES: usize = 28;

    pub fn head_size(&self) -> usize {
        self.dim / self.n_heads
    }
    /// `(dim * n_kv_heads) / n_heads`, as in `run.c`.
    pub fn kv_dim(&self) -> usize {
        self.dim * self.n_kv_heads / self.n_heads
    }
    /// `n_heads / n_kv_heads` (query heads per kv head).
    pub fn kv_mul(&self) -> usize {
        self.n_heads / self.n_kv_heads
    }

    /// Parse the 28-byte header. Returns the config and whether the classifier is
    /// shared with the token embedding (`vocab_size > 0`).
    pub fn parse_header(b: &[u8]) -> Result<(Config, bool), String> {
        if b.len() < Self::HEADER_BYTES {
            return Err(format!(
                "checkpoint too short for header: {} bytes",
                b.len()
            ));
        }
        let int =
            |i: usize| i32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]);
        let pos = |name: &str, v: i32| -> Result<usize, String> {
            if v <= 0 {
                Err(format!("header field {name} = {v} must be > 0"))
            } else {
                Ok(v as usize)
            }
        };
        let vocab_raw = int(5);
        let shared = vocab_raw > 0;
        let cfg = Config {
            dim: pos("dim", int(0))?,
            hidden_dim: pos("hidden_dim", int(1))?,
            n_layers: pos("n_layers", int(2))?,
            n_heads: pos("n_heads", int(3))?,
            n_kv_heads: pos("n_kv_heads", int(4))?,
            vocab_size: pos("vocab_size", vocab_raw.checked_abs().unwrap_or(0))?,
            seq_len: pos("seq_len", int(6))?,
        };
        cfg.check()?;
        Ok((cfg, shared))
    }

    /// Structural sanity (what `run.c` silently assumes).
    pub fn check(&self) -> Result<(), String> {
        if !self.dim.is_multiple_of(self.n_heads) {
            return Err(format!(
                "dim {} not a multiple of n_heads {}",
                self.dim, self.n_heads
            ));
        }
        if !self.n_heads.is_multiple_of(self.n_kv_heads) {
            return Err(format!(
                "n_heads {} not a multiple of n_kv_heads {}",
                self.n_heads, self.n_kv_heads
            ));
        }
        if !self.head_size().is_multiple_of(2) {
            return Err(format!(
                "head_size {} is odd (RoPE needs pairs)",
                self.head_size()
            ));
        }
        if self.vocab_size < 3 {
            return Err("vocab_size < 3 (BOS/EOS ids would be invalid)".into());
        }
        Ok(())
    }

    /// Number of f32 values after the header, or None on overflow.
    fn n_floats(&self, shared: bool) -> Option<usize> {
        let (d, h, l, v, s) = (
            self.dim,
            self.hidden_dim,
            self.n_layers,
            self.vocab_size,
            self.seq_len,
        );
        let hs = self.head_size();
        let kv = self.n_kv_heads.checked_mul(hs)?;
        let q = self.n_heads.checked_mul(hs)?;
        let mut t = v.checked_mul(d)?;
        let mut add = |x: Option<usize>| -> Option<()> {
            t = t.checked_add(x?)?;
            Some(())
        };
        add(l.checked_mul(d))?; // rms_att
        add(l.checked_mul(d)?.checked_mul(q))?; // wq
        add(l.checked_mul(d)?.checked_mul(kv))?; // wk
        add(l.checked_mul(d)?.checked_mul(kv))?; // wv
        add(l.checked_mul(q)?.checked_mul(d))?; // wo
        add(l.checked_mul(d))?; // rms_ffn
        add(l.checked_mul(d)?.checked_mul(h))?; // w1
        add(l.checked_mul(h)?.checked_mul(d))?; // w2
        add(l.checked_mul(d)?.checked_mul(h))?; // w3
        add(Some(d))?; // rms_final
        add(s.checked_mul(hs))?; // freq_cis real + imag (2 · seq_len · head_size/2)
        if !shared {
            add(v.checked_mul(d))?;
        }
        Some(t)
    }
}

/// The f32 parameters that never go through the accelerator.
#[derive(Clone, Debug, PartialEq)]
pub struct NonLinear {
    /// `vocab × dim`, row `t` is token `t`'s embedding.
    pub token_embedding: Vec<f32>,
    /// Per layer, `dim` each.
    pub rms_att: Vec<Vec<f32>>,
    pub rms_ffn: Vec<Vec<f32>>,
    /// `dim`.
    pub rms_final: Vec<f32>,
}

/// One layer's linear weights, each `[out][in]` row-major (llama2.c layout).
#[derive(Clone, Debug, PartialEq)]
pub struct LayerWeights {
    /// `dim × dim`
    pub wq: Vec<f32>,
    /// `kv_dim × dim`
    pub wk: Vec<f32>,
    /// `kv_dim × dim`
    pub wv: Vec<f32>,
    /// `dim × dim`
    pub wo: Vec<f32>,
    /// `hidden × dim`
    pub w1: Vec<f32>,
    /// `dim × hidden`
    pub w2: Vec<f32>,
    /// `hidden × dim`
    pub w3: Vec<f32>,
}

/// A float llama2.c model.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub cfg: Config,
    /// True when the classifier is the token embedding table (`vocab_size > 0` in the header).
    pub shared_classifier: bool,
    pub nl: NonLinear,
    pub layers: Vec<LayerWeights>,
    /// Classifier `vocab × dim` (a copy of the embedding when shared).
    pub wcls: Vec<f32>,
}

impl Model {
    /// Load a llama2.c `.bin` checkpoint.
    pub fn load(bin: &Path) -> Result<Model, String> {
        let bytes = std::fs::read(bin).map_err(|e| format!("read {}: {e}", bin.display()))?;
        Self::from_bytes(&bytes)
    }

    /// Parse a checkpoint image (see the module docs for the layout).
    pub fn from_bytes(b: &[u8]) -> Result<Model, String> {
        let (cfg, shared) = Config::parse_header(b)?;
        let n = cfg
            .n_floats(shared)
            .ok_or("checkpoint header implies a size that overflows usize")?;
        let want = n
            .checked_mul(4)
            .and_then(|x| x.checked_add(Config::HEADER_BYTES))
            .ok_or("checkpoint size overflows usize")?;
        if b.len() != want {
            return Err(format!(
                "checkpoint is {} bytes but the header {cfg:?} (shared={shared}) implies {want}",
                b.len()
            ));
        }
        let mut off = Config::HEADER_BYTES;
        let mut take = |count: usize| -> Vec<f32> {
            let v = b[off..off + 4 * count]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect();
            off += 4 * count;
            v
        };
        let (d, h, l, v) = (cfg.dim, cfg.hidden_dim, cfg.n_layers, cfg.vocab_size);
        let (q, kv) = (cfg.n_heads * cfg.head_size(), cfg.kv_dim());
        let split = |all: Vec<f32>, per: usize| -> Vec<Vec<f32>> {
            all.chunks_exact(per).map(<[f32]>::to_vec).collect()
        };
        let token_embedding = take(v * d);
        let rms_att = split(take(l * d), d);
        let wq = split(take(l * d * q), d * q);
        let wk = split(take(l * d * kv), d * kv);
        let wv = split(take(l * d * kv), d * kv);
        let wo = split(take(l * q * d), q * d);
        let rms_ffn = split(take(l * d), d);
        let w1 = split(take(l * d * h), d * h);
        let w2 = split(take(l * h * d), h * d);
        let w3 = split(take(l * d * h), d * h);
        let rms_final = take(d);
        let _freq_cis = take(cfg.seq_len * cfg.head_size());
        let wcls = if shared {
            token_embedding.clone()
        } else {
            take(v * d)
        };
        debug_assert_eq!(off, b.len());
        let layers = (0..l)
            .map(|i| LayerWeights {
                wq: wq[i].clone(),
                wk: wk[i].clone(),
                wv: wv[i].clone(),
                wo: wo[i].clone(),
                w1: w1[i].clone(),
                w2: w2[i].clone(),
                w3: w3[i].clone(),
            })
            .collect();
        Ok(Model {
            cfg,
            shared_classifier: shared,
            nl: NonLinear {
                token_embedding,
                rms_att,
                rms_ffn,
                rms_final,
            },
            layers,
            wcls,
        })
    }

    /// Serialize back to the llama2.c `.bin` format (used by tests to build
    /// synthetic checkpoints; the freq_cis block is written as zeros).
    pub fn to_bytes(&self) -> Vec<u8> {
        let c = &self.cfg;
        let vocab = if self.shared_classifier {
            c.vocab_size as i32
        } else {
            -(c.vocab_size as i32)
        };
        let mut out = Vec::new();
        for x in [
            c.dim as i32,
            c.hidden_dim as i32,
            c.n_layers as i32,
            c.n_heads as i32,
            c.n_kv_heads as i32,
            vocab,
            c.seq_len as i32,
        ] {
            out.extend_from_slice(&x.to_le_bytes());
        }
        let mut put = |v: &[f32]| {
            v.iter()
                .for_each(|f| out.extend_from_slice(&f.to_le_bytes()))
        };
        put(&self.nl.token_embedding);
        self.nl.rms_att.iter().for_each(|x| put(x));
        self.layers.iter().for_each(|x| put(&x.wq));
        self.layers.iter().for_each(|x| put(&x.wk));
        self.layers.iter().for_each(|x| put(&x.wv));
        self.layers.iter().for_each(|x| put(&x.wo));
        self.nl.rms_ffn.iter().for_each(|x| put(x));
        self.layers.iter().for_each(|x| put(&x.w1));
        self.layers.iter().for_each(|x| put(&x.w2));
        self.layers.iter().for_each(|x| put(&x.w3));
        put(&self.nl.rms_final);
        put(&vec![0.0; c.seq_len * c.head_size()]);
        if !self.shared_classifier {
            put(&self.wcls);
        }
        out
    }

    /// A random model (seeded) for tests that must not depend on downloaded data.
    pub fn synthetic(cfg: Config, seed: u64) -> Model {
        cfg.check().expect("synthetic: bad config");
        let mut rng = splitsys_golden::rng::Rng::new(seed);
        let mut gen = |n: usize, scale: f32| -> Vec<f32> {
            (0..n)
                .map(|_| (rng.range_i64(-1000, 1000) as f32 / 1000.0) * scale)
                .collect()
        };
        let (d, h, v) = (cfg.dim, cfg.hidden_dim, cfg.vocab_size);
        let kv = cfg.kv_dim();
        let token_embedding = gen(v * d, 1.0);
        let rms_att = (0..cfg.n_layers).map(|_| gen(d, 1.0)).collect();
        let rms_ffn = (0..cfg.n_layers).map(|_| gen(d, 1.0)).collect();
        let rms_final = gen(d, 1.0);
        let s_in = 1.0 / (d as f32).sqrt();
        let s_h = 1.0 / (h as f32).sqrt();
        let layers = (0..cfg.n_layers)
            .map(|_| LayerWeights {
                wq: gen(d * d, s_in),
                wk: gen(kv * d, s_in),
                wv: gen(kv * d, s_in),
                wo: gen(d * d, s_in),
                w1: gen(h * d, s_in),
                w2: gen(d * h, s_h),
                w3: gen(h * d, s_in),
            })
            .collect();
        Model {
            cfg,
            shared_classifier: true,
            wcls: token_embedding.clone(),
            nl: NonLinear {
                token_embedding,
                rms_att,
                rms_ffn,
                rms_final,
            },
            layers,
        }
    }
}
