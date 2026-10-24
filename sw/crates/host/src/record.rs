//! Run records (DEMO.md §4 schema), session totals, and result paths.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use splitsys_cycle::Counters;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GemmInfo {
    pub name: String,
    #[serde(rename = "M")]
    pub m: u64,
    #[serde(rename = "K")]
    pub k: u64,
    #[serde(rename = "N")]
    pub n: u64,
    /// "FULL" | "STRIP"
    pub mode: String,
    pub requant: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelInfo {
    #[serde(flatten)]
    pub counters: Counters,
    pub model_git_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Derived {
    /// mac_useful / (64 · cycles_total), measured counters.
    pub util: f64,
    /// Same from the model counters.
    pub util_model: f64,
    /// A + W bytes read per STREAM cycle by this mode's datapath (16 FULL, 20 STRIP).
    pub operand_bytes_per_cycle: f64,
    /// (measured − model) / model for cycles_total (a fraction; ×100 for %).
    pub model_err_total: f64,
    /// Operand bytes streamed (tiles·K·bytes_per_cycle) / cycles_total, measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operand_bytes_per_cycle_avg: Option<f64>,
    /// mac_useful / cycles_total, measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macs_per_cycle: Option<f64>,
    /// mac_useful / operand bytes streamed, measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arithmetic_intensity: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Check {
    pub golden_match: bool,
    pub outputs_compared: u64,
    pub mismatches: u64,
    pub invariants_ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invariant_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Row-major index (m*N + n) of the first mismatching in-bounds output.
    pub first_mismatch: Option<u64>,
}

/// Extra provenance (not in the DEMO.md §4 example, all optional).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct Provenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// End-to-end wall time including UART. Never an accelerator speed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_ms_end_to_end: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latencies_provisional: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RunRecord {
    pub run_id: String,
    pub source: String,
    pub build_id: String,
    pub git_hash: String,
    pub clock_mhz: u64,
    pub gemm: GemmInfo,
    pub counters: Counters,
    pub model: ModelInfo,
    pub derived: Derived,
    pub check: Check,
    #[serde(default, skip_serializing_if = "is_default_prov")]
    pub provenance: Provenance,
}

fn is_default_prov(p: &Provenance) -> bool {
    *p == Provenance::default()
}

impl RunRecord {
    pub fn verified(&self) -> bool {
        self.check.golden_match && self.check.invariants_ok && self.check.mismatches == 0
    }
}

/// Validate a JSON value against the DEMO.md §4 contract (keys and types).
/// Extra keys are allowed; missing or mistyped required keys are errors.
pub fn validate_run_json(v: &Value) -> Result<(), String> {
    fn need<'a>(v: &'a Value, path: &str, key: &str) -> Result<&'a Value, String> {
        v.get(key).ok_or_else(|| format!("missing {path}{key}"))
    }
    fn is_u(v: &Value) -> bool {
        v.as_u64().is_some()
    }
    let s = |k: &str| -> Result<(), String> {
        need(v, "", k)?
            .as_str()
            .map(|_| ())
            .ok_or_else(|| format!("{k} must be a string"))
    };
    for k in ["run_id", "source", "build_id", "git_hash"] {
        s(k)?;
    }
    let src = v["source"].as_str().unwrap_or("");
    if !["hw", "sim", "model"].contains(&src) {
        return Err(format!("source '{src}' not in hw|sim|model"));
    }
    if !need(v, "", "clock_mhz")?.is_number() {
        return Err("clock_mhz must be a number".into());
    }
    let g = need(v, "", "gemm")?;
    need(g, "gemm.", "name")?
        .as_str()
        .ok_or("gemm.name must be a string")?;
    for k in ["M", "K", "N"] {
        if !is_u(need(g, "gemm.", k)?) {
            return Err(format!("gemm.{k} must be an unsigned integer"));
        }
    }
    let mode = need(g, "gemm.", "mode")?.as_str().unwrap_or("");
    if mode != "FULL" && mode != "STRIP" {
        return Err(format!("gemm.mode '{mode}' not FULL|STRIP"));
    }
    if !need(g, "gemm.", "requant")?.is_boolean() {
        return Err("gemm.requant must be a bool".into());
    }
    for blk in ["counters", "model"] {
        let c = need(v, "", blk)?;
        for k in [
            "cycles_total",
            "cycles_setup",
            "cycles_stream",
            "cycles_flush",
            "cycles_drain",
            "cycles_stall",
            "mac_useful",
            "tiles",
        ] {
            if !is_u(need(c, &format!("{blk}."), k)?) {
                return Err(format!("{blk}.{k} must be an unsigned integer"));
            }
        }
        let q = need(c, &format!("{blk}."), "mac_useful_q")?
            .as_array()
            .ok_or(format!("{blk}.mac_useful_q must be an array"))?;
        if q.len() != 4 || !q.iter().all(is_u) {
            return Err(format!("{blk}.mac_useful_q must be 4 unsigned integers"));
        }
    }
    need(&v["model"], "model.", "model_git_hash")?
        .as_str()
        .ok_or("model.model_git_hash must be a string")?;
    let d = need(v, "", "derived")?;
    for k in ["util", "util_model", "operand_bytes_per_cycle", "model_err_total"] {
        if !need(d, "derived.", k)?.is_number() {
            return Err(format!("derived.{k} must be a number"));
        }
    }
    let c = need(v, "", "check")?;
    for k in ["golden_match", "invariants_ok"] {
        if !need(c, "check.", k)?.is_boolean() {
            return Err(format!("check.{k} must be a bool"));
        }
    }
    for k in ["outputs_compared", "mismatches"] {
        if !is_u(need(c, "check.", k)?) {
            return Err(format!("check.{k} must be an unsigned integer"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Time and paths
// ---------------------------------------------------------------------------

/// Seconds since the Unix epoch.
pub fn now_unix() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// `YYYY-MM-DDTHH:MM:SSZ` (UTC) for a Unix time.
pub fn iso_utc(unix: f64) -> String {
    let secs = unix.max(0.0) as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Find the repo root (directory holding `spec/spec.toml`) from the current
/// directory upwards, else from the compile-time crate location.
pub fn repo_root() -> PathBuf {
    if let Ok(mut d) = std::env::current_dir() {
        loop {
            if d.join("spec").join("spec.toml").is_file() {
                return d;
            }
            if !d.pop() {
                break;
            }
        }
    }
    let compiled = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    if compiled.join("spec").join("spec.toml").is_file() {
        return compiled;
    }
    PathBuf::from(".")
}

/// Directory for run records of a source. Mock (model) runs go to
/// `results/model/mock/` so they can never be mistaken for hardware results.
pub fn runs_dir(results: &Path, source: &str) -> PathBuf {
    match source {
        "model" => results.join("model").join("mock"),
        s => results.join(s),
    }
}

/// Session-totals file per source (hw uses `results/session.json`, DEMO.md §4).
pub fn session_path(results: &Path, source: &str) -> PathBuf {
    match source {
        "hw" => results.join("session.json"),
        s => runs_dir(results, s).join("session.json"),
    }
}

/// File name for a run id (colons are not legal in Windows file names).
pub fn run_file_name(run_id: &str) -> String {
    format!("{}.json", run_id.replace(':', "-"))
}

/// Write JSON atomically (temp file + rename).
pub fn write_json(path: &Path, v: &impl Serialize) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v)?)?;
    std::fs::rename(&tmp, path)
}

/// Running totals (DEMO.md §4 `results/session.json`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct Session {
    pub source: String,
    pub gemms_run: u64,
    pub gemms_verified: u64,
    pub macs_verified: u64,
    pub mismatches: u64,
    pub failed_runs: u64,
    pub started_utc: String,
    pub updated_utc: String,
    /// Seconds since `started_utc` at the last update.
    pub uptime_s: u64,
    #[serde(default)]
    pub started_unix: f64,
    #[serde(default)]
    pub build_ids: Vec<String>,
}

impl Session {
    pub fn load_or_new(path: &Path, source: &str) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Session>(&b).ok())
            .filter(|s| s.source == source)
            .unwrap_or_else(|| {
                let now = now_unix();
                Session {
                    source: source.into(),
                    started_utc: iso_utc(now),
                    updated_utc: iso_utc(now),
                    started_unix: now,
                    ..Default::default()
                }
            })
    }

    pub fn add(&mut self, r: &RunRecord) {
        self.gemms_run += 1;
        if r.verified() {
            self.gemms_verified += 1;
            self.macs_verified += r.counters.mac_useful;
        } else {
            self.failed_runs += 1;
        }
        self.mismatches += r.check.mismatches;
        if !self.build_ids.contains(&r.build_id) {
            self.build_ids.push(r.build_id.clone());
        }
        let now = now_unix();
        self.updated_utc = iso_utc(now);
        self.uptime_s = (now - self.started_unix).max(0.0) as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The JSON example of DEMO.md §4, extracted from the doc itself so the test
    /// tracks the contract.
    fn demo_example() -> Value {
        let md = std::fs::read_to_string(repo_root().join("docs/DEMO.md")).expect("docs/DEMO.md");
        let start = md.find("## 4. Data contract").expect("§4");
        let body = &md[start..];
        let j0 = body.find("```json").expect("json block") + "```json".len();
        let j1 = j0 + body[j0..].find("```").expect("end of block");
        serde_json::from_str(&body[j0..j1]).expect("DEMO.md §4 example parses")
    }

    #[test]
    fn demo_md_example_validates_and_roundtrips() {
        let ex = demo_example();
        validate_run_json(&ex).unwrap();
        let rec: RunRecord = serde_json::from_value(ex.clone()).expect("deserialize §4 example");
        let back = serde_json::to_value(&rec).unwrap();
        validate_run_json(&back).unwrap();
        // Every key of the example survives the round trip with the same value
        // (numbers compared as f64: the example writes 0.0 for derived floats).
        fn same(a: &Value, b: &Value, path: &str) {
            match (a, b) {
                (Value::Object(x), Value::Object(y)) => {
                    for (k, v) in x {
                        same(v, y.get(k).unwrap_or_else(|| panic!("lost key {path}.{k}")), &format!("{path}.{k}"));
                    }
                }
                (Value::Number(x), Value::Number(y)) => {
                    assert_eq!(x.as_f64(), y.as_f64(), "{path}")
                }
                _ => assert_eq!(a, b, "{path}"),
            }
        }
        same(&ex, &back, "");
    }

    #[test]
    fn validator_rejects_broken_records() {
        let ex = demo_example();
        let mut v = ex.clone();
        v["source"] = "live".into();
        assert!(validate_run_json(&v).is_err());
        let mut v = ex.clone();
        v["counters"]["mac_useful_q"] = serde_json::json!([0, 0, 0]);
        assert!(validate_run_json(&v).is_err());
        let mut v = ex.clone();
        v["check"].as_object_mut().unwrap().remove("mismatches");
        assert!(validate_run_json(&v).is_err());
        let mut v = ex;
        v["gemm"]["mode"] = "INDEP".into();
        assert!(validate_run_json(&v).is_err());
    }

    #[test]
    fn iso_utc_known_values() {
        assert_eq!(iso_utc(0.0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_792_854_131.0), "2026-10-24T15:02:11Z");
        assert_eq!(iso_utc(951_782_400.0), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn paths_never_mix_sources() {
        let r = Path::new("results");
        assert_eq!(runs_dir(r, "model"), Path::new("results/model/mock"));
        assert_eq!(runs_dir(r, "hw"), Path::new("results/hw"));
        assert_eq!(session_path(r, "hw"), Path::new("results/session.json"));
        assert_eq!(session_path(r, "model"), Path::new("results/model/mock/session.json"));
        assert_eq!(run_file_name("2026-10-24T15:02:11Z-x"), "2026-10-24T15-02-11Z-x.json");
    }
}
