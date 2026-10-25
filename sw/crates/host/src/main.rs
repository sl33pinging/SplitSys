//! `splitsys-host`: CLI for the SplitSys board (hardware, co-sim, or mock).

use clap::{Parser, Subcommand};
use splitsys_host::client::{self, Board, PingInfo};
use splitsys_host::mock::MockConfig;
use splitsys_host::record::{
    iso_utc, now_unix, repo_root, runs_dir, session_path, validate_run_json, write_json, Session,
};
use splitsys_host::runner::{self, Context, GemmSpec, LoadCache};
use splitsys_host::server::{self, ServeOpts};
use splitsys_host::transport::{self, baud_label, BaudSel, TransportSpec};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

/// `--version`: crate version plus the embedded host git hash (the dashboard
/// assets are compiled in, so this identifies the served dashboard too).
const HOST_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (git ", env!("SPLITSYS_HOST_GIT_HASH"), ")");

#[derive(Parser)]
#[command(name = "splitsys-host", version = HOST_VERSION, about = "SplitSys host: drive the board, verify GEMMs bit-exactly, serve the dashboard")]
struct Cli {
    /// mock | serial:COM5 | serial:/dev/ttyUSB1 | serial:auto | tcp:127.0.0.1:PORT
    #[arg(long, global = true, default_value = "serial:auto")]
    transport: TransportSpec,
    /// UART baud rate (serial only): a rate, or auto = PING at the spec rate
    /// (1562500, KG3+) and, if nothing answers, at 115200 (KG2). Only PING is sent
    /// at a rate that has not answered.
    #[arg(long, global = true, default_value = "auto")]
    baud: BaudSel,
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
    Selftest {
        /// Afterwards read back each job's checked OBUF range and diff it against golden.
        #[arg(long)]
        dump: bool,
        /// Differing bytes printed per job with --dump.
        #[arg(long, default_value_t = 16)]
        max_diffs: usize,
    },
    /// Read a buffer region via READ_MEM (chunked) and print or save it.
    Dump {
        /// wbuf | abuf | obuf | rqparam (or 0..3)
        #[arg(long)]
        region: String,
        /// Byte address in the region.
        #[arg(long, default_value_t = 0)]
        addr: usize,
        /// Bytes to read (default: to the end of the region).
        #[arg(long)]
        len: Option<usize>,
        /// Write to a file: .bin raw bytes, anything else the hex dump text.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Run one GEMM: a gemm.json file, or --name <benchmark> --m <M> --mode full|strip|both|auto.
    /// auto: the cycle model (splitsys_cycle::choose_mode) picks the mode per GEMM.
    Run {
        /// JSON file: {"name","M","K","N","mode","requant"?,"seed"?}
        gemm: Option<PathBuf>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        m: Option<usize>,
        /// full | strip | both | auto (model-scored pick per GEMM, ties -> FULL)
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
        /// Random batch (H-5): N jobs per mode from the `splitsys-vec gemm` generator
        /// (with --mode auto: N jobs, each run once in the mode the model picks).
        #[arg(long)]
        random: Option<usize>,
        #[arg(long, default_value_t = splitsys_spec::M_MAX)]
        max_m: usize,
        #[arg(long, default_value_t = splitsys_spec::K_MAX)]
        max_k: usize,
        #[arg(long, default_value_t = splitsys_spec::N_MAX)]
        max_n: usize,
    },
    /// Run a suite: bench (4 GEMMs × M∈{1,64}) | sweep (× M∈{1..64}) | smoke.
    Bench {
        #[arg(default_value = "bench")]
        suite: String,
        /// both (FULL and STRIP per shape) | full | strip | auto (model-scored pick per shape)
        #[arg(long, default_value = "both")]
        mode: String,
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
    /// Session totals (results/session.json etc.), derived from the run records.
    Session {
        /// Recompute and write session.json from results/<source>/ (top level only).
        #[arg(long)]
        rebuild: bool,
        /// hw | sim | model
        #[arg(long, default_value = "hw")]
        source: String,
    },
    /// Validate run-record JSON files against the DEMO.md §4 schema.
    CheckJson { files: Vec<PathBuf> },
    /// Serve the mock board over TCP, like the co-sim bridge (for testing tcp:).
    MockBridge {
        #[arg(long, default_value_t = 0)]
        port: u16,
    },
    /// G5: generate text with the tiny LM stories260K, every linear layer on the board
    /// (bit-exact checked, counters vs model); records in results/<source>/lm/.
    Lm {
        #[arg(long, default_value = "Once upon a time")]
        prompt: String,
        /// New tokens to generate (after the prompt).
        #[arg(long, default_value_t = 8)]
        tokens: usize,
        /// Directory with stories260K.bin and tok512.bin (default: <repo>/models/stories260K).
        #[arg(long)]
        model_dir: Option<PathBuf>,
        /// full | strip | auto (cycle-model pick per GEMM)
        #[arg(long, default_value = "auto")]
        mode: String,
        /// Skip the OBUF poison writes (less UART traffic; weaker stale-data check).
        #[arg(long)]
        no_poison: bool,
    },
}

/// Open the board and PING it (over serial with `--baud auto`, probing the known
/// rates). Every command but `echo` starts here, so nothing else is ever sent at a
/// rate that has not answered PING.
fn connect_board(cli: &Cli) -> Result<(Board, PingInfo), String> {
    let c = client::connect(&cli.transport, cli.baud, None, cli.timeout_scale, MockConfig::default())?;
    if let (Some(b), false) = (c.baud, c.no_answer.is_empty()) {
        let tried: Vec<String> = c.no_answer.iter().map(|(r, _)| r.to_string()).collect();
        eprintln!(
            "note: no answer at {} baud; the board answered at {b} baud ({}), build_id {:08x}. Pass --baud {b} to skip the probe.",
            tried.join(", "),
            if b == transport::KG2_BAUD { "the KG2 rate" } else { "the spec rate" },
            c.ping.build_id
        );
    }
    Ok((c.board, c.ping))
}

/// Open without PING (only `echo`: the echo bitstream does not speak the protocol).
fn open_raw(cli: &Cli) -> Result<Board, String> {
    transport::open(&cli.transport, cli.baud.primary(), cli.timeout_scale, MockConfig::default()).map(Board::new)
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
            let (b, p) = connect_board(cli)?;
            // `baud N` is parsed by demo/start_demo.ps1 to pin the server's rate.
            let baud = transport::baud_of(&b.describe())
                .map(|r| if r >= 1_000_000 { format!("  baud {r} ({})", baud_label(r)) } else { format!("  baud {r}") })
                .unwrap_or_default();
            println!(
                "PING ok: build_id {:08x}  proto v{}{baud}  via {}  source {}",
                p.build_id,
                p.version,
                b.describe(),
                b.effective_source().as_str()
            );
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Selftest { dump, max_diffs } => {
            let (mut b, _) = connect_board(cli)?;
            let s = b.selftest().map_err(|e| e.to_string())?;
            let pf = |p: bool| if p { "PASS" } else { "FAIL" };
            println!("SELFTEST FULL : {} (fletcher32 {:08x})", pf(s.pass_full), s.fletcher_full);
            println!("SELFTEST STRIP: {} (fletcher32 {:08x})", pf(s.pass_strip), s.fletcher_strip);
            println!("source {} via {}", b.effective_source().as_str(), b.describe());
            let mut ok = s.pass_full && s.pass_strip;
            if *dump {
                if splitsys_host::diag::selftest_ranges_overlap() {
                    println!("WARNING: self-test OBUF ranges overlap; the earlier job's read-back is not meaningful");
                }
                for j in splitsys_host::diag::selftest_diff(&mut b, *max_diffs).map_err(|e| e.to_string())? {
                    println!(
                        "{} job: OBUF bytes [{:#06x}, {:#06x}) {} bytes: {} differ; fletcher32 readback {:08x} ROM {:08x}",
                        if j.mode == splitsys_spec::MODE_STRIP { "STRIP" } else { "FULL " },
                        j.out_byte_addr,
                        j.out_byte_addr + j.len,
                        j.len,
                        j.mismatches,
                        j.readback_fletcher,
                        j.stored_fletcher
                    );
                    for d in &j.diffs {
                        println!("   {}: got {:#04x} want {:#04x}", d.loc, d.got, d.want);
                    }
                    if j.mismatches > j.diffs.len() {
                        println!("   ... {} more", j.mismatches - j.diffs.len());
                    }
                    ok &= j.mismatches == 0;
                }
            }
            Ok(if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        Cmd::Dump { region, addr, len, out } => {
            let r = splitsys_host::diag::parse_region(region)?;
            let size = splitsys_host::diag::region_bytes(r);
            let len = len.unwrap_or(size.saturating_sub(*addr));
            if *addr + len > size || len == 0 {
                return Err(format!("dump: [{addr}, {}) outside region {region} (size {size})", addr + len));
            }
            let (mut b, _) = connect_board(cli)?;
            let bytes = b.read_mem(r, *addr, len).map_err(|e| e.to_string())?;
            let text = splitsys_host::diag::hexdump(r, *addr, &bytes);
            match out {
                Some(p) if p.extension().is_some_and(|e| e == "bin") => {
                    std::fs::write(p, &bytes).map_err(|e| format!("{}: {e}", p.display()))?;
                    println!("wrote {len} bytes to {}", p.display());
                }
                Some(p) => {
                    std::fs::write(p, &text).map_err(|e| format!("{}: {e}", p.display()))?;
                    println!("wrote hex dump of {len} bytes to {}", p.display());
                }
                None => print!("{text}"),
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Run { gemm, name, m, mode, seed, int32, no_poison, random, max_m, max_k, max_n } => {
            if let Some(n) = random {
                return random_batch(cli, &results, &root, *n, mode, *seed, (*max_m, *max_k, *max_n), !no_poison);
            }
            let mut specs = Vec::new();
            if let Some(p) = gemm {
                let b = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
                let g: GemmSpec = serde_json::from_slice(&b).map_err(|e| format!("{}: {e}", p.display()))?;
                specs.push(g);
            } else {
                let name = name.as_deref().ok_or("run: give a gemm.json or --name <benchmark>")?;
                let m = m.ok_or("run: --m is required with --name")?;
                let modes: Vec<&str> = if mode.eq_ignore_ascii_case("both") { vec!["FULL", "STRIP"] } else { vec![mode.as_str()] };
                if !modes.iter().all(|md| runner::is_auto(md) || runner::mode_code(md).is_ok()) {
                    return Err(format!("--mode {mode}: expected full | strip | both | auto"));
                }
                for md in modes {
                    let mut g = GemmSpec::named(name, m, md, *seed)?;
                    g.requant = !int32;
                    specs.push(g);
                }
            }
            let ok = run_specs(cli, &results, &root, &specs, !no_poison, None)?;
            Ok(if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        Cmd::Bench { suite, mode, seed, no_poison } => {
            let specs = runner::suite_modes(suite, *seed, mode)?;
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
            let mut b = open_raw(cli)?;
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
            let (mut b, _) = connect_board(cli)?; // PING: learn the build id (mock-over-tcp is labelled model)
            println!(
                "memtest: {frames} frames, seed {seed}, via {} (source {})",
                b.describe(),
                b.effective_source().as_str()
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
            println!(
                "regions (frames): WBUF {} ABUF {} OBUF {} RQPARAM {}",
                r.per_region[0], r.per_region[1], r.per_region[2], r.per_region[3]
            );
            if let Some(e) = &r.first_error {
                println!("first error: {e}");
            }
            Ok(if r.ok() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        Cmd::Session { rebuild, source } => {
            if !["hw", "sim", "model"].contains(&source.as_str()) {
                return Err(format!("--source {source}: expected hw | sim | model"));
            }
            let s = if *rebuild {
                Session::rebuild_and_write(&results, source).map_err(|e| e.to_string())?
            } else {
                Session::rebuild(&results, source)
            };
            println!(
                "SESSION {source}: {} GEMMs run, {} verified, {} MACs verified, {} mismatches, {} failed; {} .. {}; builds {}; {}{}",
                s.gemms_run,
                s.gemms_verified,
                s.macs_verified,
                s.mismatches,
                s.failed_runs,
                s.started_utc,
                s.updated_utc,
                s.build_ids.join(","),
                s.derived_from.clone().unwrap_or_default(),
                if *rebuild { format!("; wrote {}", session_path(&results, source).display()) } else { String::new() }
            );
            Ok(ExitCode::SUCCESS)
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
        Cmd::Lm { prompt, tokens, model_dir, mode, no_poison } => {
            let dir = model_dir.clone().unwrap_or_else(|| root.join(splitsys_host::lm::DEFAULT_MODEL_DIR));
            let o = splitsys_host::lm::GenOpts {
                prompt: prompt.clone(),
                max_new_tokens: *tokens,
                mode: splitsys_host::lm::LmMode::parse(mode)?,
                poison: !no_poison,
                stop: None,
            };
            let (mut b, _) = connect_board(cli)?;
            let mut ctx = Context::new(results.clone(), &root);
            let ok = splitsys_host::lm::cli_lm(&mut b, &mut ctx, &dir, &o)?;
            Ok(if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE })
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
    let (mut b, ping) = connect_board(cli)?;
    let src = runner::effective_source(b.source(), Some(&format!("{:08x}", ping.build_id)));
    let mut ctx = Context::new(results.to_path_buf(), root);
    ctx.poison = poison;
    ctx.build_id = Some(format!("{:08x}", ping.build_id));
    let mut cache = LoadCache::default();
    let mut all_ok = true;
    let mut rows = Vec::new();
    let mut tally = splitsys_host::batch::AutoTally::default();
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
                for d in &r.check.counter_diffs {
                    println!("   counter {d}");
                }
                if let Some(sel) = &r.mode_select {
                    println!("{}", auto_line(&r, sel, src));
                    tally.add(&r);
                }
                rows.push(serde_json::json!({
                    "run_id": r.run_id, "name": r.gemm.name, "M": r.gemm.m, "mode": r.gemm.mode,
                    "util": r.derived.util, "util_model": r.derived.util_model,
                    "model_err_total": r.derived.model_err_total, "verified": ok,
                    "mode_select": r.mode_select,
                }));
            }
            Err(e) => {
                all_ok = false;
                println!("{:<10} {:>3} {:>4} {:>4} {:<5} ERROR: {e}", g.name, g.m, g.k, g.n, g.mode);
            }
        }
    }
    if tally.runs > 0 {
        for l in tally.lines(src) {
            println!("{l}");
        }
    }
    // Totals are derived from the run records on disk (never accumulated).
    let session = Session::rebuild_and_write(results, src).map_err(|e| e.to_string())?;
    if let Some(suite) = suite {
        let now = now_unix();
        let p = runs_dir(results, src).join(format!("bench_{}_{}.json", suite, iso_utc(now).replace(':', "-")));
        let summary = serde_json::json!({
            "kind": "bench_summary", "suite": suite, "source": src, "transport": b.describe(),
            "utc": iso_utc(now), "runs": rows, "all_verified": all_ok,
            "link": b.stats, "wall_s_end_to_end": t0.elapsed().as_secs_f64(),
            "auto": (tally.runs > 0).then_some(&tally),
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

/// `run --random N`: the H-5 random-GEMM batch.
#[allow(clippy::too_many_arguments)]
fn random_batch(
    cli: &Cli,
    results: &std::path::Path,
    root: &std::path::Path,
    n: usize,
    mode: &str,
    seed: u64,
    max: (usize, usize, usize),
    poison: bool,
) -> Result<ExitCode, String> {
    let auto = runner::is_auto(mode);
    let modes: Vec<u8> = match mode.to_ascii_lowercase().as_str() {
        "both" => vec![splitsys_spec::MODE_FULL, splitsys_spec::MODE_STRIP],
        "auto" => vec![splitsys_spec::MODE_FULL], // operands from the FULL generator
        m => vec![runner::mode_code(m)?],
    };
    let (mut b, ping) = connect_board(cli)?;
    let id = format!("{:08x}", ping.build_id);
    let src = runner::effective_source(b.source(), Some(&id));
    let mut ctx = Context::new(results.to_path_buf(), root);
    ctx.poison = poison;
    ctx.build_id = Some(id);
    println!(
        "random batch: {n} jobs × {}, seed {seed}, max M/K/N {}/{}/{}, via {} (source {src})",
        if auto { "1 mode each, chosen by the cycle model (auto)".to_string() } else { format!("{} mode(s)", modes.len()) },
        max.0,
        max.1,
        max.2,
        b.describe()
    );
    for &m in &modes {
        let ms = if m == splitsys_spec::MODE_STRIP { "strip" } else { "full" };
        println!(
            "  {}: {}",
            if auto { "operands and bases from" } else { "same jobs for sim" },
            splitsys_host::batch::vec_command(seed, n, ms, max)
        );
    }
    let mut k = 0usize;
    let each = |bj: &splitsys_host::batch::BatchJob, res: &Result<(splitsys_host::record::RunRecord, Option<PathBuf>), runner::RunError>| {
        k += 1;
        match res {
            Ok((r, path)) => {
                let ok = r.verified() && r.check.counter_diffs.is_empty();
                println!(
                    "  {:>4} {:<18} M={:<3} K={:<3} N={:<3} {:<5} {} util {:>6.2}% cycles {:>7} {} {}",
                    k,
                    bj.name,
                    r.gemm.m,
                    r.gemm.k,
                    r.gemm.n,
                    r.gemm.mode,
                    if r.gemm.requant { "i8 " } else { "i32" },
                    r.derived.util * 100.0,
                    r.counters.cycles_total,
                    if ok { "OK" } else { "FAIL" },
                    path.as_ref().map(|p| p.display().to_string()).unwrap_or_default()
                );
                for d in &r.check.counter_diffs {
                    println!("         counter {d}");
                }
                if let Some(sel) = &r.mode_select {
                    println!("{}", auto_line(r, sel, src));
                }
            }
            Err(e) => println!("  {:>4} {:<18} ERROR: {e}", k, bj.name),
        }
    };
    let (s, tally) = if auto {
        let (s, t) = splitsys_host::batch::run_batch_auto(&mut b, &mut ctx, seed, n, max, each)?;
        (s, Some(t))
    } else {
        (splitsys_host::batch::run_batch(&mut b, &mut ctx, seed, n, &modes, max, each)?, None)
    };
    // Totals are derived from the run records on disk (never accumulated).
    let session = Session::rebuild_and_write(results, src).map_err(|e| e.to_string())?;
    println!(
        "session {}: {} GEMMs verified, {} mismatches (derived from {})",
        session_path(results, src).display(),
        session.gemms_verified,
        session.mismatches,
        session.derived_from.as_deref().unwrap_or("records")
    );
    println!("{}", s.line());
    if let Some(t) = &tally {
        for l in t.lines(src) {
            println!("{l}");
        }
    }
    if let Some(f) = &s.first_failure {
        println!("first failure: {f}");
    }
    Ok(if s.ok() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

/// One auto run, honestly labelled: the chosen mode's cycles are measured; the
/// other mode's are the model's prediction only (it was not run).
fn auto_line(r: &splitsys_host::record::RunRecord, sel: &splitsys_host::record::ModeSelect, src: &str) -> String {
    let other = if sel.chosen == "STRIP" { "FULL" } else { "STRIP" };
    let (tc, to) = if sel.chosen == "STRIP" {
        (sel.model_tiles_strip, sel.model_tiles_full)
    } else {
        (sel.model_tiles_full, sel.model_tiles_strip)
    };
    format!(
        "         auto -> {}: measured {} {} cycles (model {}, {tc} tiles) | {other} not run: model {} cycles ({to} tiles)",
        sel.chosen,
        src.to_ascii_uppercase(),
        r.counters.cycles_total,
        sel.model_cycles_chosen(),
        sel.model_cycles_other(),
    )
}
