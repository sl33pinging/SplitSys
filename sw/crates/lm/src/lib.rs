//! `splitsys_lm`: run a llama2.c-format tiny language model (karpathy/tinyllamas
//! stories260K) with every linear layer executed as a SplitSys GEMM.
//!
//! Reference implementation mirrored: `run.c` of https://github.com/karpathy/llama2.c
//! at commit 350e04fe35433e6d2941dce5a1f53308f87058eb (checkpoint layout:
//! `read_checkpoint` / `memory_map_weights`; forward pass: `forward`, `rmsnorm`,
//! `softmax`, `matmul`; tokenizer: `build_tokenizer`, `encode`, `decode`; greedy
//! sampling: `sample_argmax`; generation loop: `generate`).
//!
//! # Numerics of the int8 path (lead decision, G5)
//! * Weights: int8 symmetric **per output column**: `sw[n] = max_k |W[n][k]| / 127`
//!   (`1.0` for an all-zero column), `q = clamp(round_half_away(W / sw[n]), -127, 127)`.
//! * Activations: int8 symmetric **per tensor, per call**: `sx = max |x| / 127`
//!   (`1.0` if `x` is all zero), `q = clamp(round_half_away(x / sx), -127, 127)`.
//!   A non-finite activation is an error.
//! * The GEMM (M=1, K=in, N=out ≤ N_MAX) runs in the int32 bypass
//!   (`FLAGS_INT32`): no requant parameters are involved.
//! * Dequantization: `y[n] = (acc[n] as f32) * (sx * sw[n])` (f32, this exact
//!   association). `|acc| ≤ K·127·127 < 2^24`, so `acc as f32` is exact.
//! * Everything else (embedding lookup, RMSNorm, RoPE, attention, SwiGLU,
//!   residuals, argmax) is f32 on the host, in the same operation order as `run.c`.
//! * The classifier (shared with the token embedding when `vocab_size > 0`) is
//!   quantized like any other linear layer and split into chunks of `N_MAX`
//!   columns (`cls0`, `cls1`, ...). The embedding *lookup* uses the f32 table.

pub mod forward;
pub mod model;
pub mod quant;
pub mod tokenizer;

pub use forward::{
    acc_from_obuf, argmax, generate_greedy, golden_exec, reference_step_f32, F32Generator,
    Generator, KvCache, LinearCall, Proj,
};
pub use model::{Config, LayerWeights, Model, NonLinear};
pub use quant::{
    check_fits, dequantize, layer_layout, quantize_activation, LayerSlot, QLinear, QModel,
    CLS_NAMES, LAYER_CALL_ORDER,
};
pub use tokenizer::{Tokenizer, BOS};
