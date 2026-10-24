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

**Default (general judges):**
1. *(10 s)* "LLM chips are built around big systolic arrays. When a model generates text one token at a time, those matmuls are one row tall, and most of the array sits idle."
2. *(25 s)* Run `ffn_up` M = 1 in FULL. "Here's the array on a real decode-shaped matmul. Most PEs are dark, and utilization is the number on screen."
3. *(25 s)* Switch to STRIP. "Same hardware, rewired at runtime into a 4×16 shape. Utilization doubles, and the result is still bit-exact against our golden model."
4. *(20 s)* Scene 2. "Across batch sizes, the split wins in the decode regime, and both modes converge at prefill. Hollow markers are our cycle model's predictions; they match the hardware counters."
5. *(20 s)* Scene 3. "It isn't free: the split costs 25% more operand bandwidth per cycle and this many LUTs. That's the real design tradeoff."
6. *(20 s)* "Every number here was measured on this board. Over a thousand GEMMs verified, zero mismatches. Happy to go deeper."

**MatX variant (emphasis):** the prefill vs decode regimes and why array shape matters for LLM-specific silicon; the splittable-array idea at small scale; the operand-bandwidth cost of small arrays; the model-first methodology in the spirit of Pope et al., *Efficiently Scaling Transformer Inference*. Do not claim to know MatX's internal design. Say "a small-scale exploration of the splittable-array idea."

**Annapurna Labs variant (emphasis):** systolic matmul engine utilization; on-chip SRAM feeding the array and whether it keeps up (`cycles_stall`, bytes/cycle); explicit, sequencer-controlled data movement; DV rigor (golden model, invariants, counters equal to the model). Frame it as a question they deal with: "what does reconfigurability cost in area and timing, and when does it pay?"

---

## 8. Q&A bank (rehearse these)

| Question | Answer (short) |
| --- | --- |
| Why not just batch decode requests? | Batching raises M and fixes utilization for throughput serving. It doesn't help latency-bound single-stream decode or small per-head attention matmuls. STRIP targets small M at the cost of more operand bandwidth. |
| What does the split cost? | The cost panel: ΔLUT, ΔFF, ΔFmax from the `ENABLE_STRIP=0` build, plus 25% more operand bytes per cycle. |
| Is the SRAM keeping up? | Show the measured `cycles_stall` and operand bytes/cycle. The BRAM ports are sized to supply 20 B/cycle, so stalls are expected to be 0; quote what was measured. Explain what would change with a narrower buffer or DDR-fed weights. |
| How do you know it's correct? | Rust golden model, bit-exact compare on every run, invariants (`mac_useful == M·N·K`), cocotb unit and system tests, self-test ROM. |
| How accurate is the model? | The per-counter error table. Calibrated once against hardware, then frozen before measurement. |
| Why output-stationary? | Accumulators stay in the DSP P register, no partial-sum traffic, simple drain. Weight-stationary would favor weight reuse across many rows, which decode lacks. |
| Why int8? | One DSP48E1 per MAC, matches inference quantization practice, and keeps the result bit-exact and checkable. |
| Why is end-to-end slow? | UART. On-chip cycles are what we measure; wall time is dominated by the link. |
| What would you do with more time? | INDEP mode for per-head attention; shadow accumulators to overlap drain; DDR3 weight streaming to study memory-bound decode; automatic mode selection in the sequencer. |
| How does this scale? | Utilization ceilings scale with array height versus M. Bandwidth per MAC grows as arrays shrink. The model predicts both; show the formula. |
| What was hardest? | Pick one real bug (for example, the STRIP lower-half skew alignment) and explain how a test caught it. |

---

## 9. Failure fallbacks (in order)

1. **Live UART demo.**
2. Board self-test (BTN0, LEDs) plus REPLAY mode of the recorded `hw` session (amber banner).
3. Backup video.

Say which mode you are in. Never present replayed or simulated data as live.
