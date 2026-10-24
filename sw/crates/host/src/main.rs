//! `splitsys-host`: CLI for the SplitSys board (hardware, co-sim, or mock).

use clap::{Parser, Subcommand};
use splitsys_host::client::Board;
use splitsys_host::mock::MockConfig;
use splitsys_host::record::{
    iso_utc, now_unix, repo_root, runs_dir, session_path, validate_run_json, write_json, Session,
};
use splitsys_host::runner::{self, Context, GemmSpec, LoadCache};
use splitsys_host::server::{self, ServeOpts};
use splitsys_host::transport::{self, TransportSpec};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(name = "splitsys-host", version, about = "SplitSys host: drive the board, verify GEMMs bit-exactly, serve the dashboard")]
struct Cli {
    /// mock | serial:COM5 | serial:/dev/ttyUSB1 | serial:auto | tcp:127.0.0.1:PORT
    #[arg(long, global = true, default_value = "serial:auto")]
    transport: TransportSpec,
    /// UART baud rate (serial only).
    #[arg(long, global = true, default_value_t = splitsys_spec::BAUD as u32)]
    baud: u32,
    /// Multiply per-frame timeouts (default: 1 serial, 200 tcp co-sim).
    #[arg(long, global = true)]
    timeout_scale: Option<u32>,
    /// Results directory (default: <repo>/results).
    #[arg(long, global = true)]
    results: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print build_id and protocol version.
    Ping,
    /// Run the on-board self-test (one FULL and one STRIP GEMM from ROM).
    Selftest,
    /// Run one GEMM: a gemm.json file, or --name <benchmark> --m <M> --mode full|strip|both.
    Run {
        /// JSON file: {"name","M","K","N","mode","requant"?,"seed"?}
        gemm: Option<PathBuf>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        m: Option<usize>,
        #[arg(long, default_value = "full")]
        mode: String,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// int32 bypass output (bring-up).
        #[arg(long)]
        int32: bool,
        /// Skip writing the OBUF poison pattern before the run (less UART traffic).
        #[arg(long)]
        no_poison: bool,
    },
    /// Run a suite in both modes: bench (4 GEMMs × M∈{1,64}) | sweep (× M∈{1..64}) | smoke.
    Bench {
        #[arg(default_value = "bench")]
        suite: String,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        #[arg(long)]
        no_poison: bool,
    },
    /// Serve the dashboard on 127.0.0.1.
    Serve {
        #[arg(long, default_value_t = 8080)]
        port: u16,
        /// Directory of recorded hw run JSONs to offer as REPLAY (e.g. results/hw).
        #[arg(long)]
        replay: Option<PathBuf>,
        /// Serve demo/ assets from disk instead of the embedded copies (development).
        #[arg(long)]
        assets: Option<PathBuf>,
        /// Start with the attract loop on.
        #[arg(long)]
        attract: bool,
    },
    /// Raw byte echo test for the heartbeat_uart_echo bring-up bitstream.
    Echo {
        #[arg(long, default_value_t = 1000)]
        bytes: usize,
    },
    /// Random WRITE_MEM/READ_MEM round trips over all regions (checklist H-3).
    Memtest {
        #[arg(long, default_value_t = 1000)]
        frames: u64,
        /// RNG seed (default: from the clock; always printed).
        #[arg(long)]
        seed: Option<u64>,
    },
    /// Validate run-record JSON files against the DEMO.md §4 schema.
    CheckJson { files: Vec<PathBuf> },
    /// Serve the mock board over TCP, like the co-sim bridge (for testing tcp:).
    MockBridge {
        #[arg(long, default_value_t = 0)]
        port: u16,
    },
}

fn open_board(cli: &Cli) -> Result<Board, String> {
    transport::open(&cli.transport, cli.baud, cli.timeout_scale, MockConfig::default()).map(Board::new)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match real_main(&cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("splitsys-host: error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn real_main(cli: &Cli) -> Result<ExitCode, String> {
    let root = repo_root();
    let results = cli.results.clone().unwrap_or_else(|| root.join("results"));
    match &cli.cmd {
        Cmd::Ping => {
            let mut b = open_board(cli)?;
            let p = b.ping().map_err(|e| format!("{e} via {}", b.describe()))?;
            println!(
                "PING ok: build_id {:08x}  proto v{}  via {}  source {}",
                p.build_id,
                p.version,
                b.describe(),
                b.source().as_str()
            );
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Selftest => {
            let mut b = open_board(cli)?;
            let s = b.selftest().map_err(|e| e.to_string())?;
            let pf = |p: bool| if p { "PASS" } else { "FAIL" };
            println!("SELFTEST FULL : {} (fletcher32 {:08x})", pf(s.pass_full), s.fletcher_full);
            println!("SELFTEST STRIP: {} (fletcher32 {:08x})", pf(s.pass_strip), s.fletcher_strip);
            println!("source {} via {}", b.source().as_str(), b.describe());
            Ok(if s.pass_full && s.pass_strip { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        Cmd::Run { gemm, name, m, mode, seed, int32, no_poison } => {
            let mut specs = Vec::new();
            if let Some(p) = gemm {
                let b = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
                let g: GemmSpec = serde_json::from_slice(&b).map_err(|e| format!("{}: {e}", p.display()))?;
                specs.push(g);
            } else {
                let name = name.as_deref().ok_or("run: give a gemm.json or --name <benchmark>")?;
                let m = m.ok_or("run: --m is required with --name")?;
                let modes: Vec<&str> = if mode.eq_ignore_ascii_case("both") { vec!["FULL", "STRIP"] } else { vec![mode.as_str()] };
                for md in modes {
                    let mut g = GemmSpec::named(name, m, md, *seed)?;
                    g.requant = !int32;
                    specs.push(g);
                }
            }
            let ok = run_specs(cli, &results, &root, &specs, !no_poison, None)?;
            Ok(if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        Cmd::Bench { suite, seed, no_poison } => {
            let specs = runner::suite(suite, *seed)?;
            let ok = run_specs(cli, &results, &root, &specs, !no_poison, Some(suite))?;
            Ok(if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        Cmd::Serve { port, replay, assets, attract } => {
            server::serve(ServeOpts {
                port: *port,
                transport: cli.transport.clone(),
                baud: cli.baud,
                timeout_scale: cli.timeout_scale,
                results,
                root,
                assets: assets.clone(),
                replay: replay.clone(),
                attract: *attract,
            })?;
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Echo { bytes } => {
            let mut b = open_board(cli)?;
            if b.source() == transport::Source::Model {
                return Err("echo needs a raw byte line (serial: with the heartbeat_uart_echo bitstream, or tcp:); the mock board speaks only the framed protocol".into());
            }
            let (sent, bad, ms) = echo(b.transport(), *bytes)?;
            println!("ECHO {sent} bytes, {bad} mismatched, {ms:.1} ms");
            Ok(if bad == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        Cmd::Memtest { frames, seed } => {
            let frames = *frames;
            let seed = seed.unwrap_or_else(|| now_unix() as u64);
            let mut b = open_board(cli)?;
            println!(
                "memtest: {frames} frames, seed {seed}, via {} (source {})",
                b.describe(),
                b.source().as_str()
            );
            let step = (frames / 10).max(1);
            let r = splitsys_host::memtest::memtest(&mut b, frames, seed, |i, r| {
                if (i + 1) % step == 0 || i + 1 == frames {
                    println!(
                        "  {:>7}/{frames} frames  {:>9} bytes  bad frames {}  errors {}",
                        i + 1,
                        r.bytes,
                        r.bad_frames,
                        r.errors
                    );
                }
            });
            let l = r.link;
            println!(
                "MEMTEST {}: seed {} frames {} bytes {} bad_frames {} bad_bytes {} errors {} | NACK_CRC {} bad_rsp_crc {} timeouts {} busy {} retries {}",
                if r.ok() { "PASS" } else { "FAIL" },
                r.seed, r.frames, r.bytes, r.bad_frames, r.bad_bytes, r.errors,
                l.nack_crc, l.bad_rsp_crc, l.timeouts, l.busy, l.retries
            );
            if let Some(e) = &r.first_error {
                println!("first error: {e}");
            }
            Ok(if r.ok() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        Cmd::CheckJson { files } => {
            let mut ok = true;
            for f in files {
                let r = std::fs::read(f)
                    .map_err(|e| e.to_string())
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).map_err(|e| e.to_string()))
                    .and_then(|v| validate_run_json(&v));
                match r {
                    Ok(()) => println!("OK   {}", f.display()),
                    Err(e) => {
                        ok = false;
                        println!("FAIL {}: {e}", f.display())
                    }
                }
            }
            Ok(if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        Cmd::MockBridge { port } => {
            mock_bridge(*port)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Byte echo: send `n` pseudo-random bytes in 64-byte bursts, compare what returns.
pub fn echo(t: &mut dyn transport::Transport, n: usize) -> Result<(usize, usize, f64), String> {
    let mut rng = splitsys_golden::rng::Rng::new(0xEC40);
    let data: Vec<u8> = (0..n).map(|_| rng.next_u64() as u8).collect();
    let t0 = Instant::now();
    let mut got = Vec::with_capacity(n);
    for chunk in data.chunks(64) {
        t.send(chunk).map_err(|e| e.to_string())?;
        let want = got.len() + chunk.len();
        let deadline = Instant::now() + Duration::from_secs(2) * t.timeout_scale();
        let mut buf = [0u8; 256];
        while got.len() < want && Instant::now() < deadline {
            let k = t.recv(&mut buf, Duration::from_millis(100)).map_err(|e| e.to_string())?;
            got.extend_from_slice(&buf[..k]);
        }
        if got.len() < want {
            return Err(format!("echo: timeout after {} of {n} bytes returned", got.len()));
        }
    }
    let bad = data.iter().zip(&got).filter(|(a, b)| a != b).count() + got.len().abs_diff(n);
    Ok((n, bad, t0.elapsed().as_secs_f64() * 1e3))
}

fn mock_bridge(port: u16) -> Result<(), String> {
    use std::io::{Read, Write};
    let l = std::net::TcpListener::bind(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    println!("mock bridge (MODEL data, not RTL) listening on tcp:{}", l.local_addr().map_err(|e| e.to_string())?);
    for s in l.incoming().flatten() {
        std::thread::spawn(move || {
            let mut s = s;
            let mut board = splitsys_host::mock::MockBoard::default();
            let mut buf = [0u8; 4096];
            while let Ok(n) = s.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let out = board.rx(&buf[..n]);
                if s.write_all(&out).is_err() {
                    break;
                }
            }
        });
    }
    Ok(())
}

fn run_specs(
    cli: &Cli,
    results: &std::path::Path,
    root: &std::path::Path,
    specs: &[GemmSpec],
    poison: bool,
    suite: Option<&str>,
) -> Result<bool, String> {
    let mut b = open_board(cli)?;
    let ping = b.ping().map_err(|e| format!("{e} via {}", b.describe()))?;
    let src = runner::effective_source(b.source(), Some(&format!("{:08x}", ping.build_id)));
    let mut ctx = Context::new(results.to_path_buf(), root);
    ctx.poison = poison;
    ctx.build_id = Some(format!("{:08x}", ping.build_id));
    let mut cache = LoadCache::default();
    let mut session = Session::load_or_new(&session_path(results, src), src);
    let mut all_ok = true;
    let mut rows = Vec::new();
    let t0 = Instant::now();
    if src == "model" {
        println!("NOTE: mock board — every number below is MODEL data (source \"model\", mock: true), not hardware; mock runs are never counted as verified.");
    }
    println!(
        "{:<10} {:>3} {:>4} {:>4} {:<5} {:>9} {:>8} {:>8} {:>9}  {:<6} file",
        "gemm", "M", "K", "N", "mode", "cycles", "util%", "model%", "err%", "check"
    );
    for g in specs {
        match runner::run_gemm(&mut b, &mut ctx, &mut cache, g) {
            Ok((r, path)) => {
                session.add(&r);
                let ok = r.verified();
                all_ok &= ok;
                println!(
                    "{:<10} {:>3} {:>4} {:>4} {:<5} {:>9} {:>8.2} {:>8.2} {:>+9.3}  {:<6} {}",
                    r.gemm.name,
                    r.gemm.m,
                    r.gemm.k,
                    r.gemm.n,
                    r.gemm.mode,
                    r.counters.cycles_total,
                    r.derived.util * 100.0,
                    r.derived.util_model * 100.0,
                    r.derived.model_err_total * 100.0,
                    if ok { "OK" } else { "FAIL" },
                    path.map(|p| p.display().to_string()).unwrap_or_default()
                );
                if !ok {
                    println!("   check: {:?}", r.check);
                }
                rows.push(serde_json::json!({
                    "run_id": r.run_id, "name": r.gemm.name, "M": r.gemm.m, "mode": r.gemm.mode,
                    "util": r.derived.util, "util_model": r.derived.util_model,
                    "model_err_total": r.derived.model_err_total, "verified": ok,
                }));
            }
            Err(e) => {
                all_ok = false;
                println!("{:<10} {:>3} {:>4} {:>4} {:<5} ERROR: {e}", g.name, g.m, g.k, g.n, g.mode);
            }
        }
    }
    write_json(&session_path(results, src), &session).map_err(|e| e.to_string())?;
    if let Some(suite) = suite {
        let now = now_unix();
        let p = runs_dir(results, src).join(format!("bench_{}_{}.json", suite, iso_utc(now).replace(':', "-")));
        let summary = serde_json::json!({
            "kind": "bench_summary", "suite": suite, "source": src, "transport": b.describe(),
            "utc": iso_utc(now), "runs": rows, "all_verified": all_ok,
            "link": b.stats, "wall_s_end_to_end": t0.elapsed().as_secs_f64(),
        });
        write_json(&p, &summary).map_err(|e| e.to_string())?;
        println!("summary: {}", p.display());
    }
    println!(
        "{} run(s), {} passed checks{}; session {}: {} GEMMs verified, {} MACs, {} mismatches; link retries {}",
        specs.len(),
        if all_ok { "all" } else { "NOT all" },
        if src == "model" { " (mock board: circular, not counted as verified)" } else { "" },
        session_path(results, src).display(),
        session.gemms_verified,
        session.macs_verified,
        session.mismatches,
        b.stats.retries
    );
    Ok(all_ok)
}
