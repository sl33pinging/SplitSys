//! Dashboard server: a tiny std-only HTTP/1.1 server on 127.0.0.1 with SSE.
//!
//! A single worker thread owns the board connection; HTTP handlers only queue
//! commands and read the shared snapshot, so the UI never blocks on the UART.
//!
//! Endpoints: `GET /`, `/app.js`, `/style.css`, `GET /api/state`, `GET /events`
//! (SSE: `run_started`, `run_finished`, `board_status`, `scene`, `state`),
//! `POST /api/run` (named presets only), `POST /api/scene`, `POST /api/demo`,
//! `POST /api/attract`, `POST /api/replay`.

use crate::client::Board;
use crate::mock::MockConfig;
use crate::record::{iso_utc, load_runs, now_unix, runs_dir, RunRecord, Session};
use crate::runner::{self, Context, GemmSpec, LoadCache};
use crate::transport::{self, TransportSpec};
use serde_json::{json, Value};
use splitsys_cycle as cycle;
use splitsys_spec as spec;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const INDEX_HTML: &str = include_str!("../../../../demo/index.html");
const APP_JS: &str = include_str!("../../../../demo/app.js");
const STYLE_CSS: &str = include_str!("../../../../demo/style.css");
const LM_JS: &str = include_str!("../../../../demo/lm.js");

/// Keep this many recent runs in the snapshot.
const MAX_RUNS: usize = 400;

pub struct ServeOpts {
    pub port: u16,
    pub transport: TransportSpec,
    /// Serial rate, or auto (probe the known rates at each (re)connect, last good first).
    pub baud: transport::BaudSel,
    pub timeout_scale: Option<u32>,
    pub results: PathBuf,
    pub root: PathBuf,
    /// Serve demo assets from this directory instead of the embedded copies (dev).
    pub assets: Option<PathBuf>,
    /// Directory of recorded run JSONs to offer as REPLAY.
    pub replay: Option<PathBuf>,
    /// Scene 4 REPLAY generation: a gen id in `<replay>/lm` or an LM record path
    /// (default: the newest record there).
    pub replay_lm: Option<String>,
    pub attract: bool,
}

#[derive(Debug)]
enum Cmd {
    Run(GemmSpec),
    /// Several specs shown one after another (like the demo, without the scene change).
    RunMany(Vec<GemmSpec>),
    Demo,
    Selftest,
    /// G5: a tiny-LM generation (Scene 4); every linear layer on the board.
    Lm(crate::lm::GenOpts),
}

struct Replay {
    dir: String,
    recorded_utc: String,
    runs: Vec<RunRecord>,
    session: Option<Value>,
}

struct App {
    source: String,
    transport: String,
    board_status: &'static str,
    board_error: Option<String>,
    build_id: Option<String>,
    proto_version: Option<u8>,
    last_seen_utc: Option<String>,
    link: Value,
    session: Session,
    runs: VecDeque<RunRecord>,
    /// Roofline points over every record of `source` (not capped like `runs`).
    roof: Vec<Value>,
    /// Records of `source` on disk; `runs` keeps the latest `MAX_RUNS`.
    runs_total: usize,
    /// (build_id, records) over every record of `source`, first-appearance order.
    builds: Vec<(String, usize)>,
    /// Auto-mode runs and same-shape measured pairs over every record of `source`.
    auto: AutoView,
    /// Scene 2/3 benchmark-shape points over every record of `source` (not capped).
    bench: BenchView,
    running: Option<Value>,
    queue_len: usize,
    scene: u8,
    attract: bool,
    demo_running: bool,
    highlight: Vec<String>,
    replay: Option<Replay>,
    replay_active: bool,
    last_selftest: Option<Value>,
    started_utc: String,
    /// Scene 4: a generation is running / the latest live record view.
    lm_busy: bool,
    lm_live: Option<Value>,
    /// Scene 4 REPLAY pick: a gen id in `<replay>/lm` or an LM record path; None = newest.
    replay_lm: Option<String>,
    lm_stop: Arc<std::sync::atomic::AtomicBool>,
}

/// The label a transport implies before PING: serial = hw, tcp = sim, mock = model.
fn transport_source(t: &TransportSpec) -> &'static str {
    match t {
        TransportSpec::Mock => "model",
        TransportSpec::Tcp(_) => "sim",
        TransportSpec::Serial(_) => "hw",
    }
}

/// The rule behind the Scene 3 "measured int8" roofline dots, stated on screen.
pub const ROOF_RULE: &str = "every run record of the source with gemm.requant and nonzero derived.arithmetic_intensity and derived.macs_per_cycle (uncapped)";

/// Does a record go on the roofline as a measured int8 point (see `ROOF_RULE`)?
fn roof_point(r: &RunRecord) -> Option<Value> {
    let ai = r.derived.arithmetic_intensity.filter(|v| *v != 0.0)?;
    let mpc = r.derived.macs_per_cycle.filter(|v| *v != 0.0)?;
    r.gemm.requant.then(|| {
        json!({"ai": ai, "mpc": mpc, "mode": r.gemm.mode, "build_id": r.build_id, "run_id": r.run_id})
    })
}

/// Per-build record counts in first-appearance order (records are sorted by run_id),
/// for the "HW: builds A (n) + B (m)" attribution on aggregating panels.
fn build_counts<'a>(runs: impl IntoIterator<Item = &'a RunRecord>) -> Vec<(String, usize)> {
    let mut v: Vec<(String, usize)> = Vec::new();
    for r in runs {
        add_build(&mut v, &r.build_id);
    }
    v
}

fn add_build(v: &mut Vec<(String, usize)>, id: &str) {
    match v.iter_mut().find(|(b, _)| b == id) {
        Some((_, n)) => *n += 1,
        None => v.push((id.to_string(), 1)),
    }
}

fn builds_json(v: &[(String, usize)]) -> Value {
    Value::Array(v.iter().map(|(b, n)| json!({"build_id": b, "n": n})).collect())
}

fn roof_points<'a>(runs: impl IntoIterator<Item = &'a RunRecord>) -> Vec<Value> {
    runs.into_iter().filter_map(roof_point).collect()
}

/// Scene 2 (benchmark bars, M sweep) and Scene 3 (tradeoff gain) points over
/// every record of the source (not capped like `runs`): the newest record per
/// (M, K, N, mode, requant) whose (K, N) is a benchmark shape, so a long session
/// of other runs never pushes the benchmark sweep off the screen.
#[derive(Default)]
struct BenchView {
    newest: std::collections::BTreeMap<(u64, u64, u64, String, bool), RunRecord>,
}

pub const BENCH_RULE: &str =
    "newest record per (M, K, N, mode, requant) of a benchmark shape (K, N), over every record of the source (uncapped)";

impl BenchView {
    fn from_records<'a>(runs: impl IntoIterator<Item = &'a RunRecord>) -> Self {
        let mut v = Self::default();
        for r in runs {
            v.add(r);
        }
        v
    }

    fn add(&mut self, r: &RunRecord) {
        let g = &r.gemm;
        if spec::BENCH_SHAPES.iter().any(|b| b.k as u64 == g.k && b.n as u64 == g.n) {
            self.newest.insert((g.m, g.k, g.n, g.mode.clone(), g.requant), r.clone());
        }
    }

    /// The selected records, oldest first (records are added in run_id order).
    fn json(&self, from: &str) -> Value {
        let mut v: Vec<&RunRecord> = self.newest.values().collect();
        v.sort_by(|a, b| a.run_id.cmp(&b.run_id));
        json!({"runs": v, "from": from, "rule": BENCH_RULE})
    }
}

/// Shape key for pairing runs of the same GEMM shape across modes.
type ShapeKey = (u64, u64, u64, bool);
/// A clean measured run: (cycles_total, run_id, build_id).
type MeasuredRun = (u64, String, String);

/// Auto-mode view (G5 stretch), over every record of the source (not capped):
/// the auto runs themselves, and the latest measured cycles per (shape, mode)
/// from every record that passed all checks with all 12 counters equal to the
/// model, so "STRIP lost" can be shown measured where both modes were run.
#[derive(Default)]
struct AutoView {
    /// Auto records (compact), in record order.
    rows: Vec<Value>,
    tally: crate::batch::AutoTally,
    /// shape -> [FULL, STRIP] latest (measured cycles_total, run_id, build_id).
    measured: std::collections::BTreeMap<ShapeKey, [Option<MeasuredRun>; 2]>,
    /// Same, per build: (build_id, shape) -> [FULL, STRIP] latest measured cycles.
    measured_by_build: std::collections::BTreeMap<(String, ShapeKey), [Option<u64>; 2]>,
}

/// Show at most this many auto rows in the snapshot (the tally covers all).
const AUTO_ROWS: usize = 200;

impl AutoView {
    fn from_records<'a>(runs: impl IntoIterator<Item = &'a RunRecord>) -> Self {
        let mut v = Self::default();
        for r in runs {
            v.add(r);
        }
        v
    }

    fn add(&mut self, r: &RunRecord) {
        let key = (r.gemm.m, r.gemm.k, r.gemm.n, r.gemm.requant);
        let clean = r.verified() && r.check.counter_diffs.is_empty();
        if clean {
            let i = usize::from(r.gemm.mode == "STRIP");
            self.measured.entry(key).or_default()[i] =
                Some((r.counters.cycles_total, r.run_id.clone(), r.build_id.clone()));
            self.measured_by_build.entry((r.build_id.clone(), key)).or_default()[i] = Some(r.counters.cycles_total);
        }
        let Some(sel) = &r.mode_select else { return };
        self.tally.add(r);
        self.rows.push(json!({
            "run_id": r.run_id, "build_id": r.build_id, "source": r.source,
            "name": r.gemm.name, "M": r.gemm.m, "K": r.gemm.k, "N": r.gemm.n, "requant": r.gemm.requant,
            "chosen": sel.chosen, "measured_cycles": r.counters.cycles_total,
            "model_cycles_full": sel.model_cycles_full, "model_cycles_strip": sel.model_cycles_strip,
            "model_tiles_full": sel.model_tiles_full, "model_tiles_strip": sel.model_tiles_strip,
            "verified": r.verified(), "counters_exact": r.check.counter_diffs.is_empty(),
        }));
    }

    /// Same-shape pairs measured in both modes: how often STRIP was slower, and
    /// whether the model's pick (`choose_mode`, ties -> FULL) was the measured
    /// faster mode (a measured tie counts FULL as faster, the same rule).
    fn pairs(&self) -> Value {
        let (mut n, mut strip_slower, mut agree, mut worst) = (0u64, 0u64, 0u64, 1.0f64);
        let mut worst_shape = Value::Null;
        let mut slower: Vec<(f64, Value)> = Vec::new();
        let mut builds: Vec<(String, usize)> = Vec::new();
        for (&(m, k, nn, rq), [f, s]) in &self.measured {
            let (Some(f), Some(s)) = (f, s) else { continue };
            n += 1;
            add_build(&mut builds, &f.2);
            add_build(&mut builds, &s.2);
            let measured_pick = if s.0 < f.0 { "STRIP" } else { "FULL" };
            let pick = runner::select_mode(m as usize, k as usize, nn as usize, rq).ok();
            if s.0 > f.0 {
                strip_slower += 1;
                let x = s.0 as f64 / f.0.max(1) as f64;
                slower.push((x, json!({"M": m, "K": k, "N": nn, "requant": rq, "full": f.0, "strip": s.0,
                    "full_run_id": f.1, "strip_run_id": s.1, "model_pick": pick.as_ref().map(|p| p.chosen.clone())})));
                if x > worst {
                    worst = x;
                    worst_shape = json!({"M": m, "K": k, "N": nn, "requant": rq, "full": f.0, "strip": s.0});
                }
            }
            if pick.is_some_and(|p| p.chosen == measured_pick) {
                agree += 1;
            }
        }
        // The same score within each build (both modes measured on that build),
        // so a pooled count can be stated next to a single-build one.
        let mut per: Vec<(String, u64, u64)> = Vec::new();
        for ((b, (m, k, nn, rq)), [f, s]) in &self.measured_by_build {
            let (Some(f), Some(s)) = (f, s) else { continue };
            let measured_pick = if s < f { "STRIP" } else { "FULL" };
            let ok = runner::select_mode(*m as usize, *k as usize, *nn as usize, *rq).ok().is_some_and(|p| p.chosen == measured_pick);
            match per.iter_mut().find(|x| &x.0 == b) {
                Some(x) => {
                    x.1 += 1;
                    x.2 += u64::from(ok);
                }
                None => per.push((b.clone(), 1, u64::from(ok))),
            }
        }
        per.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let by_build: Vec<Value> =
            per.iter().map(|(b, n, ok)| json!({"build_id": b, "n": n, "model_pick_is_measured_faster": ok})).collect();
        // Largest STRIP/FULL ratios first (the panel shows a few).
        slower.sort_by(|a, b| b.0.total_cmp(&a.0));
        let slower: Vec<Value> = slower.into_iter().take(6).map(|(_, v)| v).collect();
        json!({"n": n, "strip_slower": strip_slower, "model_pick_is_measured_faster": agree,
               "worst_strip_over_full": worst, "worst": worst_shape, "slowest": slower,
               "builds": builds_json(&builds), "by_build": by_build})
    }

    fn json(&self, from: &str) -> Value {
        let skip = self.rows.len().saturating_sub(AUTO_ROWS);
        let rows: Vec<Value> = self.rows[skip..]
            .iter()
            .map(|row| {
                let mut row = row.clone();
                let key = (
                    row["M"].as_u64().unwrap_or(0),
                    row["K"].as_u64().unwrap_or(0),
                    row["N"].as_u64().unwrap_or(0),
                    row["requant"].as_bool().unwrap_or(false),
                );
                // The un-run mode, measured, if some other record ran this shape in it.
                let other = usize::from(row["chosen"] != "STRIP");
                row["other_measured"] = self
                    .measured
                    .get(&key)
                    .and_then(|p| p[other].as_ref())
                    .map_or(Value::Null, |(c, id, b)| json!({"cycles": c, "run_id": id, "build_id": b}));
                row
            })
            .collect();
        json!({
            "rows": rows, "rows_total": self.rows.len(), "tally": self.tally, "pairs": self.pairs(),
            "from": from, "policy": runner::AUTO_POLICY,
            "rule": "measured = counters.cycles_total of the mode that ran; the other mode is MODEL unless a separate verified run of the same shape measured it",
        })
    }
}

/// What the dashboard shows for a source, all derived from results/<source>/*.json:
/// totals over every record, the most recent `MAX_RUNS` records (ticker, scenes
/// 1-2), and the roofline points over every record.
struct Live {
    session: Session,
    runs: VecDeque<RunRecord>,
    roof: Vec<Value>,
    /// Number of records on disk (`runs` holds at most `MAX_RUNS` of them).
    total: usize,
    builds: Vec<(String, usize)>,
    auto: AutoView,
    bench: BenchView,
}

fn load_live(results: &Path, source: &str) -> Live {
    let all = load_runs(&runs_dir(results, source));
    let roof = roof_points(&all);
    let builds = build_counts(&all);
    let auto = AutoView::from_records(&all);
    let bench = BenchView::from_records(&all);
    let total = all.len();
    let mut runs: VecDeque<RunRecord> = all.into();
    while runs.len() > MAX_RUNS {
        runs.pop_front();
    }
    Live { session: Session::rebuild(results, source), runs, roof, total, builds, auto, bench }
}

impl App {
    fn new(source: &str, transport: &TransportSpec, results: &Path, replay: Option<Replay>, attract: bool) -> Self {
        let Live { session, runs, roof, total, builds, auto, bench } = load_live(results, source);
        Self {
            roof,
            builds,
            auto,
            bench,
            runs_total: total,
            source: source.into(),
            transport: format!("{transport:?}"),
            board_status: "connecting",
            board_error: None,
            build_id: None,
            proto_version: None,
            last_seen_utc: None,
            link: Value::Null,
            session,
            runs,
            running: None,
            queue_len: 0,
            scene: 1,
            attract,
            demo_running: false,
            replay_active: replay.is_some(),
            highlight: vec![],
            replay,
            last_selftest: None,
            started_utc: iso_utc(now_unix()),
            lm_busy: false,
            lm_live: None,
            replay_lm: None,
            lm_stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Adopt a board that just answered PING. The live label is the transport's
    /// source unless the board reports the mock build id ("MOCK"), which is MODEL
    /// data on any transport (`mock-bridge` over tcp:): source "model", records
    /// in results/mock, DEMO DATA watermark, never counted as verified. When the
    /// label changes, the shown runs/totals are swapped to that source's records
    /// so SIM and MODEL are never mixed on screen.
    fn adopt_board(&mut self, results: &Path, transport_src: transport::Source, build_id_hex: &str) {
        let src = runner::effective_source(transport_src, Some(build_id_hex));
        if self.source != src {
            let Live { session, runs, roof, total, builds, auto, bench } = load_live(results, src);
            self.source = src.into();
            self.auto = auto;
            self.bench = bench;
            self.session = session;
            self.runs = runs;
            self.roof = roof;
            self.runs_total = total;
            self.builds = builds;
        }
        self.build_id = Some(build_id_hex.into());
    }
}

type Subs = Arc<Mutex<Vec<Sender<String>>>>;

#[derive(Clone)]
struct Shared {
    app: Arc<Mutex<App>>,
    subs: Subs,
    cmd: Sender<Cmd>,
    results: PathBuf,
    assets: Option<PathBuf>,
    /// LM record summaries by path, keyed on (len, mtime) so the replay list
    /// does not re-parse ~1 MB records on every poll.
    lm_index: Arc<Mutex<LmIndex>>,
}

type LmIndex = std::collections::HashMap<PathBuf, (u64, Option<std::time::SystemTime>, Option<Value>)>;

fn broadcast(subs: &Subs, event: &str, data: &Value) {
    let msg = format!("event: {event}\ndata: {data}\n\n");
    let mut s = subs.lock().expect("subs");
    s.retain(|tx| tx.send(msg.clone()).is_ok());
}

fn read_json(p: &Path) -> Option<Value> {
    std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

impl Shared {
    fn snapshot(&self) -> Value {
        let a = self.app.lock().expect("app");
        let model_dir = self.results.join("model");
        let cost = read_json(&self.results.join("cost.json"));
        let (runs, session, source, board_status, roof, roof_from, builds, auto, bench) = match (&a.replay, a.replay_active) {
            (Some(r), true) => (
                serde_json::to_value(&r.runs).unwrap_or(Value::Null),
                r.session.clone().unwrap_or(Value::Null),
                "replay".to_string(),
                "replay",
                roof_points(&r.runs),
                "replay".to_string(),
                builds_json(&build_counts(&r.runs)),
                AutoView::from_records(&r.runs).json("replay"),
                BenchView::from_records(&r.runs).json("replay"),
            ),
            _ => (
                serde_json::to_value(&a.runs).unwrap_or(Value::Null),
                serde_json::to_value(&a.session).unwrap_or(Value::Null),
                a.source.clone(),
                a.board_status,
                a.roof.clone(),
                format!("results/{}", runs_dir(Path::new(""), &a.source).display()),
                builds_json(&a.builds),
                a.auto.json(&format!("results/{}", runs_dir(Path::new(""), &a.source).display())),
                a.bench.json(&format!("results/{}", runs_dir(Path::new(""), &a.source).display())),
            ),
        };
        let mut v = json!({
            "now_utc": iso_utc(now_unix()),
            "started_utc": a.started_utc,
            "source": source,
            "live_source": a.source,
            "demo_data": source == "model",
            "mock": a.source == "model",
            "host_version": env!("CARGO_PKG_VERSION"),
            "host_git_hash": runner::HOST_GIT_HASH,
            "transport": a.transport,
            // Serial line rate of the live board (null for tcp / mock); also inside `transport`.
            "baud": transport::baud_of(&a.transport).map(|b| json!({"rate": b, "label": transport::baud_label(b)})),
            "board": {
                "status": board_status,
                "error": a.board_error,
                "build_id": a.build_id,
                "proto_version": a.proto_version,
                "last_seen_utc": a.last_seen_utc,
                "link": a.link,
            },
            "clock_mhz": spec::CLK_HZ / 1_000_000,
            "array": { "rows": spec::ROWS, "cols": spec::COLS, "quad": spec::QUAD },
            "latencies": spec::LATENCIES.iter().map(|l| json!({"name": l.name, "value": l.value, "provisional": l.provisional})).collect::<Vec<_>>(),
            "latencies_provisional": cycle::spec::LATENCIES_PROVISIONAL,
            "bench_shapes": spec::BENCH_SHAPES.iter().map(|b| json!({"name": b.name, "K": b.k, "N": b.n})).collect::<Vec<_>>(),
            "sweep_m": spec::BENCH_M_SWEEP,
            "session": session,
            "runs": runs,
            "running": a.running,
            "queue_len": a.queue_len,
            "scene": a.scene,
            "attract": a.attract,
            "demo_running": a.demo_running,
            "highlight": a.highlight,
            "replay": a.replay.as_ref().map(|r| json!({"dir": r.dir, "recorded_utc": r.recorded_utc, "runs": r.runs.len(), "active": a.replay_active})),
            "selftest": a.last_selftest,
            "cost": cost,
            "cost_path": "results/cost.json",
            "sweep": read_json(&model_dir.join("sweep.json")),
            "roofline": read_json(&model_dir.join("roofline.json")),
            "builds_all": builds,
            "runs_total": if a.replay_active { a.replay.as_ref().map_or(0, |r| r.runs.len()) } else { a.runs_total },
            "max_runs": MAX_RUNS,
            "roof_measured": {"n": roof.len(), "points": roof, "from": roof_from, "rule": ROOF_RULE},
        });
        // (Outside json!: the macro's recursion limit.)
        v["auto"] = auto;
        v["bench"] = bench;
        v["auto_presets"] = AUTO_PRESETS
            .iter()
            .map(|p| json!({"case": p.case, "name": p.name, "M": p.m, "K": p.k, "N": p.n, "why": p.why}))
            .collect();
        v
    }
}

/// Model timeline for the PE-grid animation (first few tiles only; labelled model).
fn running_info(g: &GemmSpec) -> Value {
    let Ok((d, c)) = runner::model_for(g) else {
        return json!({"gemm": g});
    };
    let lat = cycle::Latencies::default();
    let tl: Vec<_> = cycle::timeline(&d, &lat)
        .into_iter()
        .filter(|e| e.tile.is_none_or(|t| t < 4))
        .collect();
    json!({
        "gemm": g,
        "model_cycles_total": c.cycles_total,
        "model_tiles": c.tiles,
        "l_rd": lat.l_rd,
        "timeline": tl,
        "timeline_source": "model",
        "started_utc": iso_utc(now_unix()),
    })
}

enum CmdKind {
    Demo,
    Other,
}

/// `POST /api/run {"preset": "auto"}`: named auto-mode presets (G5 stretch). The
/// host picks each one's mode with the cycle model; the record shows the choice.
pub struct AutoPreset {
    pub case: &'static str,
    pub name: &'static str,
    pub m: usize,
    pub k: usize,
    pub n: usize,
    pub why: &'static str,
}

pub const AUTO_PRESETS: &[AutoPreset] = &[
    AutoPreset { case: "strip_wins", name: "ffn_up", m: 1, k: 64, n: 256, why: "decode: FULL leaves 7 of 8 PE rows idle" },
    AutoPreset { case: "full_wins", name: "h5_worst", m: 22, k: 250, n: 17, why: "H-5 worst same-shape pair: STRIP needs 12 tiles, FULL 9" },
];

/// Specs for an auto preset: one case, or all of them in order.
pub fn auto_preset_specs(case: Option<&str>) -> Result<Vec<GemmSpec>, String> {
    let v: Vec<GemmSpec> = AUTO_PRESETS
        .iter()
        .filter(|p| case.is_none_or(|c| c == p.case))
        .map(|p| GemmSpec { name: p.name.into(), m: p.m, k: p.k, n: p.n, mode: "AUTO".into(), requant: true, seed: 1, mode_select: None })
        .collect();
    if v.is_empty() {
        let cases: Vec<&str> = AUTO_PRESETS.iter().map(|p| p.case).collect();
        return Err(format!("unknown auto case (known: {})", cases.join(", ")));
    }
    Ok(v)
}

fn worker(opts_t: TransportSpec, baud: transport::BaudSel, scale: Option<u32>, root: PathBuf, sh: Shared, rx: Receiver<Cmd>) {
    let mut ctx = Context::new(sh.results.clone(), &root);
    let mut board: Option<Board> = None;
    let mut cache = LoadCache::default();
    let mut last_ping = Instant::now() - Duration::from_secs(60);
    // The serial rate that answered last (tried first on reconnect with --baud auto).
    let mut good_baud: Option<u32> = None;
    let set_status = |sh: &Shared, status: &'static str, err: Option<String>, b: Option<&Board>| {
        let mut a = sh.app.lock().expect("app");
        let changed = a.board_status != status || a.board_error != err;
        a.board_status = status;
        a.board_error = err;
        if let Some(b) = b {
            a.link = serde_json::to_value(b.stats).unwrap_or(Value::Null);
        }
        if status == "live" {
            a.last_seen_utc = Some(iso_utc(now_unix()));
        }
        let v = json!({"status": a.board_status, "error": a.board_error, "build_id": a.build_id});
        drop(a);
        if changed {
            broadcast(&sh.subs, "board_status", &v);
        }
    };
    let mut queue: VecDeque<Cmd> = VecDeque::new();
    let mut last_scan = Instant::now();
    loop {
        // Pick up runs and session totals written by other processes (e.g. `bench`).
        if last_scan.elapsed() > Duration::from_secs(5) {
            last_scan = Instant::now();
            let src = sh.app.lock().expect("app").source.clone();
            let live = load_live(&sh.results, &src);
            let mut a = sh.app.lock().expect("app");
            if a.source == src {
                if live.runs.len() >= a.runs.len() {
                    a.runs = live.runs;
                }
                a.session = live.session;
                a.roof = live.roof;
                a.runs_total = live.total;
                a.builds = live.builds;
                a.auto = live.auto;
            }
        }
        // Gather commands.
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(c) => queue.push_back(c),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        while let Ok(c) = rx.try_recv() {
            queue.push_back(c);
        }
        // (Re)connect.
        if board.is_none() && last_ping.elapsed() > Duration::from_secs(2) {
            last_ping = Instant::now();
            match crate::client::connect(&opts_t, baud, good_baud, scale, MockConfig::default()) {
                Ok(c) => {
                    let (b, p) = (c.board, c.ping);
                    if c.baud.is_some() {
                        good_baud = c.baud;
                    }
                    let id = format!("{:08x}", p.build_id);
                    ctx.build_id = Some(id.clone());
                    cache.clear();
                    {
                        let mut a = sh.app.lock().expect("app");
                        a.adopt_board(&sh.results, b.source(), &id);
                        a.proto_version = Some(p.version);
                        a.transport = b.describe();
                    }
                    set_status(&sh, "live", None, Some(&b));
                    board = Some(b);
                }
                Err(e) => set_status(&sh, "disconnected", Some(e), None),
            }
        }
        // Heartbeat.
        if let Some(b) = board.as_mut() {
            if queue.is_empty() && last_ping.elapsed() > Duration::from_secs(3) {
                last_ping = Instant::now();
                match b.ping() {
                    Ok(_) => set_status(&sh, "live", None, Some(b)),
                    Err(e) => {
                        set_status(&sh, "disconnected", Some(e.to_string()), None);
                        board = None;
                    }
                }
            }
        }
        sh.app.lock().expect("app").queue_len = queue.len();
        let Some(cmd) = queue.pop_front() else {
            continue;
        };
        let Some(b) = board.as_mut() else {
            // Keep the command until the board is back (but do not pile up).
            if queue.len() < 8 {
                queue.push_front(cmd);
            }
            continue;
        };
        let cmd_kind = if matches!(cmd, Cmd::Demo) { CmdKind::Demo } else { CmdKind::Other };
        let specs: Vec<GemmSpec> = match cmd {
            Cmd::Run(g) => vec![g],
            Cmd::RunMany(v) => v,
            Cmd::Demo => {
                {
                    let mut a = sh.app.lock().expect("app");
                    a.demo_running = true;
                    a.highlight.clear();
                    a.scene = 1;
                }
                broadcast(&sh.subs, "scene", &json!({"scene": 1}));
                [(1, "FULL"), (1, "STRIP"), (64, "FULL"), (64, "STRIP")]
                    .iter()
                    .filter_map(|&(m, mode)| GemmSpec::named("ffn_up", m, mode, 1).ok())
                    .collect()
            }
            Cmd::Selftest => {
                let r = b.selftest();
                cache.clear(); // the self-test overwrites the buffers
                let v = match r {
                    Ok(s) => json!({"pass_full": s.pass_full, "pass_strip": s.pass_strip,
                        "fletcher_full": format!("{:08x}", s.fletcher_full),
                        "fletcher_strip": format!("{:08x}", s.fletcher_strip), "utc": iso_utc(now_unix())}),
                    Err(e) => json!({"error": e.to_string(), "utc": iso_utc(now_unix())}),
                };
                sh.app.lock().expect("app").last_selftest = Some(v.clone());
                broadcast(&sh.subs, "state", &json!({"selftest": v}));
                continue;
            }
            Cmd::Lm(o) => {
                if !lm_worker(b, &mut ctx, &root, &sh, &o) {
                    set_status(&sh, "disconnected", Some("board stopped answering during the LM run".into()), None);
                    board = None;
                }
                cache.clear(); // the LM run overwrote WBUF/ABUF/OBUF
                last_ping = Instant::now();
                continue;
            }
        };
        let is_demo = matches!(cmd_kind, CmdKind::Demo);
        let paced = specs.len() > 1;
        let n_specs = specs.len();
        for (i, g) in specs.into_iter().enumerate() {
            // AUTO -> the model's pick, before announcing it (the record says why).
            let g = match g.resolve() {
                Ok(g) => g,
                Err(e) => {
                    broadcast(&sh.subs, "run_failed", &json!({"gemm": g, "error": e}));
                    continue;
                }
            };
            let info = running_info(&g);
            sh.app.lock().expect("app").running = Some(info.clone());
            broadcast(&sh.subs, "run_started", &info);
            let t0 = Instant::now();
            let res = runner::run_gemm(b, &mut ctx, &mut cache, &g);
            match res {
                Ok((rec, _path)) => {
                    let sess_v;
                    {
                        let mut a = sh.app.lock().expect("app");
                        // Totals are derived from the records on disk (this run's
                        // record is already written), so concurrent hosts never lose updates.
                        if let Ok(sess) = Session::rebuild_and_write(&sh.results, &a.source) {
                            a.session = sess;
                        }
                        a.roof.extend(roof_point(&rec));
                        a.auto.add(&rec);
                        a.bench.add(&rec);
                        a.runs_total += 1;
                        add_build(&mut a.builds, &rec.build_id);
                        a.runs.push_back(rec.clone());
                        while a.runs.len() > MAX_RUNS {
                            a.runs.pop_front();
                        }
                        a.running = None;
                        if is_demo {
                            a.highlight.push(rec.run_id.clone());
                        }
                        sess_v = serde_json::to_value(&a.session).unwrap_or(Value::Null);
                    }
                    set_status(&sh, "live", None, Some(b));
                    broadcast(&sh.subs, "run_finished", &json!({"record": rec, "session": sess_v}));
                }
                Err(e) => {
                    sh.app.lock().expect("app").running = None;
                    let msg = e.to_string();
                    broadcast(&sh.subs, "run_failed", &json!({"gemm": g, "error": msg}));
                    if matches!(e, runner::RunError::Host(crate::client::HostError::Io(_) | crate::client::HostError::Timeout { .. })) {
                        set_status(&sh, "disconnected", Some(msg), None);
                        board = None;
                        break;
                    }
                }
            }
            if paced && i + 1 < n_specs {
                // Let the viewer see each result (DEMO.md: 1.5 s between runs),
                // at least as long as the model animation takes.
                let min = Duration::from_millis(3000);
                if t0.elapsed() < min {
                    thread::sleep(min - t0.elapsed());
                }
            }
        }
        if is_demo {
            thread::sleep(Duration::from_millis(2500));
            {
                let mut a = sh.app.lock().expect("app");
                a.demo_running = false;
                a.scene = 2;
            }
            broadcast(&sh.subs, "scene", &json!({"scene": 2}));
        }
        last_ping = Instant::now();
    }
}

fn attract_loop(sh: Shared) {
    let mut rng = splitsys_golden::rng::Rng::new(now_unix() as u64);
    let mut last_run = Instant::now();
    let mut last_scene = Instant::now();
    loop {
        thread::sleep(Duration::from_millis(500));
        let (on, busy) = {
            let a = sh.app.lock().expect("app");
            (a.attract && !a.replay_active, a.demo_running || a.running.is_some() || a.queue_len > 0 || a.lm_busy)
        };
        if !on {
            last_run = Instant::now();
            last_scene = Instant::now();
            continue;
        }
        if last_scene.elapsed() >= Duration::from_secs(20) {
            last_scene = Instant::now();
            let s = {
                let mut a = sh.app.lock().expect("app");
                a.scene = a.scene % 3 + 1;
                a.scene
            };
            broadcast(&sh.subs, "scene", &json!({"scene": s}));
        }
        if !busy && last_run.elapsed() >= Duration::from_secs(5) {
            last_run = Instant::now();
            let b = rng.pick(&spec::BENCH_SHAPES);
            let m = *rng.pick(&spec::BENCH_M_SWEEP);
            let mode = if rng.chance(1, 2) { "FULL" } else { "STRIP" };
            if let Ok(g) = GemmSpec::named(b.name, m, mode, 1) {
                let _ = sh.cmd.send(Cmd::Run(g));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

struct Req {
    method: String,
    path: String,
    body: Vec<u8>,
}

fn read_req(s: &mut TcpStream) -> Option<Req> {
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut r = BufReader::new(s.try_clone().ok()?);
    let mut line = String::new();
    r.read_line(&mut line).ok()?;
    let mut it = line.split_whitespace();
    let method = it.next()?.to_string();
    let path = it.next()?.split('?').next()?.to_string();
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).ok()? == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().unwrap_or(0).min(65536);
            }
        }
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).ok()?;
    Some(Req { method, path, body })
}

fn respond(s: &mut TcpStream, code: u16, ctype: &str, body: &[u8]) {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Error",
    };
    let hdr = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(hdr.as_bytes());
    let _ = s.write_all(body);
    close_gracefully(s);
}

/// Half-close and drain before dropping, so the OS sends FIN, not RST (an RST
/// with unread client bytes can make the browser drop the response; seen through
/// the WSL2 localhost relay).
fn close_gracefully(s: &mut TcpStream) {
    let _ = s.flush();
    let _ = s.shutdown(std::net::Shutdown::Write);
    s.set_read_timeout(Some(Duration::from_millis(200))).ok();
    let mut buf = [0u8; 1024];
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_millis(500) {
        match s.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

fn json_resp(s: &mut TcpStream, code: u16, v: &Value) {
    respond(s, code, "application/json", v.to_string().as_bytes());
}

fn asset(sh: &Shared, name: &str, embedded: &'static str) -> String {
    if let Some(dir) = &sh.assets {
        if let Ok(s) = std::fs::read_to_string(dir.join(name)) {
            return s;
        }
    }
    embedded.to_string()
}

fn handle(sh: Shared, mut s: TcpStream) {
    let Some(req) = read_req(&mut s) else {
        return;
    };
    let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => {
            let b = asset(&sh, "index.html", INDEX_HTML);
            respond(&mut s, 200, "text/html; charset=utf-8", b.as_bytes())
        }
        ("GET", "/app.js") => {
            let b = asset(&sh, "app.js", APP_JS);
            respond(&mut s, 200, "text/javascript; charset=utf-8", b.as_bytes())
        }
        ("GET", "/lm.js") => {
            let b = asset(&sh, "lm.js", LM_JS);
            respond(&mut s, 200, "text/javascript; charset=utf-8", b.as_bytes())
        }
        ("GET", "/api/lm") => json_resp(&mut s, 200, &lm_state(&sh)),
        ("POST", "/api/lm") => {
            if body["stop"].as_bool() == Some(true) {
                let a = sh.app.lock().expect("app");
                a.lm_stop.store(true, std::sync::atomic::Ordering::Relaxed);
                return json_resp(&mut s, 200, &json!({"stopping": a.lm_busy}));
            }
            match lm_opts(&body) {
                Ok(o) => {
                    if sh.app.lock().expect("app").lm_busy {
                        return json_resp(&mut s, 409, &json!({"error": "a generation is already running"}));
                    }
                    let _ = sh.cmd.send(Cmd::Lm(o));
                    json_resp(&mut s, 200, &json!({"queued": true}))
                }
                Err(e) => json_resp(&mut s, 400, &json!({"error": e})),
            }
        }
        ("GET", "/style.css") => {
            let b = asset(&sh, "style.css", STYLE_CSS);
            respond(&mut s, 200, "text/css; charset=utf-8", b.as_bytes())
        }
        ("GET", "/api/state") => json_resp(&mut s, 200, &sh.snapshot()),
        ("GET", "/events") => {
            let (tx, rx) = mpsc::channel::<String>();
            sh.subs.lock().expect("subs").push(tx);
            let hdr = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: keep-alive\r\n\r\nretry: 1000\n\n";
            if s.write_all(hdr.as_bytes()).is_err() {
                return;
            }
            loop {
                let msg = match rx.recv_timeout(Duration::from_secs(10)) {
                    Ok(m) => m,
                    Err(mpsc::RecvTimeoutError::Timeout) => ": keepalive\n\n".into(),
                    Err(_) => return,
                };
                if s.write_all(msg.as_bytes()).and_then(|_| s.flush()).is_err() {
                    return;
                }
            }
        }
        ("POST", "/api/run") => {
            // Named presets only: a SPEC §1.1 benchmark, an M from the sweep, a mode
            // (full | strip | auto), or {"preset": "auto", "case"?: strip_wins | full_wins}.
            if body["preset"].as_str() == Some("auto") {
                return match auto_preset_specs(body["case"].as_str()) {
                    Ok(v) => {
                        let n = v.len();
                        let _ = sh.cmd.send(if n == 1 { Cmd::Run(v.into_iter().next().expect("one")) } else { Cmd::RunMany(v) });
                        json_resp(&mut s, 200, &json!({"queued": n}))
                    }
                    Err(e) => json_resp(&mut s, 400, &json!({"error": e})),
                };
            }
            let name = body["preset"].as_str().or(body["name"].as_str()).unwrap_or("");
            let m = body["m"].as_u64().or(body["M"].as_u64()).unwrap_or(0) as usize;
            let mode = body["mode"].as_str().unwrap_or("");
            if !spec::BENCH_M_SWEEP.contains(&m) {
                return json_resp(&mut s, 400, &json!({"error": format!("M must be one of {:?}", spec::BENCH_M_SWEEP)}));
            }
            match GemmSpec::named(name, m, mode, 1) {
                Ok(g) => {
                    let _ = sh.cmd.send(Cmd::Run(g));
                    json_resp(&mut s, 200, &json!({"queued": true}))
                }
                Err(e) => json_resp(&mut s, 400, &json!({"error": e})),
            }
        }
        ("POST", "/api/scene") => {
            let sc = body["scene"].as_u64().unwrap_or(0);
            if !(1..=4).contains(&sc) {
                return json_resp(&mut s, 400, &json!({"error": "scene must be 1..4"}));
            }
            sh.app.lock().expect("app").scene = sc as u8;
            broadcast(&sh.subs, "scene", &json!({"scene": sc}));
            json_resp(&mut s, 200, &json!({"scene": sc}))
        }
        ("POST", "/api/demo") => {
            let _ = sh.cmd.send(Cmd::Demo);
            json_resp(&mut s, 200, &json!({"queued": true}))
        }
        ("POST", "/api/selftest") => {
            let _ = sh.cmd.send(Cmd::Selftest);
            json_resp(&mut s, 200, &json!({"queued": true}))
        }
        ("POST", "/api/attract") => {
            let on = {
                let mut a = sh.app.lock().expect("app");
                a.attract = body["on"].as_bool().unwrap_or(!a.attract);
                a.attract
            };
            broadcast(&sh.subs, "state", &json!({"attract": on}));
            json_resp(&mut s, 200, &json!({"attract": on}))
        }
        ("POST", "/api/replay") => {
            let r = {
                let mut a = sh.app.lock().expect("app");
                if a.replay.is_none() {
                    Err("no replay session loaded (start serve with --replay <dir>)")
                } else {
                    a.replay_active = body["on"].as_bool().unwrap_or(!a.replay_active);
                    Ok(a.replay_active)
                }
            };
            match r {
                Ok(on) => {
                    broadcast(&sh.subs, "state", &json!({"replay": on}));
                    json_resp(&mut s, 200, &json!({"replay": on}))
                }
                Err(e) => json_resp(&mut s, 400, &json!({"error": e})),
            }
        }
        ("POST", "/api/lm_replay") => {
            // Pick the Scene 4 REPLAY generation by gen id (listed in GET /api/lm replay_list).
            let dir = sh.app.lock().expect("app").replay.as_ref().map(|r| PathBuf::from(&r.dir).join("lm"));
            let id = body["gen_id"].as_str().unwrap_or("").to_string();
            let r = match &dir {
                None => Err("no replay session loaded (start serve with --replay <dir>)".to_string()),
                Some(d) if lm_list(d, &sh.lm_index).iter().any(|x| x["gen_id"] == id.as_str()) => Ok(id),
                Some(d) => Err(format!("no LM record {id:?} in {}", d.display())),
            };
            match r {
                Ok(id) => {
                    sh.app.lock().expect("app").replay_lm = Some(id.clone());
                    broadcast(&sh.subs, "state", &json!({"replay_lm": id}));
                    json_resp(&mut s, 200, &json!({"replay_lm": id}))
                }
                Err(e) => json_resp(&mut s, 400, &json!({"error": e})),
            }
        }
        _ => respond(&mut s, 404, "text/plain", b"not found"),
    }
}

// ---------------------------------------------------------------- Scene 4 (tiny LM)

/// Scene 4 input limits (stated in the UI via GET /api/lm `limits`).
/// Prompt: printable ASCII, 1..=80 chars (at most 80 tokens + BOS even with the
/// tokenizer's byte fallback). New tokens: 1..=200, so prompt + new tokens stay
/// well under the model's seq_len (512 for stories260K); `lm::generate` also
/// clamps the step count to seq_len.
pub const LM_PROMPT_MAX: usize = 80;
pub const LM_TOKENS_MAX: u64 = 200;

/// POST /api/lm body -> options.
fn lm_opts(body: &Value) -> Result<crate::lm::GenOpts, String> {
    let prompt = body["prompt"].as_str().unwrap_or("Once upon a time").to_string();
    if prompt.is_empty() || prompt.len() > LM_PROMPT_MAX || !prompt.bytes().all(|c| (0x20..0x7f).contains(&c)) {
        return Err(format!("prompt: 1..{LM_PROMPT_MAX} printable ASCII characters"));
    }
    let tokens = match &body["tokens"] {
        Value::Null => 8,
        v => v.as_u64().ok_or_else(|| format!("tokens: a whole number 1..{LM_TOKENS_MAX}"))?,
    };
    if !(1..=LM_TOKENS_MAX).contains(&tokens) {
        return Err(format!("tokens: 1..{LM_TOKENS_MAX}"));
    }
    let mode = crate::lm::LmMode::parse(body["mode"].as_str().unwrap_or("auto"))?;
    Ok(crate::lm::GenOpts { prompt, max_new_tokens: tokens as usize, mode, poison: true, stop: None })
}

/// The measured laptop wall time per step of the newest record in `dir` made on
/// exactly this transport (`serial:COM3@1562500`: same port and baud), for the
/// Scene 4 estimate: the record's whole wall time (record writes included) over
/// its steps. Only the fields needed are parsed.
fn lm_rate(dir: &Path, transport: &str) -> Option<Value> {
    #[derive(serde::Deserialize)]
    struct Sum {
        wall_ms: f64,
    }
    #[derive(serde::Deserialize)]
    struct Step {
        summary: Sum,
    }
    #[derive(serde::Deserialize)]
    struct Rec {
        kind: String,
        gen_id: String,
        transport: String,
        #[serde(default)]
        error: Option<String>,
        #[serde(default)]
        steps: Vec<Step>,
        summary: RecSum,
    }
    #[derive(serde::Deserialize)]
    struct RecSum {
        #[serde(default)]
        wall_s: f64,
    }
    let mut names: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    names.sort_by_key(|p| crate::lm::gen_order_key(&p.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default()));
    names
        .iter()
        .rev()
        .filter_map(|p| std::fs::read(p).ok())
        .filter_map(|b| serde_json::from_slice::<Rec>(&b).ok())
        .find(|r| r.kind == crate::lm::LM_KIND && r.transport == transport && r.error.is_none() && !r.steps.is_empty())
        .map(|r| {
            let n = r.steps.len() as f64;
            let s = if r.summary.wall_s > 0.0 { r.summary.wall_s / n } else { r.steps.iter().map(|s| s.summary.wall_ms).sum::<f64>() / n / 1000.0 };
            json!({"s_per_step": s, "gen_id": r.gen_id, "transport": r.transport, "steps": r.steps.len(),
                "baud": crate::transport::baud_of(&r.transport)})
        })
}

/// Newest LM record in a directory (by file name = gen_id), as the dashboard view.
fn newest_lm(dir: &Path) -> (Option<Value>, usize) {
    let mut names: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    names.sort_by_key(|p| crate::lm::gen_order_key(&p.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default()));
    let n = names.len();
    let rec = names
        .iter()
        .rev()
        .filter_map(|p| std::fs::read(p).ok())
        .filter_map(|b| serde_json::from_slice::<crate::lm::LmRecord>(&b).ok())
        .find(|r| r.kind == crate::lm::LM_KIND);
    (rec.as_ref().map(crate::lm::record_view), n)
}

/// One line per LM record in `dir` (oldest first) for the Scene 4 REPLAY picker:
/// what the operator reads off the screen instead of reciting it.
fn lm_list(dir: &Path, cache: &Mutex<LmIndex>) -> Vec<Value> {
    let mut names: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    names.sort_by_key(|p| crate::lm::gen_order_key(&p.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default()));
    let mut c = cache.lock().expect("lm index");
    names
        .iter()
        .filter_map(|p| {
            let md = std::fs::metadata(p).ok()?;
            let (len, mt) = (md.len(), md.modified().ok());
            if let Some((l, m, v)) = c.get(p) {
                if *l == len && *m == mt {
                    return v.clone();
                }
            }
            let v = std::fs::read(p)
                .ok()
                .and_then(|b| serde_json::from_slice::<crate::lm::LmRecord>(&b).ok())
                .filter(|r| r.kind == crate::lm::LM_KIND)
                .map(|r| lm_summary(&r, p));
            c.insert(p.clone(), (len, mt, v.clone()));
            v
        })
        .collect()
}

fn lm_summary(r: &crate::lm::LmRecord, p: &Path) -> Value {
    let tokens = r.steps.iter().filter(|s| s.phase == "generate").count();
    json!({
        "gen_id": r.gen_id, "file": p.file_name().map(|x| x.to_string_lossy().into_owned()),
        "prompt": r.prompt, "build_id": r.build_id, "source": r.source, "mock": r.mock,
        "transport": r.transport, "baud": crate::transport::baud_of(&r.transport),
        "steps": r.steps.len(), "tokens_generated": tokens, "gemms": r.summary.gemms,
        "bit_exact": r.summary.bit_exact, "counters_eq_model": r.summary.counters_eq_model,
        "status": r.status, "verified": r.verified(), "started_utc": r.started_utc,
    })
}

/// Load one LM record as the dashboard view.
fn load_lm_view(p: &Path) -> Option<Value> {
    std::fs::read(p)
        .ok()
        .and_then(|b| serde_json::from_slice::<crate::lm::LmRecord>(&b).ok())
        .filter(|r| r.kind == crate::lm::LM_KIND)
        .map(|r| crate::lm::record_view(&r))
}

/// The REPLAY generation: `pick` as a gen id in `dir` or a record path; None = newest.
/// Returns (view, how it was chosen, error if the pick was not found).
fn replay_lm_view(dir: &Path, pick: Option<&str>) -> (Option<Value>, &'static str, Option<String>) {
    if let Some(id) = pick {
        let as_path = PathBuf::from(id);
        let p = if as_path.extension().and_then(|e| e.to_str()) == Some("json") && as_path.is_file() {
            as_path
        } else {
            dir.join(format!("{id}.json"))
        };
        if let Some(v) = load_lm_view(&p) {
            return (Some(v), "selected", None);
        }
        let (v, _) = newest_lm(dir);
        return (v, "newest", Some(format!("LM record {id:?} not found in {}; showing the newest", dir.display())));
    }
    (newest_lm(dir).0, "newest", None)
}

fn lm_state(sh: &Shared) -> Value {
    let (source, busy, live, replay_dir, replay_active, transport_now, replay_pick) = {
        let a = sh.app.lock().expect("app");
        (a.source.clone(), a.lm_busy, a.lm_live.clone(), a.replay.as_ref().map(|r| PathBuf::from(&r.dir)), a.replay_active, a.transport.clone(), a.replay_lm.clone())
    };
    let dir = crate::lm::lm_dir(&sh.results, &source);
    // While this server holds a live record, skip parsing the newest file on disk
    // (a 200-token record is ~7 MB and is rewritten every step); count only.
    let (disk, n_live) = if live.is_some() {
        let n = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
            .count();
        (None, n)
    } else {
        newest_lm(&dir)
    };
    let replay_list = replay_dir.as_ref().map_or_else(Vec::new, |d| lm_list(&d.join("lm"), &sh.lm_index));
    let n_replay = replay_list.len();
    let (replay, replay_how, replay_err) = match &replay_dir {
        Some(d) => replay_lm_view(&d.join("lm"), replay_pick.as_deref()),
        None => (None, "newest", None),
    };
    json!({
        "live_source": source,
        "running": busy,
        // A record this server ran wins over the newest file on disk.
        "live": live.clone().or(disk),
        "live_this_session": live.is_some(),
        "live_dir": format!("results/{}", runs_dir(Path::new(""), &source).join("lm").display()).replace('\\', "/"),
        "live_records": n_live,
        "replay": replay,
        "replay_dir": replay_dir.map(|d| d.join("lm").display().to_string().replace('\\', "/")),
        "replay_records": n_replay,
        "replay_list": replay_list,
        "replay_pick": replay_how,
        "replay_pick_error": replay_err,
        "model_origin": crate::lm::MODEL_ORIGIN,
        "replay_active": replay_active,
        "split": {"fpga": crate::lm::SPLIT_FPGA, "laptop": crate::lm::SPLIT_LAPTOP},
        "wall_note": crate::lm::wall_note(&source, source == "model", &transport_now),
        "numerics": crate::lm::NUMERICS,
        "limits": {"prompt_max": LM_PROMPT_MAX, "tokens_max": LM_TOKENS_MAX},
        "transport": transport_now,
        "rate": lm_rate(&dir, &transport_now),
    })
}

/// Run one generation on the worker thread. Returns false if the board stopped answering.
fn lm_worker(b: &mut Board, ctx: &mut Context, root: &Path, sh: &Shared, o: &crate::lm::GenOpts) -> bool {
    use crate::lm::{LmEvent, Stories};
    let dir = root.join(crate::lm::DEFAULT_MODEL_DIR);
    let be = match Stories::load(&dir) {
        Ok(be) => be,
        Err(e) => {
            broadcast(&sh.subs, "lm_failed", &json!({"error": e}));
            return true;
        }
    };
    let stop = {
        let mut a = sh.app.lock().expect("app");
        a.lm_busy = true;
        a.lm_stop.store(false, std::sync::atomic::Ordering::Relaxed);
        a.lm_stop.clone()
    };
    let mut o = o.clone();
    o.stop = Some(stop);
    let (_rec, err, _path) = crate::lm::generate(b, ctx, &be, &o, &mut |e| match e {
        LmEvent::Started(r) | LmEvent::Step { rec: r, .. } | LmEvent::Finished(r) => {
            let ev = match e {
                LmEvent::Started(_) => "lm_started",
                LmEvent::Finished(_) => "lm_finished",
                _ => "lm_step",
            };
            sh.app.lock().expect("app").lm_live = Some(crate::lm::record_view(r));
            broadcast(&sh.subs, ev, &json!({"gen_id": r.gen_id, "steps": r.steps.len(), "status": r.status}));
        }
        LmEvent::Call { step, call } => broadcast(
            &sh.subs,
            "lm_call",
            &json!({"step": step, "idx": call.idx, "layer": call.layer, "name": call.name, "mode": call.mode,
                "auto": call.auto, "bit_exact": call.bit_exact, "counters_eq_model": call.counters_eq_model,
                "w_sent": call.w_sent, "cycles": call.counters.cycles_total, "M": call.m, "K": call.k, "N": call.n}),
        ),
    });
    sh.app.lock().expect("app").lm_busy = false;
    match err {
        Some(_) => b.ping().is_ok(),
        None => true,
    }
}

#[cfg(test)]
mod lm_scene_tests {
    use super::*;

    #[test]
    fn lm_rate_uses_newest_record_on_the_same_transport() {
        let d = std::env::temp_dir().join(format!("splitsys-lm-rate-{}-{}", std::process::id(), now_unix() as u64));
        std::fs::create_dir_all(&d).unwrap();
        let rec = |name: &str, transport: &str, wall_s: f64, walls: &[f64], err: Option<&str>| {
            let steps: Vec<Value> = walls.iter().map(|w| json!({"summary": {"wall_ms": w}})).collect();
            let v = json!({"kind": crate::lm::LM_KIND, "gen_id": name, "transport": transport, "error": err,
                "summary": {"wall_s": wall_s}, "steps": steps});
            std::fs::write(d.join(format!("{name}.json")), v.to_string()).unwrap();
        };
        rec("2026-10-25T04-00-00Z-lm", "serial:COM3@1562500", 8.8, &[2000.0, 2000.0, 2000.0, 2000.0], None);
        rec("2026-10-25T05-00-00Z-lm", "serial:COM3@115200", 60.0, &[30000.0, 30000.0], None);
        rec("2026-10-25T06-00-00Z-lm", "serial:COM3@1562500", 1.0, &[500.0], Some("mismatch"));
        let r = lm_rate(&d, "serial:COM3@1562500").unwrap();
        assert_eq!(r["gen_id"], "2026-10-25T04-00-00Z-lm");
        assert!((r["s_per_step"].as_f64().unwrap() - 2.2).abs() < 1e-9, "{r}");
        assert_eq!(r["baud"], 1562500);
        assert_eq!(lm_rate(&d, "serial:COM3@115200").unwrap()["s_per_step"], 30.0);
        assert!(lm_rate(&d, "serial:COM5@1562500").is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn lm_opts_validates_prompt_tokens_and_mode() {
        let o = lm_opts(&json!({})).unwrap();
        assert_eq!((o.prompt.as_str(), o.max_new_tokens, o.mode), ("Once upon a time", 8, crate::lm::LmMode::Auto));
        assert!(lm_opts(&json!({"tokens": 0})).is_err());
        assert!(lm_opts(&json!({"tokens": 49})).is_ok());
        assert_eq!(lm_opts(&json!({"tokens": 200})).unwrap().max_new_tokens, 200);
        assert_eq!(lm_opts(&json!({"tokens": 201})).unwrap_err(), "tokens: 1..200");
        assert!(lm_opts(&json!({"tokens": -1})).is_err());
        assert!(lm_opts(&json!({"tokens": "8"})).is_err());
        assert!(lm_opts(&json!({"tokens": 2.5})).is_err());
        assert_eq!(lm_opts(&json!({"prompt": "x".repeat(80)})).unwrap().prompt.len(), 80);
        assert_eq!(lm_opts(&json!({"prompt": "caf\u{e9}"})).unwrap_err(), "prompt: 1..80 printable ASCII characters");
        assert!(lm_opts(&json!({"prompt": ""})).is_err());
        assert!(lm_opts(&json!({"prompt": "x".repeat(81)})).is_err());
        assert!(lm_opts(&json!({"prompt": "bad"})).is_err());
        assert!(lm_opts(&json!({"mode": "both"})).is_err());
        assert_eq!(lm_opts(&json!({"mode": "strip", "tokens": 200})).unwrap().mode, crate::lm::LmMode::Strip);
    }

    #[test]
    fn newest_lm_picks_the_latest_record_and_ignores_other_files() {
        let d = std::env::temp_dir().join(format!("splitsys-lm-newest-{}-{}", std::process::id(), now_unix() as u64));
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(newest_lm(&d).1, 0, "missing dir");
        let lm = crate::lm::fake::FakeLm::stories_like(1);
        let mut ctx = Context::new(d.clone(), &d);
        let o = crate::lm::GenOpts { prompt: "Hi".into(), max_new_tokens: 1, mode: crate::lm::LmMode::Full, poison: false, stop: None };
        for _ in 0..2 {
            let mut b = Board::new(Box::new(transport::MockTransport::new(MockConfig::default())));
            let (_r, e, _p) = crate::lm::generate(&mut b, &mut ctx, &lm, &o, &mut |_| {});
            assert!(e.is_none());
        }
        let dir = crate::lm::lm_dir(&d, "model");
        std::fs::write(dir.join("zzz-not-a-record.json"), b"{\"kind\":\"other\"}").unwrap();
        let (v, n) = newest_lm(&dir);
        assert_eq!(n, 3);
        let v = v.unwrap();
        let ids: Vec<String> = crate::lm::load_lm_records(&dir).into_iter().map(|r| r.gen_id).collect();
        assert_eq!(ids.len(), 2);
        assert_eq!(v["gen_id"], ids[1].as_str());
        assert_eq!(v["source"], "model");
        // GEMM-run totals ignore the lm/ subdirectory.
        assert_eq!(Session::rebuild(&d, "model").gemms_run, 0);
        // REPLAY picker: one summary per LM record, oldest first; pick by gen id or path.
        let cache = Mutex::new(LmIndex::new());
        let list = lm_list(&dir, &cache);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["gen_id"], ids[0].as_str());
        assert_eq!(list[0]["tokens_generated"], 1);
        assert_eq!(list[0]["prompt"], "Hi");
        assert!(list[0]["gemms"].as_u64().unwrap() > 0);
        assert_eq!(lm_list(&dir, &cache), list, "cached");
        let (v, how, err) = replay_lm_view(&dir, None);
        assert_eq!((v.unwrap()["gen_id"].clone(), how, err), (json!(ids[1]), "newest", None));
        let (v, how, err) = replay_lm_view(&dir, Some(&ids[0]));
        assert_eq!((v.unwrap()["gen_id"].clone(), how, err), (json!(ids[0]), "selected", None));
        let path = dir.join(format!("{}.json", ids[0])).display().to_string();
        assert_eq!(replay_lm_view(&dir, Some(&path)).1, "selected");
        let (v, how, err) = replay_lm_view(&dir, Some("nope"));
        assert_eq!((v.unwrap()["gen_id"].clone(), how), (json!(ids[1]), "newest"));
        assert!(err.unwrap().contains("not found"));
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// Run the dashboard server (blocks forever).
pub fn serve(o: ServeOpts) -> Result<(), String> {
    let addr = format!("127.0.0.1:{}", o.port);
    let listener = TcpListener::bind(&addr).map_err(|e| format!("cannot bind {addr}: {e}"))?;
    // Provisional until the first PING: a tcp: board that reports the mock
    // build id is relabelled MODEL (see `adopt_board`).
    let source = transport_source(&o.transport);
    let replay = o.replay.as_ref().map(|d| {
        let runs = load_runs(d);
        Replay {
            dir: d.display().to_string(),
            recorded_utc: runs.last().map(|r| r.run_id.chars().take(20).collect()).unwrap_or_default(),
            session: serde_json::to_value(Session::from_records(
                runs.first().map_or("hw", |r| r.source.as_str()),
                &runs,
            ))
            .ok(),
            runs,
        }
    });
    let mut app = App::new(source, &o.transport, &o.results, replay, o.attract);
    app.replay_lm = o.replay_lm.clone();
    if let (Some(id), Some(d)) = (&o.replay_lm, &o.replay) {
        match replay_lm_view(&d.join("lm"), Some(id)) {
            (Some(v), "selected", _) => eprintln!(
                "serve: Scene 4 REPLAY plays {} (build {}, {}, {} GEMMs)",
                v["gen_id"].as_str().unwrap_or("?"),
                v["build_id"].as_str().unwrap_or("?"),
                v["transport"].as_str().unwrap_or("?"),
                v["summary"]["gemms"]
            ),
            (_, _, e) => return Err(e.unwrap_or_else(|| format!("--replay-lm {id}: no LM record"))),
        }
    }
    let (tx, rx) = mpsc::channel();
    let sh = Shared {
        app: Arc::new(Mutex::new(app)),
        subs: Arc::new(Mutex::new(Vec::new())),
        cmd: tx,
        results: o.results.clone(),
        assets: o.assets.clone(),
        lm_index: Arc::new(Mutex::new(LmIndex::new())),
    };
    {
        let sh2 = sh.clone();
        let (t, baud, scale, root) = (o.transport.clone(), o.baud, o.timeout_scale, o.root.clone());
        thread::spawn(move || worker(t, baud, scale, root, sh2, rx));
    }
    {
        let sh2 = sh.clone();
        thread::spawn(move || attract_loop(sh2));
    }
    eprintln!(
        "splitsys-host serve: http://{addr}/  (source {source}; results {}; keys 1/2/3 scenes, Space demo, A attract, R replay)",
        o.results.display()
    );
    for s in listener.incoming().flatten() {
        let sh2 = sh.clone();
        thread::spawn(move || handle(sh2, s));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{repo_root, run_file_name, write_json};
    use crate::transport::fake::spawn_tcp_bridge;
    use crate::transport::MockTransport;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("splitsys_srv_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A real SIM-labelled record in results/sim (made from a mock run, relabelled),
    /// so a wrong label would show up as mixed data.
    fn seed_sim_record(results: &Path) -> String {
        let mut b = Board::new(Box::new(MockTransport::new(MockConfig::default())));
        let mut ctx = Context::new(tmp("seed"), &repo_root());
        let g = GemmSpec::named("ffn_up", 1, "FULL", 1).unwrap();
        let (mut r, _) = runner::run_gemm(&mut b, &mut ctx, &mut LoadCache::default(), &g).unwrap();
        r.source = "sim".into();
        r.mock = false;
        r.run_id = "2026-10-24T10:00:00Z-simseed".into();
        write_json(&runs_dir(results, "sim").join(run_file_name(&r.run_id)), &r).unwrap();
        r.run_id
    }

    /// The roofline count is over every record on disk, not the `MAX_RUNS` cap,
    /// and only int8 (requant) records with nonzero intensity and MACs/cycle.
    #[test]
    fn roof_points_cover_all_records_not_the_cap() {
        let results = tmp("roof");
        let mut b = Board::new(Box::new(MockTransport::new(MockConfig::default())));
        let mut ctx = Context::new(tmp("roofctx"), &repo_root());
        let g = GemmSpec::named("ffn_up", 1, "FULL", 1).unwrap();
        let (base, _) = runner::run_gemm(&mut b, &mut ctx, &mut LoadCache::default(), &g).unwrap();
        assert!(base.gemm.requant);
        let n_int8 = MAX_RUNS + 7;
        for i in 0..n_int8 + 3 {
            let mut r = base.clone();
            r.source = "sim".into();
            r.mock = false;
            r.run_id = format!("2026-10-24T10:00:00Z-r{i:04}");
            // Two builds, the older first (records sort by run_id).
            r.build_id = if i < 10 { "52a66216".into() } else { "38a3950a".into() };
            if i >= n_int8 {
                r.gemm.requant = false; // int32 bypass: not on the int8 roofline
            }
            write_json(&runs_dir(&results, "sim").join(run_file_name(&r.run_id)), &r).unwrap();
        }
        let live = load_live(&results, "sim");
        assert_eq!(live.total, n_int8 + 3);
        assert_eq!(live.runs.len(), MAX_RUNS);
        assert_eq!(live.roof.len(), n_int8);
        let tcp = TransportSpec::Tcp("127.0.0.1:1".into());
        let a = App::new("sim", &tcp, &results, None, false);
        let sh = Shared {
            app: Arc::new(Mutex::new(a)),
            subs: Arc::new(Mutex::new(Vec::new())),
            cmd: mpsc::channel().0,
            results: results.clone(),
            assets: None,
            lm_index: Arc::new(Mutex::new(LmIndex::new())),
        };
        let s = sh.snapshot();
        assert_eq!(s["roof_measured"]["n"], n_int8);
        assert_eq!(s["roof_measured"]["points"].as_array().unwrap().len(), n_int8);
        assert_eq!(s["roof_measured"]["from"], "results/sim");
        assert_eq!(s["runs_total"], n_int8 + 3);
        assert_eq!(s["runs"].as_array().unwrap().len(), MAX_RUNS);
        // Per-build attribution is over every record (the capped `runs` list no
        // longer contains the 10 older-build records).
        assert_eq!(
            s["builds_all"],
            json!([{"build_id": "52a66216", "n": 10}, {"build_id": "38a3950a", "n": n_int8 + 3 - 10}])
        );
        let roof_builds = build_counts(s["roof_measured"]["points"].as_array().unwrap().iter().map(|p| {
            let mut r = base.clone();
            r.build_id = p["build_id"].as_str().unwrap().into();
            r
        }).collect::<Vec<_>>().iter());
        assert_eq!(roof_builds, vec![("52a66216".to_string(), 10), ("38a3950a".to_string(), n_int8 - 10)]);
        assert!(s["runs"].as_array().unwrap().iter().all(|r| r["build_id"] == "38a3950a"));
    }

    #[test]
    fn auto_presets_resolve_to_the_documented_winners() {
        let v = auto_preset_specs(None).unwrap();
        assert_eq!(v.len(), AUTO_PRESETS.len());
        for (p, g) in AUTO_PRESETS.iter().zip(&v) {
            if let Some((k, n)) = runner::benchmark_kn(p.name) {
                assert_eq!((k, n), (p.k, p.n), "preset {} K/N must be the benchmark's", p.name);
            }
            let r = g.resolve().unwrap();
            let want = if p.case == "strip_wins" { "STRIP" } else { "FULL" };
            assert_eq!(r.mode, want, "{}", p.case);
        }
        assert_eq!(auto_preset_specs(Some("full_wins")).unwrap()[0].m, 22);
        assert!(auto_preset_specs(Some("nope")).is_err());
    }

    /// Auto rows: the chosen mode's cycles are measured; the other mode is shown
    /// measured only when a separate clean run of the same shape exists.
    #[test]
    fn auto_view_labels_other_mode_and_scores_measured_pairs() {
        let mut b = Board::new(Box::new(MockTransport::new(MockConfig::default())));
        let mut ctx = Context::new(tmp("autoctx"), &repo_root());
        ctx.write_files = false;
        let mut cache = LoadCache::default();
        let mut run = |g: GemmSpec| runner::run_gemm(&mut b, &mut ctx, &mut cache, &g).unwrap().0;
        let shape = |mode: &str| GemmSpec { name: "h5_worst".into(), m: 22, k: 250, n: 17, ..GemmSpec::named("ffn_up", 1, mode, 1).unwrap() };
        let full = run(shape("FULL"));
        let strip = run(shape("STRIP"));
        let auto_full = run(shape("AUTO"));
        let auto_strip = run(GemmSpec::named("ffn_up", 1, "AUTO", 1).unwrap());
        let mut bad = strip.clone();
        bad.check.counter_diffs = vec!["cycles_total: 1 vs model 2".into()];
        let v = AutoView::from_records([&full, &bad, &strip, &auto_full, &auto_strip]);
        let j = v.json("results/test");
        assert_eq!(j["rows_total"], 2);
        assert_eq!(j["tally"]["picked_full"], 1);
        assert_eq!(j["tally"]["picked_strip"], 1);
        let r0 = &j["rows"][0];
        assert_eq!(r0["chosen"], "FULL");
        assert_eq!(r0["measured_cycles"], auto_full.counters.cycles_total);
        assert_eq!(r0["other_measured"]["cycles"], strip.counters.cycles_total, "STRIP of this shape was measured");
        assert_eq!(r0["other_measured"]["run_id"], strip.run_id.as_str());
        assert!(j["rows"][1]["other_measured"].is_null(), "ffn_up M=1 FULL never ran: MODEL only");
        // Pairs: one shape measured in both modes; STRIP slower; the model's pick matched.
        assert_eq!(j["pairs"]["n"], 1);
        assert_eq!(j["pairs"]["strip_slower"], 1);
        assert_eq!(j["pairs"]["model_pick_is_measured_faster"], 1);
        assert_eq!(j["pairs"]["worst"]["K"], 250);
        // A record with counter diffs never feeds the measured side.
        let v2 = AutoView::from_records([&full, &bad]);
        assert_eq!(v2.pairs()["n"], 0);
        // Per build: a pair counts for a build only if both modes ran on it.
        let mut full_b = full.clone();
        full_b.build_id = "bbbbbbbb".into();
        let mut strip_a = strip.clone();
        strip_a.build_id = "aaaaaaaa".into();
        let mut full_a = full.clone();
        full_a.build_id = "aaaaaaaa".into();
        let v3 = AutoView::from_records([&full_a, &strip_a, &full_b]);
        let p3 = v3.pairs();
        assert_eq!(p3["n"], 1, "pooled: newest FULL (build b) + STRIP (build a)");
        assert_eq!(p3["builds"].as_array().unwrap().len(), 2);
        assert_eq!(p3["by_build"], json!([{"build_id": "aaaaaaaa", "n": 1, "model_pick_is_measured_faster": 1}]));
    }

    /// Scene 2/3 benchmark points come from every record on disk: a long run of
    /// other shapes never pushes the benchmark sweep out of the capped `runs`.
    #[test]
    fn bench_points_cover_all_records_not_the_cap() {
        let results = tmp("bench");
        let mut b = Board::new(Box::new(MockTransport::new(MockConfig::default())));
        let mut ctx = Context::new(tmp("benchctx"), &repo_root());
        ctx.write_files = false;
        let mut cache = LoadCache::default();
        let mut put = |g: GemmSpec, id: String, build: &str| {
            let (mut r, _) = runner::run_gemm(&mut b, &mut ctx, &mut cache, &g).unwrap();
            r.source = "sim".into();
            r.mock = false;
            r.run_id = id;
            r.build_id = build.into();
            write_json(&runs_dir(&results, "sim").join(run_file_name(&r.run_id)), &r).unwrap();
            r
        };
        // Oldest: ffn_up M=1 in both modes (build A), then FULL again (build B, newer).
        put(GemmSpec::named("ffn_up", 1, "FULL", 1).unwrap(), "2026-10-24T09:00:00Z-a".into(), "aaaaaaaa");
        put(GemmSpec::named("ffn_up", 1, "STRIP", 1).unwrap(), "2026-10-24T09:00:01Z-b".into(), "aaaaaaaa");
        let newer = put(GemmSpec::named("ffn_up", 1, "FULL", 1).unwrap(), "2026-10-24T09:00:02Z-c".into(), "bbbbbbbb");
        // Then more than MAX_RUNS records of a non-benchmark shape.
        let other = GemmSpec { name: "other".into(), m: 3, k: 24, n: 40, ..GemmSpec::named("ffn_up", 1, "FULL", 1).unwrap() };
        let (base, _) = runner::run_gemm(&mut b, &mut ctx, &mut cache, &other).unwrap();
        for i in 0..MAX_RUNS + 5 {
            let mut r = base.clone();
            r.source = "sim".into();
            r.mock = false;
            r.run_id = format!("2026-10-24T10:00:00Z-r{i:04}");
            write_json(&runs_dir(&results, "sim").join(run_file_name(&r.run_id)), &r).unwrap();
        }
        let tcp = TransportSpec::Tcp("127.0.0.1:1".into());
        let a = App::new("sim", &tcp, &results, None, false);
        let sh = Shared {
            app: Arc::new(Mutex::new(a)),
            subs: Arc::new(Mutex::new(Vec::new())),
            cmd: mpsc::channel().0,
            results: results.clone(),
            assets: None,
            lm_index: Arc::new(Mutex::new(LmIndex::new())),
        };
        let s = sh.snapshot();
        assert!(s["runs"].as_array().unwrap().iter().all(|r| r["gemm"]["name"] == "other"), "bench records fell out of the window");
        let br = s["bench"]["runs"].as_array().unwrap();
        assert_eq!(br.len(), 2, "newest per (M, K, N, mode, requant)");
        assert_eq!(br[0]["gemm"]["mode"], "STRIP");
        assert_eq!(br[0]["build_id"], "aaaaaaaa");
        assert_eq!(br[1]["run_id"], newer.run_id.as_str());
        assert_eq!(br[1]["build_id"], "bbbbbbbb");
        assert_eq!(s["bench"]["from"], "results/sim");
        // A new live record replaces the older one of its key.
        let mut v = BenchView::from_records([&newer]);
        let mut n2 = newer.clone();
        n2.run_id = "2026-10-24T11:00:00Z-z".into();
        v.add(&n2);
        assert_eq!(v.json("x")["runs"][0]["run_id"], n2.run_id.as_str());
        let _ = std::fs::remove_dir_all(&results);
    }

    #[test]
    fn transport_labels_before_ping() {
        assert_eq!(transport_source(&TransportSpec::Mock), "model");
        assert_eq!(transport_source(&TransportSpec::Tcp("127.0.0.1:1".into())), "sim");
        assert_eq!(transport_source(&TransportSpec::Serial("COM3".into())), "hw");
    }

    #[test]
    fn mock_build_id_over_tcp_is_model_not_sim() {
        let results = tmp("adopt");
        let sim_id = seed_sim_record(&results);
        let tcp = TransportSpec::Tcp("127.0.0.1:1".into());
        let mut a = App::new(transport_source(&tcp), &tcp, &results, None, false);
        assert_eq!(a.source, "sim");
        assert!(a.runs.iter().any(|r| r.run_id == sim_id));

        a.adopt_board(&results, transport::Source::Sim, runner::MOCK_BUILD_ID_HEX);
        assert_eq!(a.source, "model");
        assert_eq!(a.session.source, "model");
        assert!(a.runs.iter().all(|r| r.source == "model"), "SIM records must not be shown as the mock session");

        // A real co-sim build id on the same tcp: transport is SIM again.
        a.adopt_board(&results, transport::Source::Sim, "38a3950a");
        assert_eq!(a.source, "sim");
        assert!(a.runs.iter().any(|r| r.run_id == sim_id));
        // Serial with a real build id stays HW.
        a.adopt_board(&results, transport::Source::Hw, "38a3950a");
        assert_eq!(a.source, "hw");
    }

    /// End to end: the dashboard worker on tcp: against the mock bridge labels the
    /// session and its records MODEL (mock, results/mock, demo_data, not verified).
    #[test]
    fn worker_over_tcp_mock_bridge_writes_model_records() {
        let results = tmp("worker");
        seed_sim_record(&results);
        let addr = spawn_tcp_bridge(MockConfig::default(), 64);
        let tcp = TransportSpec::Tcp(addr);
        let app = App::new(transport_source(&tcp), &tcp, &results, None, false);
        let (tx, rx) = mpsc::channel();
        let sh = Shared {
            app: Arc::new(Mutex::new(app)),
            subs: Arc::new(Mutex::new(Vec::new())),
            cmd: tx.clone(),
            results: results.clone(),
            assets: None,
            lm_index: Arc::new(Mutex::new(LmIndex::new())),
        };
        {
            let sh2 = sh.clone();
            thread::spawn(move || worker(tcp, transport::BaudSel::Auto, Some(1), repo_root(), sh2, rx));
        }
        tx.send(Cmd::Run(GemmSpec::named("ffn_up", 1, "STRIP", 1).unwrap())).unwrap();
        let t0 = Instant::now();
        loop {
            {
                let a = sh.app.lock().unwrap();
                if a.runs.iter().any(|r| r.gemm.mode == "STRIP") {
                    break;
                }
            }
            assert!(t0.elapsed() < Duration::from_secs(60), "run did not finish");
            thread::sleep(Duration::from_millis(50));
        }
        let snap = sh.snapshot();
        assert_eq!(snap["source"], "model");
        assert_eq!(snap["live_source"], "model");
        assert_eq!(snap["demo_data"], true);
        assert_eq!(snap["mock"], true);
        assert_eq!(snap["board"]["build_id"], runner::MOCK_BUILD_ID_HEX);
        assert_eq!(snap["host_git_hash"], runner::HOST_GIT_HASH);
        let a = sh.app.lock().unwrap();
        let r = a.runs.iter().find(|r| r.gemm.mode == "STRIP").unwrap();
        assert_eq!(r.source, "model");
        assert!(r.mock);
        assert_eq!(a.session.gemms_verified, 0, "mock runs are never counted as verified");
        let mock_files = load_runs(&runs_dir(&results, "model"));
        assert!(mock_files.iter().any(|r| r.run_id == a.runs.back().unwrap().run_id && r.mock));
        let sim_files = load_runs(&runs_dir(&results, "sim"));
        assert!(sim_files.iter().all(|r| r.run_id.ends_with("-simseed")), "nothing new written under results/sim");
    }
}
