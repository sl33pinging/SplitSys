//! # splitsys_cycle — the SplitSys cycle model
//!
//! Predicts **every** hardware performance counter of SPEC §3.7 for a GEMM
//! descriptor `(mode, flags, M, K, N)`, using the timing model of SPEC §4.
//!
//! The model is derived twice, independently, and the two must agree:
//!
//! 1. [`phase_walk`] steps the sequencer FSM of SPEC §3.5 cycle by cycle
//!    (`ACCEPT → TILE_SETUP → STREAM → FLUSH → DRAIN → WRITEBACK → … → DONE`),
//!    incrementing the counters exactly the way `rtl/counters.sv` does, and
//!    derives `mac_useful` / `mac_useful_q` by walking the physical PE ↔ (m, n)
//!    mapping of SPEC §3.2 with in-bounds masks.
//! 2. [`closed_form`] evaluates the SPEC §4 formula and counts per-quadrant
//!    MACs arithmetically.
//!
//! Every latency comes from a [`Latencies`] value. [`Latencies::default`]
//! reads the `L_*` constants of `splitsys-spec` (generated from
//! `spec/spec.toml`, all currently **provisional**); calibration passes an
//! explicit [`Latencies`] instead. Nothing in this crate hard-codes a latency.
//!
//! ## Phase → counter mapping (per tile)
//!
//! | counter         | per tile                                         |
//! |-----------------|--------------------------------------------------|
//! | `cycles_setup`  | `L_SETUP`                                        |
//! | `cycles_stream` | `L_RD + K`                                       |
//! | `cycles_flush`  | `(R-1) + (C-1) + L_PE`  (R = C = 8 in both modes)|
//! | `cycles_drain`  | `T_DRAIN(mode, flags) + L_RQ + L_WB` (DRAIN + WRITEBACK) |
//! | `cycles_stall`  | 0 (MVP: no hazards)                              |
//!
//! `cycles_total = L_ACCEPT + Σ_tiles T_tile + L_DONE`.
//!
//! `T_DRAIN` = number of output rows of the tile (8 FULL, 4 STRIP: one row
//! per cycle, SPEC §3.5), ×4 when the int32 bypass output path is selected
//! (four 128-bit writes per row, SPEC §3.6).

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::fmt;

pub mod report;

// ---------------------------------------------------------------------------
// Spec adapter: the only place that touches `splitsys_spec` names.
// ---------------------------------------------------------------------------

/// Constants imported from `splitsys-spec`, normalised to `u64`.
pub mod spec {
    use splitsys_spec as s;
    /// Physical array rows (R).
    pub const ROWS: u64 = s::ROWS as u64;
    /// Physical array columns (C).
    pub const COLS: u64 = s::COLS as u64;
    /// Quadrant edge (4).
    pub const QUAD: u64 = s::QUAD as u64;
    /// Peak MACs per cycle = number of PEs.
    pub const PEAK_MACS_PER_CYCLE: u64 = s::N_PES as u64;
    pub const MODE_FULL: u8 = s::MODE_FULL;
    pub const MODE_STRIP: u8 = s::MODE_STRIP;
    pub const FLAG_REQUANT_EN: u8 = s::FLAG_REQUANT_EN;
    pub const FLAG_OUT_INT32: u8 = s::FLAG_OUT_INT32;
    pub const FULL_TILE_M: u64 = s::FULL_TILE_M as u64;
    pub const FULL_TILE_N: u64 = s::FULL_TILE_N as u64;
    pub const STRIP_TILE_M: u64 = s::STRIP_TILE_M as u64;
    pub const STRIP_TILE_N: u64 = s::STRIP_TILE_N as u64;
    pub const T_DRAIN_FULL: u64 = s::T_DRAIN_FULL;
    pub const T_DRAIN_STRIP: u64 = s::T_DRAIN_STRIP;
    pub const INT32_DRAIN_FACTOR: u64 = s::INT32_DRAIN_FACTOR;
    pub const M_MAX: u64 = s::M_MAX as u64;
    pub const K_MAX: u64 = s::K_MAX as u64;
    pub const N_MAX: u64 = s::N_MAX as u64;
    pub const L_PE: u64 = s::L_PE;
    pub const L_RD: u64 = s::L_RD;
    pub const L_SETUP: u64 = s::L_SETUP;
    pub const L_RQ: u64 = s::L_RQ;
    pub const L_WB: u64 = s::L_WB;
    pub const L_ACCEPT: u64 = s::L_ACCEPT;
    pub const L_DONE: u64 = s::L_DONE;
    /// True while any latency in spec.toml is still marked `provisional`.
    pub const LATENCIES_PROVISIONAL: bool = s::L_PE_PROVISIONAL
        || s::L_RD_PROVISIONAL
        || s::L_SETUP_PROVISIONAL
        || s::L_RQ_PROVISIONAL
        || s::L_WB_PROVISIONAL
        || s::L_ACCEPT_PROVISIONAL
        || s::L_DONE_PROVISIONAL;
    /// READ_COUNTERS order.
    pub const COUNTER_NAMES: [&str; 12] = s::COUNTER_NAMES;
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

/// Array mode (SPEC §3.2). INDEP is reserved and not modeled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Mode {
    #[serde(rename = "FULL")]
    Full,
    #[serde(rename = "STRIP")]
    Strip,
}

impl Mode {
    pub const ALL: [Mode; 2] = [Mode::Full, Mode::Strip];

    /// Descriptor encoding (SPEC §3.5 offset 0).
    pub fn code(self) -> u8 {
        match self {
            Mode::Full => spec::MODE_FULL,
            Mode::Strip => spec::MODE_STRIP,
        }
    }

    pub fn from_code(code: u8) -> Option<Mode> {
        if code == spec::MODE_FULL {
            Some(Mode::Full)
        } else if code == spec::MODE_STRIP {
            Some(Mode::Strip)
        } else {
            None
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Full => "FULL",
            Mode::Strip => "STRIP",
        }
    }

    /// Output-tile rows: FULL 8 (all array rows), STRIP 4 (one half).
    pub fn tile_rows(self) -> u64 {
        match self {
            Mode::Full => spec::FULL_TILE_M,
            Mode::Strip => spec::STRIP_TILE_M,
        }
    }

    /// Output-tile columns: FULL 8, STRIP 16 (two 4×8 slabs side by side).
    pub fn tile_cols(self) -> u64 {
        match self {
            Mode::Full => spec::FULL_TILE_N,
            Mode::Strip => spec::STRIP_TILE_N,
        }
    }

    /// Activation bytes read per STREAM cycle (one int8 per used A lane).
    pub fn a_bytes_per_cycle(self) -> u64 {
        self.tile_rows()
    }

    /// Weight bytes read per STREAM cycle (one int8 per used W lane).
    pub fn w_bytes_per_cycle(self) -> u64 {
        self.tile_cols()
    }

    /// Operand bytes per STREAM cycle: FULL 8 A + 8 W = 16, STRIP 4 A + 16 W = 20 (SPEC §4).
    pub fn operand_bytes_per_cycle(self) -> u64 {
        self.a_bytes_per_cycle() + self.w_bytes_per_cycle()
    }

    /// Map physical PE (r, c) to the tile-local output element (i, j) it
    /// accumulates (SPEC §3.2). FULL: identity. STRIP: PE(i, c) holds
    /// C[i, c] and PE(4+i, c) holds C[i, 8+c].
    pub fn pe_to_output(self, r: u64, c: u64) -> (u64, u64) {
        match self {
            Mode::Full => (r, c),
            Mode::Strip => {
                let half = spec::ROWS / 2;
                if r < half {
                    (r, c)
                } else {
                    (r - half, spec::COLS + c)
                }
            }
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Quadrant index of physical PE (r, c): Q0 rows 0–3 × cols 0–3, Q1 rows
/// 0–3 × cols 4–7, Q2 rows 4–7 × cols 0–3, Q3 rows 4–7 × cols 4–7.
pub fn quadrant_of(r: u64, c: u64) -> usize {
    let qr = usize::from(r >= spec::QUAD);
    let qc = usize::from(c >= spec::QUAD);
    2 * qr + qc
}

/// Whether the descriptor selects the int32 bypass output path.
///
/// SPEC §3.5 defines `flags` bit1 `out_int32`; SPEC §3.6 counts the bypass's
/// four writes per row as `cycles_drain`. This model keys the slow path on
/// `FLAG_OUT_INT32`. (SPEC §1.2 phrases bypass as `requant_en = 0`; see the
/// open question raised with spec-steward.)
pub fn is_int32_out(flags: u8) -> bool {
    flags & spec::FLAG_OUT_INT32 != 0
}

/// A GEMM descriptor, as far as timing is concerned (SPEC §3.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Descriptor {
    pub mode: Mode,
    pub flags: u8,
    #[serde(rename = "M")]
    pub m: u64,
    #[serde(rename = "K")]
    pub k: u64,
    #[serde(rename = "N")]
    pub n: u64,
}

/// Why a descriptor cannot be modeled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DescError {
    OutOfRange {
        field: &'static str,
        value: u64,
        max: u64,
    },
}

impl fmt::Display for DescError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DescError::OutOfRange { field, value, max } => {
                write!(f, "{field} = {value} out of range 1..={max} (SPEC §3.5)")
            }
        }
    }
}

impl std::error::Error for DescError {}

impl Descriptor {
    pub fn new(mode: Mode, flags: u8, m: u64, k: u64, n: u64) -> Self {
        Descriptor {
            mode,
            flags,
            m,
            k,
            n,
        }
    }

    /// Range check against the spec limits (the RTL NACKs out-of-range descriptors).
    pub fn validate(&self) -> Result<(), DescError> {
        for (field, value, max) in [
            ("M", self.m, spec::M_MAX),
            ("K", self.k, spec::K_MAX),
            ("N", self.n, spec::N_MAX),
        ] {
            if value == 0 || value > max {
                return Err(DescError::OutOfRange { field, value, max });
            }
        }
        Ok(())
    }

    pub fn int32_out(&self) -> bool {
        is_int32_out(self.flags)
    }

    /// Number of row blocks (inner loop).
    pub fn row_blocks(&self) -> u64 {
        self.m.div_ceil(self.mode.tile_rows())
    }

    /// Number of column blocks (outer loop).
    pub fn col_blocks(&self) -> u64 {
        self.n.div_ceil(self.mode.tile_cols())
    }
}

/// Every latency the model uses. Defaults come from `splitsys-spec`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Latencies {
    #[serde(rename = "L_PE")]
    pub l_pe: u64,
    #[serde(rename = "L_RD")]
    pub l_rd: u64,
    #[serde(rename = "L_SETUP")]
    pub l_setup: u64,
    #[serde(rename = "L_RQ")]
    pub l_rq: u64,
    #[serde(rename = "L_WB")]
    pub l_wb: u64,
    #[serde(rename = "L_ACCEPT")]
    pub l_accept: u64,
    #[serde(rename = "L_DONE")]
    pub l_done: u64,
}

impl Default for Latencies {
    fn default() -> Self {
        Latencies {
            l_pe: spec::L_PE,
            l_rd: spec::L_RD,
            l_setup: spec::L_SETUP,
            l_rq: spec::L_RQ,
            l_wb: spec::L_WB,
            l_accept: spec::L_ACCEPT,
            l_done: spec::L_DONE,
        }
    }
}

impl Latencies {
    /// Per-tile phase lengths `(setup, stream, flush, drain, writeback)`.
    pub fn phase_lengths(&self, d: &Descriptor) -> [u64; 5] {
        let setup = self.l_setup;
        let stream = self.l_rd + d.k;
        let flush = (spec::ROWS - 1) + (spec::COLS - 1) + self.l_pe;
        let drain = t_drain(d.mode, d.flags);
        let writeback = self.l_rq + self.l_wb;
        [setup, stream, flush, drain, writeback]
    }
}

/// `T_DRAIN(mode, flags)`: one cycle per drained output row (8 FULL, 4
/// STRIP), ×4 for the int32 bypass path (SPEC §3.5, §3.6, §4).
pub fn t_drain(mode: Mode, flags: u8) -> u64 {
    let base = match mode {
        Mode::Full => spec::T_DRAIN_FULL,
        Mode::Strip => spec::T_DRAIN_STRIP,
    };
    if is_int32_out(flags) {
        base * spec::INT32_DRAIN_FACTOR
    } else {
        base
    }
}

// ---------------------------------------------------------------------------
// Outputs
// ---------------------------------------------------------------------------

/// The SPEC §3.7 counter set, in the DEMO.md §4 field order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counters {
    pub cycles_total: u64,
    pub cycles_setup: u64,
    pub cycles_stream: u64,
    pub cycles_flush: u64,
    pub cycles_drain: u64,
    pub cycles_stall: u64,
    pub mac_useful: u64,
    pub mac_useful_q: [u64; 4],
    pub tiles: u64,
}

impl Counters {
    /// Values in `READ_COUNTERS` readback order (cycles_total, cycles_setup,
    /// cycles_stream, cycles_flush, cycles_drain, cycles_stall, mac_useful,
    /// mac_useful_q0..q3, tiles), paired with their spec names.
    pub fn readback(&self) -> Vec<(&'static str, u64)> {
        let vals = [
            self.cycles_total,
            self.cycles_setup,
            self.cycles_stream,
            self.cycles_flush,
            self.cycles_drain,
            self.cycles_stall,
            self.mac_useful,
            self.mac_useful_q[0],
            self.mac_useful_q[1],
            self.mac_useful_q[2],
            self.mac_useful_q[3],
            self.tiles,
        ];
        spec::COUNTER_NAMES.iter().copied().zip(vals).collect()
    }

    /// Check the three SPEC §3.7 invariants. Returns a description of the
    /// first violated invariant.
    pub fn check_invariants(&self, d: &Descriptor, lat: &Latencies) -> Result<(), String> {
        let mnk = d.m * d.n * d.k;
        if self.mac_useful != mnk {
            return Err(format!("mac_useful {} != M*N*K {}", self.mac_useful, mnk));
        }
        let qsum: u64 = self.mac_useful_q.iter().sum();
        if qsum != self.mac_useful {
            return Err(format!(
                "sum(mac_useful_q) {} != mac_useful {}",
                qsum, self.mac_useful
            ));
        }
        let phases = self.cycles_setup
            + self.cycles_stream
            + self.cycles_flush
            + self.cycles_drain
            + self.cycles_stall
            + lat.l_accept
            + lat.l_done;
        if self.cycles_total != phases {
            return Err(format!(
                "cycles_total {} != phases + L_ACCEPT + L_DONE {}",
                self.cycles_total, phases
            ));
        }
        Ok(())
    }

    /// Utilization = mac_useful / (64 · cycles_total) (SPEC §3.7).
    pub fn utilization(&self) -> f64 {
        if self.cycles_total == 0 {
            return 0.0;
        }
        self.mac_useful as f64 / (spec::PEAK_MACS_PER_CYCLE * self.cycles_total) as f64
    }

    /// Achieved MACs per cycle over the whole job.
    pub fn macs_per_cycle(&self) -> f64 {
        if self.cycles_total == 0 {
            return 0.0;
        }
        self.mac_useful as f64 / self.cycles_total as f64
    }
}

/// FSM state of the sequencer as the counters see it (SPEC §3.5, §3.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Phase {
    /// Descriptor accept → first TILE_SETUP (counted only in cycles_total).
    Accept,
    TileSetup,
    Stream,
    Flush,
    Drain,
    Writeback,
    /// Last WRITEBACK → DONE (counted only in cycles_total).
    Done,
}

/// One contiguous FSM phase of the walk, for the dashboard's wavefront
/// animation (label it "model").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhaseEvent {
    /// Tile index in issue order, or `None` for ACCEPT / DONE.
    pub tile: Option<u64>,
    /// Row block of the tile (inner loop).
    pub mb: Option<u64>,
    /// Column block of the tile (outer loop).
    pub nb: Option<u64>,
    pub phase: Phase,
    /// First cycle of the phase, counting from descriptor accept = 0.
    pub start: u64,
    /// Length in cycles.
    pub len: u64,
}

// ---------------------------------------------------------------------------
// Derivation 1: cycle-stepped FSM walk
// ---------------------------------------------------------------------------

/// Result of [`walk`]: the counters plus the phase timeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Walk {
    pub counters: Counters,
    pub timeline: Vec<PhaseEvent>,
}

/// Counters from the cycle-stepped FSM walk. See [`walk`].
pub fn phase_walk(d: &Descriptor, lat: &Latencies) -> Counters {
    walk(d, lat, false).counters
}

/// Per-phase timeline from the FSM walk (model-derived; used for the PE-grid
/// animation). Within a tile, PE(r, c) sees operand k at cycle
/// `stream.start + L_RD + k + r + c`.
pub fn timeline(d: &Descriptor, lat: &Latencies) -> Vec<PhaseEvent> {
    walk(d, lat, true).timeline
}

/// Walk the sequencer FSM one cycle at a time.
///
/// Each simulated cycle increments `cycles_total` and the counter of the
/// current phase, as `rtl/counters.sv` does. MACs are attributed by walking
/// every physical PE of every tile: PE(r, c) is mapped to its output element
/// via [`Mode::pe_to_output`], masked by `m < M && n < N`, and performs K
/// MACs whose operands arrive at cycles `stream_start + L_RD + r + c + k`.
/// The walk asserts that each PE's last accumulator update lands before the
/// tile's DRAIN begins (i.e. the FLUSH length is sufficient).
///
/// # Panics
/// If the descriptor is out of range, or if a MAC would land after FLUSH.
pub fn walk(d: &Descriptor, lat: &Latencies, record: bool) -> Walk {
    d.validate().expect("descriptor out of range");
    let mut w = Walker {
        ctr: Counters::default(),
        timeline: Vec::new(),
        cycle: 0,
        record,
    };

    w.step(Phase::Accept, lat.l_accept, None);

    let [setup, stream, flush, drain, writeback] = lat.phase_lengths(d);
    let (tr, tc) = (d.mode.tile_rows(), d.mode.tile_cols());
    let mut tile_idx = 0u64;
    // Tile order: column-block outer, row-block inner (SPEC §3.5).
    for nb in 0..d.col_blocks() {
        for mb in 0..d.row_blocks() {
            let t = Some((tile_idx, mb, nb));
            w.step(Phase::TileSetup, setup, t);

            let stream_start = w.cycle;
            w.step(Phase::Stream, stream, t);
            w.step(Phase::Flush, flush, t);
            let drain_start = w.cycle;

            // MAC accounting over the physical PE grid.
            for r in 0..spec::ROWS {
                for c in 0..spec::COLS {
                    let (i, j) = d.mode.pe_to_output(r, c);
                    debug_assert!(i < tr && j < tc);
                    let m = mb * tr + i;
                    let n = nb * tc + j;
                    if m < d.m && n < d.n {
                        let first_arrival = stream_start + lat.l_rd + r + c;
                        let last_update = first_arrival + (d.k - 1) + lat.l_pe;
                        assert!(
                            last_update < drain_start,
                            "PE({r},{c}) tile {tile_idx}: last MAC lands at cycle {last_update}, \
                             DRAIN starts at {drain_start}"
                        );
                        w.ctr.mac_useful += d.k;
                        w.ctr.mac_useful_q[quadrant_of(r, c)] += d.k;
                    }
                }
            }

            w.step(Phase::Drain, drain, t);
            w.step(Phase::Writeback, writeback, t);
            w.ctr.tiles += 1;
            tile_idx += 1;
        }
    }

    w.step(Phase::Done, lat.l_done, None);
    debug_assert_eq!(w.cycle, w.ctr.cycles_total);
    Walk {
        counters: w.ctr,
        timeline: w.timeline,
    }
}

/// Cycle-stepping state of [`walk`].
struct Walker {
    ctr: Counters,
    timeline: Vec<PhaseEvent>,
    cycle: u64,
    record: bool,
}

impl Walker {
    /// Spend `len` cycles in `phase`, incrementing counters one cycle at a time.
    fn step(&mut self, phase: Phase, len: u64, tile: Option<(u64, u64, u64)>) {
        if self.record && len > 0 {
            self.timeline.push(PhaseEvent {
                tile: tile.map(|t| t.0),
                mb: tile.map(|t| t.1),
                nb: tile.map(|t| t.2),
                phase,
                start: self.cycle,
                len,
            });
        }
        for _ in 0..len {
            self.ctr.cycles_total += 1;
            match phase {
                Phase::TileSetup => self.ctr.cycles_setup += 1,
                Phase::Stream => self.ctr.cycles_stream += 1,
                Phase::Flush => self.ctr.cycles_flush += 1,
                Phase::Drain | Phase::Writeback => self.ctr.cycles_drain += 1,
                Phase::Accept | Phase::Done => {}
            }
            self.cycle += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Derivation 2: closed form (SPEC §4)
// ---------------------------------------------------------------------------

/// Count of `x` in `0..len` with `(x mod period) / width == slot`.
fn count_slot(len: u64, period: u64, width: u64, slot: u64) -> u64 {
    let lo = slot * width;
    let hi = lo + width;
    let full = (len / period) * width;
    let rem = len % period;
    full + rem.clamp(lo, hi) - lo
}

/// Counters from the SPEC §4 closed form.
///
/// ```text
/// T_tile = L_SETUP + (L_RD + K) + ((R-1) + (C-1) + L_PE) + T_DRAIN + L_RQ + L_WB
/// tiles  = ceil(M / tile_rows) · ceil(N / tile_cols)
/// cycles_total = L_ACCEPT + tiles · T_tile + L_DONE
/// ```
///
/// Per-quadrant MACs: FULL — `K · #{m : (m mod 8) in quadrant rows} ·
/// #{n : (n mod 8) in quadrant cols}`. STRIP — the 16 tile columns split
/// into four groups of 4 (tile cols 0–3 → Q0, 4–7 → Q1, 8–11 → Q2,
/// 12–15 → Q3), every tile row is used by both halves, so
/// `Q_q = K · M · #{n : (n mod 16) / 4 == q}`.
///
/// # Panics
/// If the descriptor is out of range.
pub fn closed_form(d: &Descriptor, lat: &Latencies) -> Counters {
    d.validate().expect("descriptor out of range");
    let tiles = d.row_blocks() * d.col_blocks();
    let [setup, stream, flush, drain, writeback] = lat.phase_lengths(d);
    let t_tile = setup + stream + flush + drain + writeback;
    let q = spec::QUAD;
    let mac_useful_q = match d.mode {
        Mode::Full => {
            let rows = [
                count_slot(d.m, spec::ROWS, q, 0),
                count_slot(d.m, spec::ROWS, q, 1),
            ];
            let cols = [
                count_slot(d.n, spec::COLS, q, 0),
                count_slot(d.n, spec::COLS, q, 1),
            ];
            [
                d.k * rows[0] * cols[0],
                d.k * rows[0] * cols[1],
                d.k * rows[1] * cols[0],
                d.k * rows[1] * cols[1],
            ]
        }
        Mode::Strip => {
            let tc = d.mode.tile_cols();
            [0, 1, 2, 3].map(|slot| d.k * d.m * count_slot(d.n, tc, q, slot))
        }
    };
    Counters {
        cycles_total: lat.l_accept + tiles * t_tile + lat.l_done,
        cycles_setup: tiles * setup,
        cycles_stream: tiles * stream,
        cycles_flush: tiles * flush,
        cycles_drain: tiles * (drain + writeback),
        cycles_stall: 0,
        mac_useful: d.m * d.n * d.k,
        mac_useful_q,
        tiles,
    }
}

// ---------------------------------------------------------------------------
// Public prediction API
// ---------------------------------------------------------------------------

/// Predict all counters. Runs the FSM walk and cross-checks it against the
/// closed form and the §3.7 invariants; any disagreement is a model bug and
/// panics rather than returning a number nobody can trust.
pub fn predict(d: &Descriptor, lat: &Latencies) -> Result<Counters, DescError> {
    d.validate()?;
    let walked = phase_walk(d, lat);
    let closed = closed_form(d, lat);
    assert_eq!(
        walked, closed,
        "phase walk != closed form for {d:?} {lat:?}"
    );
    if let Err(e) = walked.check_invariants(d, lat) {
        panic!("invariant violated for {d:?}: {e}");
    }
    Ok(walked)
}

/// Mode with the lower predicted `cycles_total` for this shape (ties → FULL).
pub fn choose_mode(m: u64, k: u64, n: u64, flags: u8, lat: &Latencies) -> Mode {
    let total = |mode| closed_form(&Descriptor::new(mode, flags, m, k, n), lat).cycles_total;
    if total(Mode::Strip) < total(Mode::Full) {
        Mode::Strip
    } else {
        Mode::Full
    }
}

/// Operand bytes actually streamed from ABUF + WBUF: per tile, K reads of the
/// mode's used lanes (padded lanes included — the hardware reads them).
/// `= tiles · K · operand_bytes_per_cycle(mode)`.
pub fn operand_bytes_streamed(d: &Descriptor, c: &Counters) -> u64 {
    c.tiles * d.k * d.mode.operand_bytes_per_cycle()
}

/// Arithmetic intensity = useful MACs / operand bytes streamed (A + W only;
/// requant params and output writes excluded).
pub fn arithmetic_intensity(d: &Descriptor, c: &Counters) -> f64 {
    let bytes = operand_bytes_streamed(d, c);
    if bytes == 0 {
        return 0.0;
    }
    c.mac_useful as f64 / bytes as f64
}

/// Utilization ceiling from the mapping alone (SPEC §4 table): useful MACs
/// over PE-slots during the K MAC cycles of every tile, ignoring fill,
/// flush, drain and fixed overheads. `= M·N / (64 · tiles)`.
pub fn mapping_ceiling(d: &Descriptor) -> f64 {
    let tiles = d.row_blocks() * d.col_blocks();
    (d.m * d.n) as f64 / (spec::PEAK_MACS_PER_CYCLE * tiles) as f64
}

#[cfg(test)]
mod tests;
