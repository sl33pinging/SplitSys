# SplitSys Demo, Dashboard, and Pitch

**Goal:** an accelerator engineer from MatX or Annapurna Labs walks up, and within 10 seconds sees real hardware running and a number they care about. Within 2 minutes they understand the tradeoff, see measurement agree with a model, and see what the feature cost. Within 5 minutes of questions, they conclude the builder understands utilization, data movement, and verification.

**What those engineers value, in order:**
1. **Live silicon, not slides.** The board visibly does the work, with LEDs, the 7-segment display, and live counters.
2. **The right metric.** Utilization and operand bytes per MAC, measured by hardware counters.
3. **A model that predicts the hardware.** This shows you understand the machine, not just that it runs.
4. **The cost of the feature.** Area and Fmax with and without the split. Architects always ask.
5. **Verification rigor.** Bit-exact against a golden model, with a running mismatch count of 0.
6. **Candor about limits.** No CPU-speed claims; non-goals stated plainly.

The dashboard design follows that order. It should look like a serious internal performance dashboard, not a marketing page.

---

## 1. Physical setup

```
        [ portable monitor, eye level, angled toward the aisle ]
                       ▲ HDMI from laptop
 [ laptop (operator) ]           [ Urbana on a stand, LEDs + 7-seg facing visitors ]
          └──── USB micro-B (taped down) ────┘
 small table card: "SplitSys — a splittable systolic array for LLM matmuls, on FPGA"
```

- Monitor at native resolution (most portable monitors are 1920×1080). Brightness at maximum. Run the dashboard in Chrome or Edge kiosk mode (`--kiosk http://127.0.0.1:8080`) or with F11.
- **Windows settings:** power plan on High performance; plugged in; screen sleep set to Never; notifications off (Focus on); display scaling 100–125%. Set the monitor as an extended display and put the dashboard on it. Keep the operator console on the laptop screen.
- Tape the USB cable down and route it so nobody can catch it. Label the board.
- **Cold-start drill:** power cycle → board self-test LEDs on → `splitsys-host serve` → kiosk browser → Scene 1, in under 60 seconds. A one-click script (`demo/start_demo.ps1` or `.bat`) does every step after the power cycle.

---

## 2. Dashboard architecture

- `splitsys-host serve` runs a small local HTTP server on 127.0.0.1:8080. It serves the static assets from `demo/`, embedded in the binary with `include_str!`/`include_bytes!` so a single executable runs anywhere.
- **Live updates:** Server-Sent Events at `/events`. **Snapshot:** `GET /api/state`. **Commands:** `POST /api/run`, `POST /api/scene`.
- **Frontend:** a single HTML page with vanilla JS. Charts are hand-built SVG. **No external libraries, no CDN, no web fonts** (use the system font stack: `ui-sans-serif, system-ui, "Segoe UI", sans-serif`; numbers in `ui-monospace, "Cascadia Mono", Consolas, monospace`).
- **Data comes only from `results/*.json` and live runs.** Every number carries its source badge.
- **Offline replay:** if the board is unplugged, the dashboard can replay the last recorded `hw` session. Replay shows a persistent amber `REPLAY · recorded <time>` banner. This is a fallback; live is the default.

---

## 3. Visual language

- **Canvas:** 1920×1080, dark background `#0B0D10`, panels `#14181D`, hairline borders `#262C33`, text `#E8ECF1`, secondary text `#9AA4AF`.
- **Mode colors (colorblind-safe pair):** FULL = blue `#4C9AFF`, STRIP = orange `#FF9F43`. The model prediction is always a **white tick or outline** over the measured bar. Pass = `#3DDC97`, fail = `#FF5C5C`.
- **Source badges** in the corner of every panel: `HW` (green outline), `SIM` (blue outline), `MODEL` (gray outline), `REPLAY` (amber fill).
- **Type:** headline numbers 72–96 px monospace; panel titles 28 px; body text at least 20 px; nothing under 18 px. The display must be readable from 2 m away.
- **Motion:** only for meaning. The PE grid animates the wavefront during a run; bars grow when results land. No decorative motion.
- **Language:** every panel title states the finding, not the topic. For example, "STRIP doubles decode utilization", not "Utilization".

---

## 4. Data contract (`results/<source>/<run_id>.json`)

```json
{
  "run_id": "2026-10-24T15:02:11Z-ffn_up-m1-strip",
  "source": "hw",
  "build_id": "a1b2c3d4", "git_hash": "a1b2c3d", "clock_mhz": 100,
  "gemm": { "name": "ffn_up", "M": 1, "K": 64, "N": 256, "mode": "STRIP", "requant": true },
  "counters": { "cycles_total": 0, "cycles_setup": 0, "cycles_stream": 0, "cycles_flush": 0,
                "cycles_drain": 0, "cycles_stall": 0, "mac_useful": 0, "mac_useful_q": [0,0,0,0], "tiles": 0 },
  "model":    { "cycles_total": 0, "cycles_setup": 0, "cycles_stream": 0, "cycles_flush": 0,
                "cycles_drain": 0, "cycles_stall": 0, "mac_useful": 0, "mac_useful_q": [0,0,0,0], "tiles": 0,
                "model_git_hash": "..." },
  "derived":  { "util": 0.0, "util_model": 0.0, "operand_bytes_per_cycle": 0.0, "model_err_total": 0.0 },
  "check":    { "golden_match": true, "outputs_compared": 256, "mismatches": 0, "invariants_ok": true }
}
```

`results/cost.json` holds `{ "full_only": {lut, ff, dsp, bram, fmax_mhz, wns_ns}, "split": {...}, "source": "hw-build" }`.
`results/session.json` holds running totals: GEMMs verified, MACs verified, mismatches, uptime.

---

## 5. Scenes (keys 1 / 2 / 3; Space = run the scripted demo; A = attract loop on/off)

### Scene 1 — "Live: one array, two shapes" (default)

```
┌ SplitSys · 8×8 int8 systolic array on Spartan-7 · build a1b2c3d · 100 MHz · ● BOARD LIVE ─────────────────┐
│ ┌ PE ARRAY (live) ─────────────┐  ┌ THIS RUN ─────────────────────────┐ ┌ VERIFIED ON HARDWARE ─────────┐ │
│ │  8×8 grid, quadrant borders  │  │ ffn_up · M=1 K=64 N=256 · STRIP   │ │  1,284 GEMMs                   │ │
│ │  FULL: one blue 8×8 block    │  │                                   │ │  412,301,312 MACs bit-exact    │ │
│ │  STRIP: two orange 4×8 slabs │  │   21.4%   utilization   [HW]      │ │  0 mismatches                  │ │
│ │  wavefront animates per run  │  │   model 21.6%   (err −0.9%)       │ │  vs Rust golden model          │ │
│ │  per-quadrant util labels    │  │   2.0× vs FULL on same GEMM       │ └────────────────────────────────┘ │
│ └──────────────────────────────┘  │ cycle breakdown stacked bar:      │ ┌ COST OF SPLITTING ─────────────┐ │
│                                   │ setup | stream | flush | drain    │ │ +X LUT (+y%) · +Z FF · Fmax Δ  │ │
│                                   └───────────────────────────────────┘ └────────────────────────────────┘ │
│ bottom ticker: last 8 runs (name, mode, util, ✓)                                                            │
└─────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

(The numbers above are layout placeholders. Real values come only from `results/`.)

- **PE grid:** 8×8 cells, with quadrant borders drawn heavier. In FULL mode all 64 cells are tinted blue. In STRIP mode the upper and lower halves are tinted orange, with a visible "seam" at row 4 labeled `W lanes 8–15 injected here`. During a run, a diagonal wavefront animates. The animation timing comes from the cycle model and is labeled `dataflow animation: model`. **Idle PEs for this shape (for example, rows 1–7 at M = 1 in FULL mode) are drawn dark.** That waste is the whole point and should be obvious at a glance.
- **Per-quadrant utilization** from `mac_useful_q` (HW) is printed in each quadrant.
- **Verified panel:** live counters from `results/session.json`. This is the strongest credibility signal on the screen.

### Scene 2 — "Results: where the split pays"

- **Headline chart:** utilization vs M (1, 2, 4, 8, 16, 32, 64), log-2 x-axis, two lines (FULL blue, STRIP orange) of **measured** points, with **model** shown as hollow white markers. Shade the "decode regime" (M ≤ 4). Title: "STRIP wins at small M; both converge at prefill".
- **Benchmark bars:** for each of qkv / attn_out / ffn_up / ffn_down at M = 1 and M = 64, paired bars (FULL, STRIP) with model ticks.
- **Model-vs-measured table:** per counter, measured, model, and error %, with any value over 5% highlighted and an explanation footnote.

### Scene 3 — "Architecture and the tradeoff"

- Block diagram of the design (from SPEC §2) as SVG, with the STRIP mux and the `4 + c` skew delay called out.
- **Roofline-style plot:** x = arithmetic intensity (MACs per operand byte), y = achieved MACs/cycle. Compute roof at 64. Bandwidth ceilings: 16 B/cycle (FULL) and 20 B/cycle (STRIP). Points for each benchmark in both modes.
- **Tradeoff card:** "STRIP buys utilization on skinny matmuls with +25% operand bandwidth per cycle and +X LUTs."
- **Limits card:** "Not a speed claim vs CPUs. UART-bound end to end. No DDR3. Attention matmuls and softmax on host."

### Scripted demo (Space)

Runs, in order, with a 1.5 s pause between each:
1. `ffn_up`, M = 1, FULL
2. the same, STRIP
3. `ffn_up`, M = 64, FULL
4. the same, STRIP

It then jumps to Scene 2 with the newest points highlighted.

**Attract loop (A):** cycles through Scenes 1→2→3 every 20 s, running a random verified GEMM every 5 s so the board LEDs and counters stay live while nobody is talking.

---

## 6. Booth checklist (10:15 Sunday)

- [ ] Self-test LEDs on after power-up; 7-seg shows the last run.
- [ ] Dashboard Scene 1 live; `BOARD LIVE` badge green; `build_id` matches `fpga/known_good/`.
- [ ] Attract loop running; operator console visible only on the laptop.
- [ ] Backup video on the laptop desktop; REPLAY mode tested.
- [ ] Spare USB cable, power strip, chargers in reach.
- [ ] Devpost submitted with the correct table number.

---


## 7. Two-minute pitch

<!--
Owner: demo-storyteller. Sources for every number in §7–§8:
  [M] MODEL results/model/sweep.json @ model_git_hash 72da65248adf (also results/plots/manifest.json)
  [B] BUILD results/cost.json, committed 8ccb333: S4 freeze builds, build_id 0x52A66216 (git 52a66216), post-route, both builds
  [SIM] results/sim/*.json top level only (never archive_dirty/): 70 co-sim records (results/sim/session.json gemms_run 70,
        mismatches 0). 66 at build 0f402cc7 / git f402cc7 (committed e2cea28) + 4 scripted-demo at build 05900507 /
        git 5900507 (committed fb89e37); same RTL (git diff f402cc7 5900507 -- rtl spec is empty); all counters == model,
        cycles_stall 0 on all 70. SIM is NOT hardware.
  MatX public facts: https://matx.com/research/series_b, fetched 2026-10-24: "The MatX One chip is based on a
        splittable systolic array"; "combines the low latency of SRAM-first designs with the long-context support of HBM".
  [V] tb/SIGNOFF.md, DECISIONS D-016/D-017   [S] SPEC v1.1 constants
  ⟦HW: …⟧ / ⟦SIM: …⟧ = fill from results/hw or results/sim only. If a value never arrives, say so; never guess.
Word counts (≤ 280 each, ~150 wpm → under 2:00) include the stage directions, so the spoken
part is shorter. Check: see the "Pitch word counts" command in §9.
-->

**Delivery rules.**
- Lead with the visible effect: dark PEs, then lit PEs. Then give the number, then the cost.
- Say "model" or "measured" every time a number is spoken.
- If the board is in REPLAY or SIM, say so in the first sentence.

### 7.1 Default (Cal Hacks judges)

<!-- pitch:general -->
*[Scene 1, point at the board.]* LLM chips are built around big grids of multipliers called systolic arrays. When a model writes text one token at a time, each matrix multiply is a single row tall, and most of the grid sits idle.

*[Run `ffn_up` M=1, FULL.]* This is our 8-by-8 array on a Spartan-7 FPGA, running one transformer layer at decode size. Watch the grid: only the top row lights up. The hardware counters measure ⟦HW: ffn_up M=1 FULL util⟧ utilization.

*[Same job, STRIP.]* Same hardware, rewired at runtime into a 4-by-16 shape. The bottom half now computes extra output columns instead of idling. Measured: ⟦HW: ffn_up M=1 STRIP util⟧, which is ⟦HW: STRIP/FULL ratio⟧ times higher. Every output still matches our Rust reference bit for bit.

*[Scene 2.]* Across batch sizes, the split pays off when generating one token at a time, and the two modes converge once the batch fills the array. The hollow markers are our cycle model's predictions, written before the board was touched. They match the counters to within ⟦HW: max model error⟧.

*[Scene 3.]* It isn't free. The split needs 25% more operand bandwidth every cycle and about two thousand more lookup tables, most of them in the wider output path. That's the real tradeoff, and we measured it.

*[Verified panel.]* ⟦HW: GEMMs verified⟧ matrix multiplies on this board, ⟦HW: mismatches⟧ mismatches. Everything was simulated first, including the real host software driving the simulated chip. Happy to go deeper.
<!-- /pitch -->

<!-- "about two thousand more lookup tables" = cost.json breakdown.delta.total.lut 2247 [B]. "25%" = 20 vs 16 B/cycle [S]. -->

### 7.2 MatX variant

**Framing:** a small-scale, measured exploration of the splittable-array idea. Claim nothing about MatX's design. What MatX has said publicly (matx.com/research/series_b) is that it builds LLM chips based on a splittable systolic array, combining SRAM-first latency with HBM for long context. Reiner Pope is first author of *Efficiently Scaling Transformer Inference*.

<!-- pitch:matx -->
*[Scene 1.]* Decode and prefill want different array shapes. This is a small-scale, measured exploration of the splittable-array idea: an 8-by-8 int8 output-stationary array that rewires at row 4 into 4-by-16, per job, with no reconfiguration.

*[Run `ffn_up` M=1, FULL then STRIP.]* At one row, the mapping caps FULL at 12.5% and STRIP at 25%. Fixed per-tile setup, flush, and drain bring our model to 7.0% and 14.4%. The counters read ⟦HW: FULL util⟧ and ⟦HW: STRIP util⟧.

*[Scene 2.]* At prefill both shapes fill the array. The model still has STRIP slightly ahead, 57.7% against 55.6%, but that's drain, four rows per tile instead of eight, not mapping. Overlap the drain and the gap goes away.

The method is model-first, in the spirit of *Efficiently Scaling Transformer Inference*. A cycle model predicts all twelve hardware counters before measurement. Worst per-counter error on the board: ⟦HW: max error⟧.

*[Scene 3.]* The cost of splitting: STRIP streams 20 operand bytes per cycle against 16, so 25% more. In area, post-route, it's +2,247 LUTs and 16 DSPs. Only about 670 LUTs are the split muxing. The rest is the 16-lane requantizer that a 16-wide tile needs. In our design, the expensive part of splitting wasn't the array, it was the edges.

Next experiment: stream weights from DRAM, where FULL's lower bandwidth might be the better choice.
<!-- /pitch -->

<!-- 12.5/25 = SPEC §4 ceilings [S]; 7.0/14.4, 57.7/55.6 [M]; 2,247 LUT, 16 DSP, "about 670" = 666 = 376+290 LUT (breakdown.delta array+sequencer) [B]. -->

### 7.3 Annapurna Labs variant

**Framing:** utilization, SRAM feeding the array, explicit data movement, the area cost of flexibility, and DV rigor. The public facts about Trainium are a systolic tensor engine, a large on-chip SRAM, and software-controlled data movement. Claim nothing beyond those.

<!-- pitch:annapurna -->
*[Scene 1.]* A systolic engine is only as good as the fraction of cycles its PEs do useful work, and that depends on whether the SRAM keeps it fed. SplitSys is an 8-by-8 int8 array on a Spartan-7, with on-chip BRAM buffers and a sequencer that moves every operand explicitly: setup, stream, flush, drain, writeback.

*[Run `ffn_up` M=1, FULL then STRIP.]* On a one-row decode matmul, FULL keeps one row of eight busy: ⟦HW: FULL util⟧ measured. Rewired as 4-by-16: ⟦HW: STRIP util⟧, still bit-exact.

The price is feed bandwidth: 20 operand bytes per cycle instead of 16. The BRAM ports are sized for both, and we measured ⟦HW: cycles_stall⟧ stall cycles. Twelve hardware counters split every cycle into setup, stream, flush, drain, and stall. A cycle model predicts each one, with a worst error of ⟦HW: max error⟧.

*[Scene 3.]* Area, post-route: +2,247 LUTs and +16 DSPs, with timing met at 100 MHz in both builds. About 670 of those LUTs are the split itself. Most of the rest is the 16-lane requantizer the wider tile needs.

Verification: a Rust golden model checks every run bit-exactly. Eleven test suites are signed off, and injected-bug mutants were caught in five of them. A pin-level system simulation passed 617 of 617 jobs, and the real host ran against the simulated chip. On this board: ⟦HW: GEMMs verified⟧ GEMMs, ⟦HW: mismatches⟧ mismatches.

So, what does reconfigurability cost, and when does it pay? Here it pays at decode and barely matters at prefill.
<!-- /pitch -->

---

## 8. Q&A bank (rehearse these)

Each answer takes under 30 seconds. Rows marked ⟦HW⟧ get their number from `results/hw` before judging; if the number never arrives, say "we didn't measure that".

### 8.1 Core

| Question | Answer (short) |
| --- | --- |
| Why not just batch decode requests? | Batching raises M and fixes utilization for throughput serving. It doesn't help latency-bound single-stream decode, or small per-head attention matmuls. STRIP targets small M, at the cost of more operand bandwidth. |
| What does the split cost? | Post-route: +2,247 LUTs (+43.7%), +1,590 FFs, +16 DSPs, BRAM unchanged [B]. The STRIP muxes in the array and sequencer are only +666 LUTs. Most of the rest is requant lanes 8–15, which the FULL-only build drops because FULL drains 8 lanes. Bandwidth: +25% operand bytes per cycle [S]. |
| Is the SRAM keeping up? | On hardware, `cycles_stall` = ⟦HW: cycles_stall⟧. In co-simulation it was 0 on all 70 runs [SIM]. The BRAM ports supply 20 B/cycle, so the model assumes zero stalls [S]. With a narrower buffer or DRAM-fed weights, STRIP would stall first, because it needs 25% more bytes. |
| How do you know it's correct? | A Rust golden model checks every run bit-exactly. The invariants `mac_useful == M·N·K` and Σquadrants == total are checked on every run. The eligible sign-off run passed 11 of 11 cocotb suites and 64 of 64 tests, including 262 directed jobs on the core. Mutants were caught in the skew, counters, packet, array and core suites. An eligible pin-level system simulation passed 617 of 617 jobs [V]. Co-simulation with the real host: 70 runs, 0 mismatches [SIM]. There's a self-test ROM in the bitstream. On hardware: ⟦HW: GEMMs / mismatches⟧. |
| How accurate is the model? | Per-counter error on the board: ⟦HW: max per-counter error⟧. In simulation the counters equal the model exactly: unit, core, and system suites [V], and 70 co-sim runs with 0 counter differences [SIM]. That's expected: the sequencer is deterministic and stall-free, so the model and RTL implement the same state lengths. The model's value is that it's independent code, so any disagreement is a bug. |
| Why output-stationary? | Accumulators stay in the DSP P register, there's no partial-sum traffic, and drain is simple. Weight-stationary favors weight reuse across many rows, which decode lacks. |
| Why int8? | One DSP48E1 per MAC, the same as common inference quantization practice, and the result stays bit-exact and checkable. |
| Why is end-to-end slow? | The UART at 115200 baud. We measure on-chip cycles from the counters and never present wall time as accelerator speed. |
| What would you do with more time? | Shadow accumulators to overlap drain. Load requant parameters during STREAM instead of a 16-cycle setup. INDEP mode for per-head attention. DDR3 weight streaming to study memory-bound decode. |
| How does this scale? | The utilization ceiling is about M over the array height, until M reaches the height. Bandwidth per MAC grows as arrays get shorter and wider. The model predicts both; show the T_tile formula (SPEC §4). |
| What was hardest? | The STRIP lower-half alignment, below. |

### 8.2 Questions the results raise

| Question | Answer (short) |
| --- | --- |
| STRIP is faster even at prefill. Isn't FULL pointless? | In the model, STRIP is never slower: 1.013–1.036× at M ≥ 8 [M]. That edge is drain, 4 rows per tile against 8, not mapping, and it's paid for by the 16-lane output path. FULL wins when operand bandwidth is the limit, because it needs 16 B/cycle instead of 20. We didn't build that regime (no DRAM). |
| Why is decode at 7% / 14%, not 12.5% / 25%? | Those are mapping ceilings [S]. Each tile also spends fixed cycles on setup (16), flush (17), and drain plus writeback. For `ffn_up` that's 115 cycles per FULL tile against 64 cycles of useful streaming, so 7.0% and 14.4% [M]. |
| Why does prefill top out around 56%? | The same per-tile overhead. Stages don't overlap, so the array idles during drain. With K = 64, the useful K cycles are about 56–58% of a tile [M]. `ffn_down` (K = 256) reaches 83.4% (FULL) and 84.5% (STRIP) [M]. |
| Your simulation matches the model exactly. Isn't that circular? | Partly, and that's why we don't lean on it. The model and RTL share confirmed latencies, and the sequencer is deterministic with no DRAM, so SIM == MODEL [SIM] only shows the RTL implements the spec. They are independent code, so a mismatch would expose a bug. The real test is the board: ⟦HW: max per-counter error⟧. |
| Is the 2× just the mapping ceiling restated? | The ceiling explains the direction. The measured number includes every overhead cycle, and STRIP also halves the tile count (16 vs 32 for `ffn_up`) [M]. Measured: ⟦HW: ratio⟧ against a model of 2.07× [M]. |
| Why +44% LUTs for a mux? | It isn't for the mux. The muxes are about 670 LUTs [B]. The FULL-only build also drops 8 of the 16 requant lanes (16 DSPs, about 1,500 LUTs). That's the honest bill for a 16-wide output tile. |
| Did splitting cost Fmax? | We can't say, and we don't claim it. Both builds meet 100 MHz: WNS +2.097 ns (FULL-only), +0.523 ns (split) [B]. The split build's worst path is a replicated reset net with 0 logic levels and 96% routing. That's placement, not STRIP logic. The same RTL closed at +1.143 ns in an earlier build [B, cost.json at 8aeb9b0]. The WNS-based estimates (126.5 vs 105.5 MHz) aren't a sweep. |
| What if hardware disagrees with the model? | Then it's a bug in one of them, and we reproduce it in simulation before touching the board again. ⟦HW: if it happened, what it was⟧. |
| Is d_model = 64 a toy? | Yes, by design: it fits on-chip and every run is checkable. Utilization depends on M against the array height and on K against the per-tile overhead, not on d_model. That's why K = 256 `ffn_down` reaches 83% at prefill in the model [M]. |
| Is the data real model weights? | No. It's synthetic int8 from a fixed seed, with real LLM shapes. Bit-exactness doesn't depend on the values. Utilization doesn't either, because the counters count in-bounds MACs, not nonzero values. |

### 8.3 War stories (real, from the logs; pick one)

| Story | 30-second version |
| --- | --- |
| STRIP lower-half skew | The spec gave two formulas for the delay of W lanes 8–15 entering at row 4: `4 + c` in one section and `12 + c` in another. `12 + c` would have fed the lower half operands 8 cycles late. It was caught in spec review, and the spec, RTL, and tests were fixed together (D-008). The skew unit test now fails on the old formula, so it would catch a regression [V]. |
| UART pins | The vendor constraints file names the UART pins from the USB bridge's side. The schematic showed the FPGA's RX and TX had to be swapped before any hardware test, which stays unconfirmed until hardware step H-2. One link in that reasoning came from a datasheet recalled from memory, so we also prebuilt a bitstream with the pins swapped back. ⟦HW: which one worked at H-2⟧. |
| Latency surprise | The requant pipeline measured 7 cycles, not the provisional 4 (D-016). Every latency lives in one spec file, so the RTL, the model, and the tests changed together, and the tests assert measured latency == the spec constant [V]. |
| The harness that lied | Under `make -j`, cocotb could skip a simulation and still exit 0. We fixed it so every run must produce a fresh results file, and re-ran every suite [V]. |

---

## 9. Failure fallbacks (in order)

1. **Live UART demo.**
2. Board self-test (BTN0, LEDs) plus REPLAY mode of the recorded `hw` session (amber banner).
3. Backup video.

Say which mode you are in. Never present replayed or simulated data as live.

**What to say in each mode.**
- **REPLAY:** "This is a recording of the hardware session from ⟦HW: time⟧. The board is running its self-test live." Show LED0 and LED1.
- **SIM only (PLAN cut rule):** "STRIP on hardware isn't working yet; these STRIP numbers are from co-simulation of the same RTL, labeled SIM." Never round SIM into HW. The SIM numbers you may say: `ffn_up` M=1 FULL 6.95% → STRIP 14.40%, and M=64 55.65% / 57.65%. These come from 70 co-sim runs from two clean builds of the same RTL (66 at f402cc7, 4 at 5900507), with every counter equal to the model [SIM].
- **Board dead:** give the self-test and video pitch. The numbers come from the README tables with their source labels.

**Storyteller pre-judging checks** (in addition to the §6 booth checklist):
- [ ] `scripts/wsl 'python results/plots/make_plots.py --check'`: plots regenerated with `results/hw` present. The titles now say `HW`, not `MODEL`.
- [ ] No placeholder left in public text: `grep -c '⟦' README.md docs/devpost.md` prints 0 for both. Strip the DEMO.md placeholders before rehearsal.
- [ ] Every spoken number has been re-read from `results/` that morning. The model hash in the plots matches `results/model`.
- [ ] Pitch word counts (each ≤ 280): `awk '/<!-- pitch:/{f=1;n=$0;next}/<!-- \/pitch -->/{print n, c; c=0; f=0} f{c+=NF}' docs/DEMO.md`
- [ ] Each pitch has been rehearsed once against a skeptical-engineer Q&A from §8.2.
