//! `splitsys-vec`: writes SplitSys test vectors under `sw/crates/vec/out/` (or `--out`).
//!
//! ```text
//! splitsys-vec unit <pe|requant|skew|packet> [--seed S] [--n N]
//! splitsys-vec gemm [--seed S] [--n N] [--mode full|strip] [--max-m M] [--max-k K] [--max-n N]
//! splitsys-vec directed
//! splitsys-vec selftest
//! splitsys-vec bench
//! splitsys-vec all          # every canonical set with default seeds
//! common: [--out DIR]
//! ```

use splitsys_golden::consts::{K_MAX, M_MAX, N_MAX};
use splitsys_vec::{bench, jobs, selftest, unit, Meta, OutFile};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "usage: splitsys-vec <unit <module>|gemm|directed|selftest|bench|all> \
[--seed S] [--n N] [--mode full|strip] [--max-m M] [--max-k K] [--max-n N] [--out DIR]";

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn git(args: &[&str], cwd: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// HEAD hash, and whether the generator's inputs (golden, vec sources, spec) are dirty.
fn git_meta() -> (String, bool) {
    let dir = manifest_dir();
    let Some(root) = git(&["rev-parse", "--show-toplevel"], &dir) else {
        return ("unknown".into(), false);
    };
    let root = PathBuf::from(root);
    let hash = git(&["rev-parse", "--short=12", "HEAD"], &root).unwrap_or_else(|| "unknown".into());
    let dirty = git(
        &[
            "status",
            "--porcelain",
            "--",
            "sw/crates/golden",
            "sw/crates/vec/src",
            "sw/crates/vec/Cargo.toml",
            "sw/crates/spec",
            "spec",
        ],
        &root,
    )
    .map(|s| !s.is_empty())
    .unwrap_or(false);
    (hash, dirty)
}

struct Args {
    pos: Vec<String>,
    opts: HashMap<String, String>,
}

fn parse_args(raw: &[String]) -> Result<Args, String> {
    let mut pos = Vec::new();
    let mut opts = HashMap::new();
    let mut it = raw.iter();
    while let Some(a) = it.next() {
        if let Some(key) = a.strip_prefix("--") {
            let v = it.next().ok_or_else(|| format!("--{key} needs a value"))?;
            opts.insert(key.to_string(), v.clone());
        } else {
            pos.push(a.clone());
        }
    }
    Ok(Args { pos, opts })
}

impl Args {
    fn num(&self, key: &str, default: u64) -> Result<u64, String> {
        match self.opts.get(key) {
            None => Ok(default),
            Some(v) => {
                let v = v.trim();
                let parsed = if let Some(h) = v.strip_prefix("0x") {
                    u64::from_str_radix(h, 16)
                } else {
                    v.parse()
                };
                parsed.map_err(|_| format!("--{key}: bad number '{v}'"))
            }
        }
    }
    fn check_known(&self, known: &[&str]) -> Result<(), String> {
        for k in self.opts.keys() {
            if !known.contains(&k.as_str()) && k != "out" {
                return Err(format!("unknown option --{k}"));
            }
        }
        Ok(())
    }
}

fn write_all(out_dir: &Path, files: &[OutFile]) -> Result<(), String> {
    for f in files {
        let p = out_dir.join(&f.path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        std::fs::write(&p, &f.bytes).map_err(|e| format!("write {}: {e}", p.display()))?;
        println!("wrote {} ({} bytes)", p.display(), f.bytes.len());
    }
    Ok(())
}

fn run(raw: &[String]) -> Result<(), String> {
    let args = parse_args(raw)?;
    let cmd = args.pos.first().ok_or(USAGE)?.as_str();
    let (git_hash, git_dirty) = git_meta();
    let meta = Meta {
        git_hash,
        git_dirty,
        command: raw.join(" "),
    };
    let out_dir = args
        .opts
        .get("out")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir().join("out"));
    let files: Vec<OutFile> = match cmd {
        "unit" => {
            args.check_known(&["seed", "n"])?;
            let module = args
                .pos
                .get(1)
                .ok_or("unit needs a module: pe|requant|skew|packet")?;
            let seed = args.num("seed", 1)?;
            let n = args.num("n", unit::default_n(module) as u64)? as usize;
            unit::generate(module, seed, n, &meta)?
        }
        "gemm" => {
            args.check_known(&["seed", "n", "mode", "max-m", "max-k", "max-n"])?;
            let mode = jobs::parse_mode(args.opts.get("mode").map_or("full", String::as_str))?;
            jobs::gemm(
                args.num("seed", 1)?,
                args.num("n", 100)? as usize,
                mode,
                args.num("max-m", M_MAX as u64)? as usize,
                args.num("max-k", K_MAX as u64)? as usize,
                args.num("max-n", N_MAX as u64)? as usize,
                &meta,
            )?
        }
        "directed" => {
            args.check_known(&[])?;
            jobs::directed(&meta)
        }
        "selftest" => {
            args.check_known(&[])?;
            selftest::selftest(&meta)
        }
        "bench" => {
            args.check_known(&[])?;
            bench::bench(&meta)
        }
        "all" => {
            args.check_known(&[])?;
            let mut v = selftest::selftest(&meta);
            v.extend(jobs::directed(&meta));
            v.extend(bench::bench(&meta));
            for m in unit::MODULES {
                v.extend(unit::generate(m, 1, unit::default_n(m), &meta)?);
            }
            for mode in ["full", "strip"] {
                v.extend(jobs::gemm(
                    1,
                    100,
                    jobs::parse_mode(mode)?,
                    M_MAX,
                    K_MAX,
                    N_MAX,
                    &meta,
                )?);
            }
            v
        }
        _ => return Err(USAGE.into()),
    };
    write_all(&out_dir, &files)
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    match run(&raw) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("splitsys-vec: {e}");
            ExitCode::FAILURE
        }
    }
}
