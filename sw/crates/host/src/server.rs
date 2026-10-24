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
use crate::record::{iso_utc, now_unix, runs_dir, session_path, write_json, RunRecord, Session};
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

/// Keep this many recent runs in the snapshot.
const MAX_RUNS: usize = 400;

pub struct ServeOpts {
    pub port: u16,
    pub transport: TransportSpec,
    pub baud: u32,
    pub timeout_scale: Option<u32>,
    pub results: PathBuf,
    pub root: PathBuf,
    /// Serve demo assets from this directory instead of the embedded copies (dev).
    pub assets: Option<PathBuf>,
    /// Directory of recorded run JSONs to offer as REPLAY.
    pub replay: Option<PathBuf>,
    pub attract: bool,
}

#[derive(Debug)]
enum Cmd {
    Run(GemmSpec),
    Demo,
    Selftest,
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
}

type Subs = Arc<Mutex<Vec<Sender<String>>>>;

#[derive(Clone)]
struct Shared {
    app: Arc<Mutex<App>>,
    subs: Subs,
    cmd: Sender<Cmd>,
    results: PathBuf,
    assets: Option<PathBuf>,
}

fn broadcast(subs: &Subs, event: &str, data: &Value) {
    let msg = format!("event: {event}\ndata: {data}\n\n");
    let mut s = subs.lock().expect("subs");
    s.retain(|tx| tx.send(msg.clone()).is_ok());
}

fn read_json(p: &Path) -> Option<Value> {
    std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// Load run records from a directory (sorted by run_id), skipping non-records.
pub fn load_runs(dir: &Path) -> Vec<RunRecord> {
    let mut v: Vec<RunRecord> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .filter_map(|p| read_json(&p))
                .filter(|v| crate::record::validate_run_json(v).is_ok())
                .filter_map(|v| serde_json::from_value::<RunRecord>(v).ok())
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| a.run_id.cmp(&b.run_id));
    v
}

impl Shared {
    fn snapshot(&self) -> Value {
        let a = self.app.lock().expect("app");
        let model_dir = self.results.join("model");
        let cost = read_json(&self.results.join("cost.json"));
        let (runs, session, source, board_status) = match (&a.replay, a.replay_active) {
            (Some(r), true) => (
                serde_json::to_value(&r.runs).unwrap_or(Value::Null),
                r.session.clone().unwrap_or(Value::Null),
                "replay".to_string(),
                "replay",
            ),
            _ => (
                serde_json::to_value(&a.runs).unwrap_or(Value::Null),
                serde_json::to_value(&a.session).unwrap_or(Value::Null),
                a.source.clone(),
                a.board_status,
            ),
        };
        json!({
            "now_utc": iso_utc(now_unix()),
            "started_utc": a.started_utc,
            "source": source,
            "live_source": a.source,
            "demo_data": source == "model",
            "transport": a.transport,
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
        })
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

fn worker(opts_t: TransportSpec, baud: u32, scale: Option<u32>, root: PathBuf, sh: Shared, rx: Receiver<Cmd>) {
    let mut ctx = Context::new(sh.results.clone(), &root);
    let mut board: Option<Board> = None;
    let mut cache = LoadCache::default();
    let mut last_ping = Instant::now() - Duration::from_secs(60);
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
    loop {
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
            match transport::open(&opts_t, baud, scale, MockConfig::default()) {
                Ok(t) => {
                    let mut b = Board::new(t);
                    match b.ping() {
                        Ok(p) => {
                            let id = format!("{:08x}", p.build_id);
                            ctx.build_id = Some(id.clone());
                            cache.clear();
                            {
                                let mut a = sh.app.lock().expect("app");
                                a.build_id = Some(id);
                                a.proto_version = Some(p.version);
                                a.transport = b.describe();
                            }
                            set_status(&sh, "live", None, Some(&b));
                            board = Some(b);
                        }
                        Err(e) => set_status(&sh, "disconnected", Some(e.to_string()), None),
                    }
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
        let specs: Vec<GemmSpec> = match cmd {
            Cmd::Run(g) => vec![g],
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
        };
        let is_demo = specs.len() > 1;
        let n_specs = specs.len();
        for (i, g) in specs.into_iter().enumerate() {
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
                        a.session.add(&rec);
                        let sp = session_path(&sh.results, &a.source);
                        let _ = write_json(&sp, &a.session);
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
            if is_demo && i + 1 < n_specs {
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
            (a.attract && !a.replay_active, a.demo_running || a.running.is_some() || a.queue_len > 0)
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
            // Named presets only: a SPEC §1.1 benchmark, an M from the sweep, a mode.
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
            if !(1..=3).contains(&sc) {
                return json_resp(&mut s, 400, &json!({"error": "scene must be 1..3"}));
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
        _ => respond(&mut s, 404, "text/plain", b"not found"),
    }
}

/// Run the dashboard server (blocks forever).
pub fn serve(o: ServeOpts) -> Result<(), String> {
    let addr = format!("127.0.0.1:{}", o.port);
    let listener = TcpListener::bind(&addr).map_err(|e| format!("cannot bind {addr}: {e}"))?;
    let source = match o.transport {
        TransportSpec::Mock => "model",
        TransportSpec::Tcp(_) => "sim",
        TransportSpec::Serial(_) => "hw",
    };
    let session = Session::load_or_new(&session_path(&o.results, source), source);
    let mut runs: VecDeque<RunRecord> = load_runs(&runs_dir(&o.results, source)).into();
    while runs.len() > MAX_RUNS {
        runs.pop_front();
    }
    let replay = o.replay.as_ref().map(|d| {
        let runs = load_runs(d);
        Replay {
            dir: d.display().to_string(),
            recorded_utc: runs.last().map(|r| r.run_id.chars().take(20).collect()).unwrap_or_default(),
            session: read_json(&d.join("session.json")).or_else(|| read_json(&d.join("../session.json"))),
            runs,
        }
    });
    let replay_active = replay.is_some();
    let app = App {
        source: source.into(),
        transport: format!("{:?}", o.transport),
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
        attract: o.attract,
        demo_running: false,
        highlight: vec![],
        replay,
        replay_active,
        last_selftest: None,
        started_utc: iso_utc(now_unix()),
    };
    let (tx, rx) = mpsc::channel();
    let sh = Shared {
        app: Arc::new(Mutex::new(app)),
        subs: Arc::new(Mutex::new(Vec::new())),
        cmd: tx,
        results: o.results.clone(),
        assets: o.assets.clone(),
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
