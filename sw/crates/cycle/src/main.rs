//! `splitsys-cycle`: CLI for the SplitSys cycle model (all output `source: "model"`).
//!
//! ```text
//! splitsys-cycle predict <descriptor.json> [--timeline]
//! splitsys-cycle choose --m 1 --k 64 --n 256
//! splitsys-cycle sweep    [--out results/model/sweep.json]
//! splitsys-cycle roofline [--out results/model/roofline.json]
//! splitsys-cycle explain  [--out results/model/README.md]
//! splitsys-cycle all      (sweep + roofline + explain into results/model/)
//! ```
//! Every subcommand accepts `--latencies <file.json>` (keys `L_PE, L_RD,
//! L_SETUP, L_RQ, L_WB, L_ACCEPT, L_DONE`) to override the spec defaults,
//! plus `--calibrated` to mark the result as no longer provisional.

use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use splitsys_cycle::report::{self, GemmBlock, ModelBlock};
use splitsys_cycle::{
    choose_mode, predict, spec, timeline, Descriptor, Latencies, Mode, PhaseEvent,
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "splitsys-cycle",
    about = "SplitSys cycle model (SPEC §3.7, §4)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone)]
struct LatOpts {
    /// JSON file overriding the spec latencies (calibration).
    #[arg(long)]
    latencies: Option<PathBuf>,
    /// Mark the latencies as calibrated (latencies_provisional = false).
    #[arg(long)]
    calibrated: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Predict every counter for one descriptor (JSON: {mode, M, K, N, flags|requant} or a run record with a `gemm` block).
    Predict {
        descriptor: PathBuf,
        /// Also print the per-phase timeline (model-derived).
        #[arg(long)]
        timeline: bool,
        #[command(flatten)]
        lat: LatOpts,
    },
    /// Pick the mode with the lower cycles_total (ties → FULL).
    Choose {
        #[arg(long)]
        m: u64,
        #[arg(long)]
        k: u64,
        #[arg(long)]
        n: u64,
        #[command(flatten)]
        lat: LatOpts,
    },
    /// Write the utilization-vs-M sweep (default results/model/sweep.json; `-` = stdout).
    Sweep {
        #[arg(long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        lat: LatOpts,
    },
    /// Write roofline points and roofs (default results/model/roofline.json; `-` = stdout).
    Roofline {
        #[arg(long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        lat: LatOpts,
    },
    /// Write the plain-English explanation (default results/model/README.md; `-` = stdout).
    Explain {
        #[arg(long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        lat: LatOpts,
    },
    /// sweep + roofline + explain into results/model/.
    All {
        #[command(flatten)]
        lat: LatOpts,
    },
}

fn load_lat(o: &LatOpts) -> Result<(Latencies, bool), String> {
    let lat = match &o.latencies {
        None => Latencies::default(),
        Some(p) => {
            let s = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
            serde_json::from_str(&s).map_err(|e| format!("{}: {e}", p.display()))?
        }
    };
    Ok((
        lat,
        !o.calibrated && (o.latencies.is_some() || spec::LATENCIES_PROVISIONAL),
    ))
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ModeRepr {
    Code(u8),
    Name(String),
}

#[derive(Deserialize)]
struct DescInput {
    #[serde(default)]
    name: Option<String>,
    mode: ModeRepr,
    #[serde(default)]
    flags: Option<u8>,
    #[serde(default)]
    requant: Option<bool>,
    #[serde(rename = "M", alias = "m")]
    m: u64,
    #[serde(rename = "K", alias = "k")]
    k: u64,
    #[serde(rename = "N", alias = "n")]
    n: u64,
}

fn parse_descriptor(text: &str) -> Result<(String, Descriptor), String> {
    let mut v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if let Some(g) = v.get("gemm") {
        v = g.clone();
    }
    let d: DescInput = serde_json::from_value(v).map_err(|e| e.to_string())?;
    let mode = match d.mode {
        ModeRepr::Code(c) => Mode::from_code(c),
        ModeRepr::Name(s) => match s.to_ascii_uppercase().as_str() {
            "FULL" => Some(Mode::Full),
            "STRIP" => Some(Mode::Strip),
            _ => None,
        },
    }
    .ok_or("mode must be FULL/STRIP (INDEP is reserved and not modeled)")?;
    // flags wins (must be FLAGS_INT8 or FLAGS_INT32); otherwise requant (default true) → FLAGS_INT8, false → FLAGS_INT32.
    let flags = d.flags.unwrap_or(if d.requant.unwrap_or(true) {
        spec::FLAGS_INT8
    } else {
        spec::FLAGS_INT32
    });
    let desc = Descriptor::new(mode, flags, d.m, d.k, d.n);
    desc.validate().map_err(|e| e.to_string())?;
    Ok((d.name.unwrap_or_else(|| "custom".to_string()), desc))
}

#[derive(Serialize)]
struct Prediction {
    source: &'static str,
    latencies_provisional: bool,
    latencies: Latencies,
    gemm: GemmBlock,
    model: ModelBlock,
    derived: report::Derived,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeline: Option<Vec<PhaseEvent>>,
}

fn write_out(out: Option<PathBuf>, default: &str, text: &str) -> Result<(), String> {
    let path = out.unwrap_or_else(|| report::results_dir().join(default));
    if path == Path::new("-") {
        print!("{text}");
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    eprintln!("wrote {}", path.display());
    Ok(())
}

fn json<T: Serialize>(v: &T) -> String {
    let mut s = serde_json::to_string_pretty(v).expect("serialize");
    s.push('\n');
    s
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.cmd {
        Cmd::Predict {
            descriptor,
            timeline: want_tl,
            lat,
        } => {
            let (lat, prov) = load_lat(&lat)?;
            let text = std::fs::read_to_string(&descriptor)
                .map_err(|e| format!("{}: {e}", descriptor.display()))?;
            let (name, d) = parse_descriptor(&text)?;
            let hash = report::model_git_hash();
            let rec = report::model_record(&name, &d, &lat, &hash);
            predict(&d, &lat).map_err(|e| e.to_string())?;
            let p = Prediction {
                source: "model",
                latencies_provisional: prov,
                latencies: lat,
                gemm: rec.gemm,
                model: rec.model,
                derived: rec.derived,
                timeline: want_tl.then(|| timeline(&d, &lat)),
            };
            print!("{}", json(&p));
        }
        Cmd::Choose { m, k, n, lat } => {
            let (lat, _) = load_lat(&lat)?;
            Descriptor::new(Mode::Full, report::bench_flags(), m, k, n)
                .validate()
                .map_err(|e| e.to_string())?;
            println!("{}", choose_mode(m, k, n, report::bench_flags(), &lat));
        }
        Cmd::Sweep { out, lat } => {
            let (lat, prov) = load_lat(&lat)?;
            let s = report::sweep(&lat, prov, &report::model_git_hash());
            write_out(out, "sweep.json", &json(&s))?;
        }
        Cmd::Roofline { out, lat } => {
            let (lat, prov) = load_lat(&lat)?;
            let r = report::roofline(&lat, prov, &report::model_git_hash());
            write_out(out, "roofline.json", &json(&r))?;
        }
        Cmd::Explain { out, lat } => {
            let (lat, prov) = load_lat(&lat)?;
            let s = report::sweep(&lat, prov, &report::model_git_hash());
            write_out(out, "README.md", &report::explain(&s))?;
        }
        Cmd::All { lat } => {
            let (lat, prov) = load_lat(&lat)?;
            let hash = report::model_git_hash();
            let s = report::sweep(&lat, prov, &hash);
            write_out(None, "sweep.json", &json(&s))?;
            write_out(
                None,
                "roofline.json",
                &json(&report::roofline(&lat, prov, &hash)),
            )?;
            write_out(None, "README.md", &report::explain(&s))?;
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("splitsys-cycle: {e}");
            ExitCode::FAILURE
        }
    }
}
