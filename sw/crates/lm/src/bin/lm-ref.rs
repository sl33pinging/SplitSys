//! lm-ref: run stories260K on the host, f32 or int8 (every linear layer as a
//! SplitSys GEMM executed by golden), print the text and the per-token GEMM traffic.
//!
//! cargo run -p splitsys-lm --bin lm-ref -- --prompt "Once upon a time" --tokens 40 [--f32|--int8]
//!     [--model models/stories260K/stories260K.bin] [--tokenizer models/stories260K/tok512.bin]

use splitsys_lm::{
    generate_greedy, golden_exec, F32Generator, Generator, Model, QModel, Tokenizer,
};
use std::path::PathBuf;
use std::process::ExitCode;

fn default_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../models/stories260K")
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lm-ref: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut prompt = String::new();
    let mut tokens = 40usize;
    let mut int8 = true;
    let mut model = default_dir().join("stories260K.bin");
    let mut tok = default_dir().join("tok512.bin");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = |n: &str| args.next().ok_or(format!("{n} needs a value"));
        match a.as_str() {
            "--prompt" => prompt = val("--prompt")?,
            "--tokens" => {
                tokens = val("--tokens")?
                    .parse()
                    .map_err(|e| format!("--tokens: {e}"))?
            }
            "--f32" => int8 = false,
            "--int8" => int8 = true,
            "--model" => model = PathBuf::from(val("--model")?),
            "--tokenizer" => tok = PathBuf::from(val("--tokenizer")?),
            "-h" | "--help" => {
                println!("lm-ref [--prompt TEXT] [--tokens N] [--f32|--int8] [--model PATH] [--tokenizer PATH]");
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let m = Model::load(&model).map_err(|e| format!("{e} (run models/fetch_stories260k.sh)"))?;
    let t = Tokenizer::load(&tok, m.cfg.vocab_size)?;
    let ptoks = t.encode(&prompt, true);
    let steps = tokens.min(m.cfg.seq_len);
    eprintln!("config: {:?}", m.cfg);
    eprintln!("prompt tokens: {ptoks:?}");

    #[derive(Default)]
    struct Traffic {
        calls: usize,
        w: usize,
        a: usize,
        out_extent: usize,
        out_inbounds: usize,
        desc: usize,
    }
    let mut tr = Traffic::default();
    let mut mism = 0usize;
    let n_fwd;
    let seq = if int8 {
        let q = QModel::quantize(&m);
        let mut g = Generator::new(&q);
        let mut exec = |c: &splitsys_lm::LinearCall| {
            tr.calls += 1;
            tr.w += c.job.w_image.len();
            tr.a += c.job.a_image.len();
            tr.out_extent += c.job.obuf_image.len();
            tr.out_inbounds += c.job.compare.iter().filter(|&&b| b).count();
            tr.desc += c.job.desc.to_bytes().len();
            let got = golden_exec(c)?;
            if c.check(&got).is_err() {
                mism += 1;
            }
            Ok(got)
        };
        let s = generate_greedy(&ptoks, steps, &mut |tk| g.step(tk, &mut exec))?;
        n_fwd = g.pos;
        s
    } else {
        let mut g = F32Generator::new(&m);
        let s = generate_greedy(&ptoks, steps, &mut |tk| g.step(tk))?;
        n_fwd = g.pos;
        s
    };
    let mut text = Vec::new();
    for w in seq.windows(2) {
        text.extend(t.decode_safe(w[0], w[1]));
    }
    println!("{}", String::from_utf8_lossy(&text));
    eprintln!(
        "mode: {}  forward passes: {n_fwd}  tokens: {:?}",
        if int8 { "int8 (golden exec)" } else { "f32" },
        seq
    );
    if int8 {
        let per = |v: usize| v as f64 / n_fwd.max(1) as f64;
        eprintln!("GEMM calls/token: {:.0}", per(tr.calls));
        eprintln!("bytes/token: weights {:.0}, activations {:.0}, descriptors {:.0}, int32 read-back {:.0} in-bounds ({:.0} full extent)",
            per(tr.w), per(tr.a), per(tr.desc), per(tr.out_inbounds), per(tr.out_extent));
        let total = per(tr.w) + per(tr.a) + per(tr.desc) + per(tr.out_inbounds);
        eprintln!("payload bytes/token (in-bounds read-back): {total:.0}  => ~{:.1} s/token at 10 KB/s (payload only, no framing)", total / 10_000.0);
        eprintln!("golden_exec vs expected_acc mismatches: {mism}");
    }
    Ok(())
}
