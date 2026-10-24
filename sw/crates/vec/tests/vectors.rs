//! Determinism and self-consistency of generated vectors.

use serde_json::Value;
use splitsys_golden::consts::{MODE_FULL, MODE_STRIP};
use splitsys_golden::pack::{unpack_a_blocks8, unpack_rqparams, unpack_w_blocks16};
use splitsys_golden::{gemm_i32, pack_c_i32, pack_c_i8_blocks16, requant_mat, Descriptor};
use splitsys_vec::{bench, jobs, selftest, unit, Meta, OutFile};

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn parse(f: &OutFile) -> Value {
    serde_json::from_slice(&f.bytes).unwrap()
}

#[test]
fn same_seed_same_bytes_different_seed_different_bytes() {
    let m = Meta::test();
    for module in unit::MODULES {
        let a = unit::generate(module, 7, 50, &m).unwrap();
        let b = unit::generate(module, 7, 50, &m).unwrap();
        let c = unit::generate(module, 8, 50, &m).unwrap();
        assert_eq!(a, b, "unit {module} not deterministic");
        assert_ne!(a, c, "unit {module} ignores the seed");
    }
    for mode in [MODE_FULL, MODE_STRIP] {
        let a = jobs::gemm(3, 12, mode, 20, 40, 40, &m).unwrap();
        assert_eq!(a, jobs::gemm(3, 12, mode, 20, 40, 40, &m).unwrap());
        assert_ne!(a, jobs::gemm(4, 12, mode, 20, 40, 40, &m).unwrap());
    }
    assert_eq!(jobs::directed(&m), jobs::directed(&m));
    assert_eq!(selftest::selftest(&m), selftest::selftest(&m));
    assert_eq!(bench::bench(&m), bench::bench(&m));
}

/// Decode a job from its JSON alone (images + descriptor), recompute it with golden,
/// and compare with the stored expectation.
fn check_job_json(j: &Value) {
    let name = j["name"].as_str().unwrap();
    let d = Descriptor::from_bytes(&unhex(j["desc_hex"].as_str().unwrap())).unwrap();
    d.validate().unwrap();
    let (m, k, n) = d.dims();
    let mut a_img = None;
    let mut w_img = None;
    let mut rq_img = None;
    for l in j["load"].as_array().unwrap() {
        let bytes = unhex(l["hex"].as_str().unwrap());
        let addr = l["byte_addr"].as_u64().unwrap() as usize;
        match l["region_name"].as_str().unwrap() {
            "ABUF" => {
                assert_eq!(addr, d.a_base as usize * 8);
                a_img = Some(bytes)
            }
            "WBUF" => {
                assert_eq!(addr, d.w_base as usize * 16);
                w_img = Some(bytes)
            }
            "RQPARAM" => {
                assert_eq!(addr, usize::from(d.rq_base) * 4);
                rq_img = Some(bytes)
            }
            r => panic!("unexpected region {r}"),
        }
    }
    let a = unpack_a_blocks8(&a_img.unwrap(), m, k);
    let w = unpack_w_blocks16(&w_img.unwrap(), k, n);
    let acc = gemm_i32(&a, &w);
    let img = match rq_img {
        Some(rq) => {
            assert!(d.requant_en(), "{name}");
            pack_c_i8_blocks16(&requant_mat(&acc, &unpack_rqparams(&rq, n)))
        }
        None => {
            assert!(!d.requant_en(), "{name}");
            pack_c_i32(&acc)
        }
    };
    let e = &j["expected"];
    assert_eq!(
        e["obuf_byte_addr"].as_u64().unwrap() as usize,
        d.out_base as usize * 16
    );
    assert_eq!(unhex(e["obuf_hex"].as_str().unwrap()), img, "{name}");
    let ranges = e["compare_ranges"].as_array().unwrap();
    assert!(!ranges.is_empty(), "{name}");
}

#[test]
fn job_json_round_trips_through_golden() {
    let m = Meta::test();
    for mode in [MODE_FULL, MODE_STRIP] {
        let f = &jobs::gemm(11, 30, mode, 64, 64, 64, &m).unwrap()[0];
        for j in parse(f)["jobs"].as_array().unwrap() {
            check_job_json(j);
            assert_eq!(j["mode"], if mode == MODE_FULL { "full" } else { "strip" });
        }
    }
    let d = parse(&jobs::directed(&m)[0]);
    let all = d["jobs"].as_array().unwrap();
    assert!(all.len() > 200);
    for j in all {
        check_job_json(j);
    }
    // Every sequence references existing jobs.
    for s in d["sequences"].as_array().unwrap() {
        for name in s["jobs"].as_array().unwrap() {
            assert!(all.iter().any(|j| j["name"] == *name), "missing {name}");
        }
    }
    let b = parse(&bench::bench(&m)[0]);
    let entries = b["entries"].as_array().unwrap();
    assert_eq!(
        entries.len(),
        splitsys_spec::BENCH_SHAPES.len() * splitsys_spec::BENCH_M_SWEEP.len()
    );
    for e in entries {
        check_job_json(&e["job"]);
    }
}

#[test]
fn directed_single_nonzero_has_exactly_one_nonzero_output() {
    let d = parse(&jobs::directed(&Meta::test())[0]);
    let mut seen = 0;
    for j in d["jobs"].as_array().unwrap() {
        let name = j["name"].as_str().unwrap();
        if !name.contains("_single_nz_") {
            continue;
        }
        seen += 1;
        let acc = j["expected"]["acc"].as_array().unwrap();
        let nz = acc
            .iter()
            .flat_map(|r| r.as_array().unwrap())
            .filter(|v| v.as_i64().unwrap() != 0)
            .count();
        assert_eq!(nz, 1, "{name}");
    }
    assert_eq!(seen, 8 * 16 + 4 * 16);
}

#[test]
fn pe_vectors_are_consistent() {
    let f = &unit::generate("pe", 5, 200, &Meta::test()).unwrap()[0];
    let v = parse(f);
    let get = |k: &str| -> Vec<i64> {
        v[k].as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_i64().unwrap())
            .collect()
    };
    let (a, w, vld, first, acc) = (get("a"), get("w"), get("vld"), get("first"), get("acc"));
    let mut s: i64 = 0;
    for t in 0..a.len() {
        if vld[t] == 1 {
            s = if first[t] == 1 {
                a[t] * w[t]
            } else {
                s + a[t] * w[t]
            };
        }
        assert_eq!(acc[t], s, "cycle {t}");
    }
    let groups = v["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 8 + 16 + 8 + 200);
    assert_eq!(groups[0]["acc"], 256 * 16384);
    assert_eq!(groups[0]["name"], "neg128_x_neg128_k256");
}

#[test]
fn skew_vectors_follow_delays() {
    for f in unit::generate("skew", 9, 100, &Meta::test()).unwrap() {
        let v = parse(&f);
        let delays: Vec<usize> = v["delays"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap() as usize)
            .collect();
        let ind = v["in_data"].as_array().unwrap();
        let outd = v["out_data"].as_array().unwrap();
        for (j, &d) in delays.iter().enumerate() {
            let i: Vec<u64> = ind[j]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_u64().unwrap())
                .collect();
            let o: Vec<u64> = outd[j]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_u64().unwrap())
                .collect();
            for t in 0..o.len() {
                assert_eq!(o[t], if t >= d { i[t - d] } else { 0 });
            }
        }
        if f.path.contains("w_strip") {
            assert_eq!(&delays[8..], &[4, 5, 6, 7, 8, 9, 10, 11]);
        }
    }
}
