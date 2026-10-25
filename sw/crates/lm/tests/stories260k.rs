//! Tests against the real karpathy/tinyllamas stories260K checkpoint, fetched by
//! `models/fetch_stories260k.sh` (sha256-pinned). If the files are absent the tests
//! print SKIP and pass, unless SPLITSYS_LM_REQUIRE_MODEL=1 (then they fail).
//! Override the directory with SPLITSYS_LM_MODEL_DIR.

use splitsys_golden::consts::{FLAGS_INT32, WBUF_WORDS};
use splitsys_lm::*;
use std::path::PathBuf;

/// Published greedy (temperature 0, no prompt) output of `./run stories260K.bin -z
/// tok512.bin -t 0.0` (256 steps, the run.c default), quoted verbatim from
/// https://huggingface.co/karpathy/tinyllamas/blob/0bd21da7698eaf29a0d7de3992de8a46ef624add/stories260K/readme.md
const README_GREEDY: &str = "Once upon a time, there was a little girl named Lily. She loved to play outside in the park. One day, she saw a big, red ball. She wanted to play with it, but it was too high.\n\
Lily's mom said, \"Lily, let's go to the park.\" Lily was sad and didn't know what to do. She said, \"I want to play with your ball, but I can't find it.\"\n\
Lily was sad and didn't know what to do. She said, \"I'm sorry, Lily. I didn't know what to do.\"\n\
Lily didn't want to help her mom, so she said, \"I'm sorry, mom. I didn't know what to do.\" Her mom said, \"Don't worry, Lily. We can help you.";

fn files() -> Option<(Model, Tokenizer)> {
    let dir = std::env::var_os("SPLITSYS_LM_MODEL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../models/stories260K")
        });
    let (bin, tok) = (dir.join("stories260K.bin"), dir.join("tok512.bin"));
    if !bin.exists() || !tok.exists() {
        if std::env::var("SPLITSYS_LM_REQUIRE_MODEL").as_deref() == Ok("1") {
            panic!(
                "model files missing in {} (run models/fetch_stories260k.sh)",
                dir.display()
            );
        }
        eprintln!(
            "SKIP: model files missing in {} (run models/fetch_stories260k.sh)",
            dir.display()
        );
        return None;
    }
    let m = Model::load(&bin).unwrap();
    let t = Tokenizer::load(&tok, m.cfg.vocab_size).unwrap();
    Some((m, t))
}

fn text(t: &Tokenizer, seq: &[u32]) -> String {
    let mut b = Vec::new();
    for w in seq.windows(2) {
        b.extend(t.decode_safe(w[0], w[1]));
    }
    String::from_utf8_lossy(&b).into_owned()
}

#[test]
fn config_matches_header() {
    let Some((m, t)) = files() else { return };
    let want = Config {
        dim: 64,
        hidden_dim: 172,
        n_layers: 5,
        n_heads: 8,
        n_kv_heads: 4,
        vocab_size: 512,
        seq_len: 512,
    };
    assert_eq!(m.cfg, want);
    assert!(
        m.shared_classifier,
        "vocab_size > 0 in the header => shared classifier"
    );
    assert_eq!(m.wcls, m.nl.token_embedding);
    assert_eq!(t.vocab_size(), 512);
    assert_eq!(t.distinct_pieces(), 512, "tok512 has duplicate pieces");
    check_fits(&m.cfg).unwrap();
}

#[test]
fn f32_greedy_reproduces_published_readme() {
    let Some((m, t)) = files() else { return };
    let mut g = F32Generator::new(&m);
    let seq = generate_greedy(&t.encode("", true), 256, &mut |tk| g.step(tk)).unwrap();
    assert_eq!(text(&t, &seq), README_GREEDY);
}

#[test]
fn f32_once_upon_a_time() {
    let Some((m, t)) = files() else { return };
    let p = t.encode("Once upon a time", true);
    eprintln!("prompt tokens: {p:?}");
    let mut g = F32Generator::new(&m);
    let seq = generate_greedy(&p, 40, &mut |tk| g.step(tk)).unwrap();
    let s = text(&t, &seq);
    eprintln!("f32 greedy, 40 steps: {s:?}");
    // The forced prompt decodes back to itself; the continuation is recorded, not asserted
    // beyond being the README story (same model, same greedy rule).
    assert!(s.starts_with("Once upon a time"));
    assert!(
        README_GREEDY.starts_with(&s),
        "continuation diverged from the README story: {s:?}"
    );
}

/// Teacher-forced top-1 agreement (both paths fed the f32 greedy stream) and
/// free-running divergence over 50 and 256 steps.
#[test]
fn int8_vs_f32_agreement() {
    for steps in [50, 256] {
        agreement(steps);
    }
}

fn agreement(steps: usize) {
    let Some((m, t)) = files() else { return };
    let q = QModel::quantize(&m);
    let mut f = F32Generator::new(&m);
    let fseq = generate_greedy(&[BOS], steps, &mut |tk| f.step(tk)).unwrap();

    let mut f = F32Generator::new(&m);
    let mut g = Generator::new(&q);
    let (mut agree, mut n, mut worst) = (0usize, 0usize, 0.0f32);
    for w in fseq.windows(2) {
        let lf = f.step(w[0]).unwrap();
        let lq = g.step(w[0], &mut golden_exec).unwrap();
        assert_eq!(argmax(&lf), w[1]);
        agree += usize::from(argmax(&lq) == argmax(&lf));
        n += 1;
        let scale = lf.iter().fold(0.0f32, |a, v| a.max(v.abs()));
        let err = lf
            .iter()
            .zip(&lq)
            .fold(0.0f32, |a, (x, y)| a.max((x - y).abs()));
        worst = worst.max(err / scale);
    }
    eprintln!("[{steps} steps] teacher-forced top-1 agreement: {agree}/{n}; worst max|dlogit|/max|logit| = {worst:.4}");

    let mut g = Generator::new(&q);
    let qseq = generate_greedy(&[BOS], steps, &mut |tk| g.step(tk, &mut golden_exec)).unwrap();
    let first_div = fseq.iter().zip(&qseq).position(|(a, b)| a != b);
    eprintln!("f32  free-run: {:?}", text(&t, &fseq));
    eprintln!("int8 free-run: {:?}", text(&t, &qseq));
    eprintln!(
        "[{steps} steps] first divergent token index: {first_div:?} (of {})",
        fseq.len()
    );
    assert!(
        agree * 10 >= n * 9,
        "top-1 agreement below 90%: {agree}/{n}"
    );
}

#[test]
fn golden_exec_stream_is_deterministic_and_valid() {
    let Some((m, _t)) = files() else { return };
    let q = QModel::quantize(&m);
    let run = || {
        let mut g = Generator::new(&q);
        let mut calls = 0usize;
        let mut wbytes = 0usize;
        let mut exec = |c: &LinearCall| -> Result<Vec<i32>, String> {
            calls += 1;
            wbytes += c.job.w_image.len();
            let d = &c.job.desc;
            assert_eq!(d.flags, FLAGS_INT32);
            d.validate().map_err(|e| format!("{}: {e}", c.label()))?;
            assert!(d.w_base as usize + c.job.w_image.len() / 16 <= WBUF_WORDS);
            let got = golden_exec(c)?;
            assert_eq!(got, c.expected_acc, "{}", c.label());
            Ok(got)
        };
        let seq = generate_greedy(&[BOS], 20, &mut |tk| g.step(tk, &mut exec)).unwrap();
        (seq, calls, wbytes)
    };
    let (a, calls, wbytes) = run();
    let (b, _, _) = run();
    assert_eq!(a, b);
    assert_eq!(calls, 20 * 37);
    assert_eq!(wbytes, 20 * 261_888);
    assert_eq!(q.packed_weight_bytes(), 261_888);
}
