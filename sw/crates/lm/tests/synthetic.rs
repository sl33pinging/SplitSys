//! Tests that need no downloaded data: a seeded synthetic llama2.c model with the
//! stories260K shapes, and a hand-built tokenizer.

use splitsys_golden::consts::{FLAGS_INT32, MODE_FULL, MODE_STRIP, N_MAX, WBUF_WORDS};
use splitsys_golden::pack::w_words;
use splitsys_lm::*;

const SEED: u64 = 0x5EED_260C;

fn cfg() -> Config {
    Config {
        dim: 64,
        hidden_dim: 172,
        n_layers: 2,
        n_heads: 8,
        n_kv_heads: 4,
        vocab_size: 512,
        seq_len: 32,
    }
}

fn model() -> Model {
    eprintln!("synthetic model seed = {SEED:#x}");
    Model::synthetic(cfg(), SEED)
}

#[test]
fn bin_roundtrip_and_header() {
    let m = model();
    let b = m.to_bytes();
    let (c, shared) = Config::parse_header(&b).unwrap();
    assert_eq!(c, cfg());
    assert!(shared);
    let m2 = Model::from_bytes(&b).unwrap();
    assert_eq!(m, m2);
    // wrong length is rejected, not truncated
    assert!(Model::from_bytes(&b[..b.len() - 4]).is_err());
    let mut longer = b.clone();
    longer.extend_from_slice(&[0; 4]);
    assert!(Model::from_bytes(&longer).is_err());
    // negative vocab = unshared classifier: the file must then carry wcls
    let mut un = m.clone();
    un.shared_classifier = false;
    un.wcls = un.wcls.iter().map(|v| v * 0.5).collect();
    let b3 = un.to_bytes();
    assert_eq!(b3.len(), b.len() + 4 * 512 * 64);
    let m3 = Model::from_bytes(&b3).unwrap();
    assert!(!m3.shared_classifier);
    assert_eq!(m3.wcls, un.wcls);
    // garbage header
    let mut bad = b.clone();
    bad[12..16].copy_from_slice(&7i32.to_le_bytes()); // n_heads = 7
    assert!(Model::from_bytes(&bad).is_err());
}

#[test]
fn layout_fits_wbuf_with_distinct_bases() {
    let c = Config {
        n_layers: 5,
        seq_len: 512,
        ..cfg()
    };
    check_fits(&c).unwrap();
    let slots = layer_layout(&c).unwrap();
    let names: Vec<_> = slots.iter().map(|s| s.name).collect();
    assert_eq!(names, LAYER_CALL_ORDER);
    let mut end = 0u32;
    for s in &slots {
        assert_eq!(s.w_base, end, "{} not packed back to back", s.name);
        assert_eq!(s.w_words, w_words(s.k, s.n));
        end += s.w_words as u32;
    }
    assert_eq!(end, 2864); // 256+128+128+256+704+704+688
    assert!(end as usize <= WBUF_WORDS);
    // too-wide hidden layer is refused
    assert!(check_fits(&Config {
        hidden_dim: 300,
        ..c
    })
    .is_err());
}

#[test]
fn quantization_bounds() {
    let m = model();
    let q = QModel::quantize(&m);
    assert_eq!(q.cls.len(), 2);
    assert_eq!(q.cls[0].n(), N_MAX);
    assert_eq!(q.cls[1].w_base as usize, w_words(64, 256));
    // every weight reconstructs within half a step of its column scale
    let lw = &m.layers[1];
    let ql = &q.layers[1][6]; // w2: K=172, N=64
    assert_eq!((ql.k(), ql.n()), (172, 64));
    for n in 0..64 {
        for k in 0..172 {
            let w = lw.w2[n * 172 + k];
            let r = f32::from(ql.w.get(k, n)) * ql.scale[n];
            assert!((w - r).abs() <= 0.5 * ql.scale[n] * 1.0001, "w2[{n}][{k}]");
        }
    }
    let (xq, sx) = quantize_activation(&[0.5, -1.27, 0.0, 0.005]).unwrap();
    assert_eq!(xq, vec![50, -127, 0, 1]);
    assert!((sx - 0.01).abs() < 1e-7);
    assert_eq!(quantize_activation(&[0.0; 3]).unwrap(), (vec![0; 3], 1.0));
    assert!(quantize_activation(&[f32::NAN]).is_err());
    assert_eq!(dequantize(&[100, -3], 0.5, &[0.25, 2.0]), vec![12.5, -3.0]);
}

/// Run `steps` greedy steps with golden as the accelerator, checking every call.
fn run_golden(q: &QModel, mode: u8, steps: usize) -> (Vec<u32>, usize) {
    let mut g = Generator::new(q);
    g.mode = mode;
    let mut calls = 0usize;
    let mut exec = |c: &LinearCall| -> Result<Vec<i32>, String> {
        calls += 1;
        let d = &c.job.desc;
        assert_eq!(d.flags, FLAGS_INT32, "{}", c.label());
        assert_eq!(d.mode, mode);
        d.validate().map_err(|e| e.to_string())?;
        let (m, k, n) = d.dims();
        assert_eq!(m, 1);
        assert_eq!(k, c.job.a.cols());
        assert!(n <= N_MAX);
        assert!(c.job.rq_image.is_empty());
        let got = golden_exec(c)?;
        c.check(&got)?;
        assert!(c.job.check_obuf(&c.job.obuf_image).is_ok());
        Ok(got)
    };
    let seq = generate_greedy(&[BOS], steps, &mut |t| g.step(t, &mut exec)).unwrap();
    (seq, calls)
}

#[test]
fn golden_exec_is_bit_exact_and_deterministic() {
    let q = QModel::quantize(&model());
    let (a, calls) = run_golden(&q, MODE_FULL, 6);
    let (b, _) = run_golden(&q, MODE_FULL, 6);
    let (c, _) = run_golden(&q, MODE_STRIP, 6);
    eprintln!("synthetic token stream: {a:?}");
    assert_eq!(a, b);
    assert_eq!(a, c, "mode must not change the result");
    let fwd = a.len().min(6);
    assert_eq!(calls, fwd * (7 * 2 + 2));
}

#[test]
fn call_order_and_names() {
    let q = QModel::quantize(&model());
    let mut g = Generator::new(&q);
    let mut seen = Vec::new();
    let mut bases = std::collections::HashMap::new();
    g.step(5, &mut |c| {
        seen.push(c.label());
        bases.insert(c.label(), c.job.desc.w_base);
        golden_exec(c)
    })
    .unwrap();
    let want: Vec<String> = (0..2)
        .flat_map(|l| LAYER_CALL_ORDER.iter().map(move |n| format!("L{l}.{n}")))
        .chain(["cls0".to_string(), "cls1".to_string()])
        .collect();
    assert_eq!(seen, want);
    // within a layer every matrix has its own w_base
    let l0: std::collections::HashSet<_> = LAYER_CALL_ORDER
        .iter()
        .map(|n| bases[&format!("L0.{n}")])
        .collect();
    assert_eq!(l0.len(), 7);
    assert_ne!(bases["cls0"], bases["cls1"]);
}

#[test]
fn exec_errors_propagate() {
    let q = QModel::quantize(&model());
    let mut g = Generator::new(&q);
    assert!(g.step(0, &mut |_| Err("board timeout".into())).is_err());
    let mut g = Generator::new(&q);
    assert!(g.step(0, &mut |_| Ok(vec![0; 3])).is_err());
    let mut g = Generator::new(&q);
    assert!(g.step(512, &mut golden_exec).is_err()); // token out of range
}

#[test]
fn int8_tracks_f32_on_synthetic() {
    let m = model();
    let q = QModel::quantize(&m);
    let mut f = F32Generator::new(&m);
    let mut g = Generator::new(&q);
    let mut tok = BOS;
    let mut worst = 0.0f32;
    for _ in 0..8 {
        let lf = f.step(tok).unwrap();
        let lq = g.step(tok, &mut golden_exec).unwrap();
        let scale = lf.iter().fold(0.0f32, |a, v| a.max(v.abs()));
        let err = lf
            .iter()
            .zip(&lq)
            .fold(0.0f32, |a, (x, y)| a.max((x - y).abs()));
        worst = worst.max(err / scale);
        tok = argmax(&lf);
    }
    eprintln!("synthetic worst relative logit error: {worst}");
    assert!(worst < 0.1, "int8 path far from f32: {worst}");
}

#[test]
fn argmax_first_max_wins() {
    assert_eq!(argmax(&[1.0, 3.0, 3.0, -1.0]), 1);
    assert_eq!(argmax(&[f32::NEG_INFINITY, -5.0]), 1);
}

/// Build a tokenizer.bin image.
fn tok_bin(pieces: &[(&[u8], f32)]) -> Vec<u8> {
    let mut b = 8i32.to_le_bytes().to_vec();
    for (p, s) in pieces {
        b.extend_from_slice(&s.to_le_bytes());
        b.extend_from_slice(&(p.len() as i32).to_le_bytes());
        b.extend_from_slice(p);
    }
    b
}

#[test]
fn tokenizer_mirrors_run_c() {
    // ids 0..2 = <unk>,<s>,</s>; 3..258 = byte pieces <0x00>..<0xFF>; then merges.
    let mut owned: Vec<(Vec<u8>, f32)> = vec![
        (b"<unk>".to_vec(), 0.0),
        (b"\n<s>\n".to_vec(), 0.0),
        (b"\n</s>\n".to_vec(), 0.0),
    ];
    for i in 0..256 {
        owned.push((format!("<0x{i:02X}>").into_bytes(), 0.0));
    }
    let base = owned.len() as u32; // 259
    for (p, s) in [
        (" ", -1.0),
        ("a", -2.0),
        ("b", -2.0),
        (" a", -0.5),
        ("ab", -0.1),
        (" ab", -0.2),
    ] {
        owned.push((p.as_bytes().to_vec(), s));
    }
    let pieces: Vec<(&[u8], f32)> = owned.iter().map(|(p, s)| (p.as_slice(), *s)).collect();
    let t = Tokenizer::from_bytes(&tok_bin(&pieces), pieces.len()).unwrap();
    let (sp, a, b, sp_a, ab, sp_ab) = (base, base + 1, base + 2, base + 3, base + 4, base + 5);
    // " ab": [ ,a,b] -> best pair "ab"(-0.1) -> [ ,ab] -> " ab"
    assert_eq!(t.encode("ab", true), vec![BOS, sp_ab]);
    assert_eq!(t.encode("", true), vec![BOS]);
    assert_eq!(t.encode("", false), Vec::<u32>::new());
    assert_eq!(t.encode("b", false), vec![sp, b]);
    // byte fallback for an unknown codepoint: 'é' = C3 A9 -> ids 0xC3+3, 0xA9+3
    assert_eq!(t.encode("é", false), vec![sp, 0xC3 + 3, 0xA9 + 3]);
    let _ = (a, sp_a, ab);
    // decode: strip the leading space after BOS; byte pieces become raw bytes
    assert_eq!(t.decode(BOS, sp_ab), "ab");
    assert_eq!(t.decode(a, sp_ab), " ab");
    assert_eq!(t.decode_bytes(a, 0x41 + 3), b"A".to_vec());
    assert_eq!(t.decode_safe(a, 0x07 + 3), Vec::<u8>::new()); // BEL is dropped
    assert_eq!(t.decode_safe(a, b'\n' as u32 + 3), b"\n".to_vec());
    // truncated file
    let bin = tok_bin(&pieces);
    assert!(Tokenizer::from_bytes(&bin[..bin.len() - 1], pieces.len()).is_err());
}
