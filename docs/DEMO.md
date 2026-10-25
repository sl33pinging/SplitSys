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

<!-- Sources: fpga/known_good/CURRENT -> fpga/known_good/KG3/top.bit; KG3/archive.txt (bit_md5 94bb7160..., lead_note on the
     FTDI latency timer); results/hw/logs/KG3/Q-1 (echo 25,385.5 ms at 16 ms timer), Q-7 (1,566.5 ms at 1 ms), Q-6 (LM 26.01 s
     total, 2.16 s/step, 12 steps), Q-8 (flash OK); docs/STATUS.md 22:08 (boot ~1 s, human) and 22:19 (start_demo auto-detected
     1,562,500 baud, build 21dd6850, dashboard 2.1 s wall); demo/start_demo.ps1 (-Baud auto default); demo/app.js key '4',
     demo/lm.js keys G (8 tokens, prompt "Once upon a time", mode auto) and X (stop); fpga/tcl/program.tcl <bitfile>. -->

- [ ] **Laptop prerequisite, check first: COM3 FTDI latency timer = 1 ms.** Device Manager → Ports (COM & LPT) → USB Serial Port (COM3) → Properties → Port Settings → Advanced… → Latency Timer (msec) = 1 → OK. Close every program using COM3 first. Without it KG3 still works, just slowly: the same 100 KB echo took 25.4 s at the 16 ms default and 1.57 s at 1 ms.
- [ ] **Cold start (KG3)** (J14 must stay fitted; it selects boot from flash):
  1. Power on. The board configures itself from QSPI flash in about 1 s (observed Saturday 22:08).
  2. On the laptop run `powershell -ExecutionPolicy Bypass -File demo\start_demo.ps1 -Transport serial:COM3 -WindowPosition <monitor x,y>` (COM3 on this laptop; check Device Manager if it differs).
  3. The script detects the baud and prints the rate: expect `board answered at 1562500 baud`, build_id 21dd6850. The kiosk opens on the monitor and shows `BOARD LIVE`.
  4. Saturday's run took 2.1 s of laptop wall time. That is startup time, never accelerator speed.
  - If the board stays dark: power off, reseat J14, power on.
- [ ] **Self-test:** press BTN0 → LED0 and LED1 on (SPEC: self-test runs on BTN0, not automatically at power-up). RGB LEDs dark.
- [ ] **Dashboard:** Scene 1 live, `BOARD LIVE` badge green, `build_id` matches `fpga/known_good/CURRENT` (KG3 = 0x21DD6850). If asked: the benchmark sweep was measured on KG1 (0x52A66216), and KG2 and KG3 have the same datapath. Both re-ran the 200-GEMM random batch with identical counters.
- [ ] **Language model (Scene 4):** press `4`, then `G`. It generates 8 tokens live from "Once upon a time", about 2 s per step (Saturday: 2.16 s, 26 s for all 12 steps). `X` stops it. Say "the board does the linear layers; the laptop does the rest".
- [ ] Attract loop running; operator console visible only on the laptop.
- [ ] **Fallback ready:**
  - **KG2 at 115,200 baud** over JTAG: `scripts/vivado -mode batch -source fpga/tcl/program.tcl -tclargs fpga/known_good/KG2/top.bit`. Then run `start_demo.ps1` as above. It auto-detects 115,200, but generation takes about 30 s per step.
  - **REPLAY** (`R`, or `start_demo.ps1 -Replay results\hw`) shows the recorded runs, and Scene 4 replays a recorded generation. Pick the recorded story in the REPLAY view before judging, and read its build, baud and GEMM count off the screen; don't assume which run it is.
- [ ] Backup video on the laptop desktop; REPLAY mode tested.
- [ ] Spare USB cable, power strip, chargers in reach.
- [ ] Devpost submitted with the correct table number.

---


## 7. Two-minute pitch

<!--
Owner: demo-storyteller. Sources for every number in §7–§8 (full derivations in README.md HONESTY KEY):
  [M] MODEL results/model/sweep.json @ model_git_hash 72da65248adf (also results/plots/manifest.json)
  [B] BUILD results/cost.json, committed 23acf6a: S4 freeze rebuild, build_id 0x38A3950A (git 38a3950a), post-route, both builds;
        split build = KG2. Values: full_only/split lut 5146/7389, ff 6346/7936, dsp 80/96, wns 2.188/0.744, fmax est 128.0/108.0;
        delta total lut 2243 (43.6%), ff 1590, dsp 16; array 378 + sequencer 289 = 667 LUT; requant 1503 LUT / 16 DSP.
        Split WNS on same datapath: 1.143 (8bbd7ba6), 0.523 (52a66216 KG1), 0.744 (38a3950a KG2), 0.473 (21dd6850 KG3,
        fpga/reports/kg3_top/summary.json: lut 7377, ff 7928, dsp 96, bram 26).
  [SIM] results/sim/*.json top level only: 70 co-sim records, all counters == model, cycles_stall 0. SIM is NOT hardware.
  MatX public facts: https://matx.com/research/series_b, fetched 2026-10-24: "The MatX One chip is based on a
        splittable systolic array"; "combines the low latency of SRAM-first designs with the long-context support of HBM".
  [HW] results/hw/*.json top level, git-tracked only: 773 GEMM records (source "hw"), all golden_match, 0 mismatches,
        393,129,309 MACs, 3,099,658 outputs, 0 of 9,276 counter values != model, cycles_stall 0 on all.
        KG1 52a66216: 272 (H-5 200, H-6 56, H-7 16). KG2 38a3950a: 301 (check 1, H-5 200, auto 100).
        KG3 21dd6850 (CURRENT, live): 200 (Q-5). H-5 counters on KG2, KG3 == KG1 on 200/200.
        ffn_up M=1: 20-40-41Z util 0.069527 (6.95%) / 0.143982 (14.40%), ratio 2.0709; M=64 0.556484 / 0.576536.
        H-5 pairs 97: STRIP more cycles on 31 (max 1.315x, M=22 K=250 N=17, 3566 vs 2711).
        AUTO (KG2, results/hw/logs/KG2/G5_auto_random100_seed1.log): FULL 38 / STRIP 62, 33 predicted-slower -> FULL,
        measured == model 100/100, 1,225,768 cycles; MODEL always-FULL 1.054x, always-STRIP 1.016x.
        Model pick vs measured on KG1 shapes run both ways: 125/125 (5 measured ties), storyteller recomputation.
        Pooled across KG1/KG2/KG3 tracked records: 126/126 (extra shape M=9 K=3 N=9, KG2 check run + auto); KG2 H-5 97/97,
        KG3 97/97. Agrees with red-team G5 recompute (lead: dashboard shows 126/126).
        LM results/hw/lm/ (3 tracked records): KG2 8 tok 444/444, KG2 30 tok 1258/1258, KG3 8 tok 444/444; 37 GEMMs/step,
        29,540 cycles/step = 295.4 us, 259,328 MACs/step (13.7%); wall 30.76 s/step (KG2, 115,200, 16 ms timer),
        2.16 s/step (KG3, 1,562,500, 1 ms timer); 245,623 bytes sent per step (steady state); WBUF 65,536 B,
        54,272 B of 261,888 weight B pinned. int8 vs f32 (sw/crates/lm/tests/stories260k.rs, golden, not board):
        61 identical generated tokens, then diverges; teacher-forced top-1 250/256; f32 == published readme text.
        Echo 100 KB: 25.4 s (16 ms timer) vs 1.57 s (1 ms), results/hw/logs/KG3/Q-1, Q-7.
  [V] tb/SIGNOFF.md, tb/system/SIGNOFF_SYSTEM.md, DECISIONS D-016/D-017/D-020   [S] SPEC v1.1 constants
Word counts (≤ 280 each, ~150 wpm → under 2:00) include the stage directions, so the spoken
part is shorter. Check: see the "Pitch word counts" command in §9.
-->

**Delivery rules.**
- Lead with the visible effect: dark PEs, then lit PEs. Then give the number, then the cost.
- Say "model" or "measured" every time a number is spoken.
- If the board is in REPLAY or SIM, say so in the first sentence.
- Any wall time you speak names its link: "two seconds per token over the serial link". The on-chip number is 295 µs.

### 7.1 Default (Cal Hacks judges)

<!-- pitch:general -->
*[Scene 1, point at the board.]* LLM chips are built around big grids of multipliers called systolic arrays. When a model writes text one token at a time, each matrix multiply is a single row tall, and most of the grid sits idle.

*[Run `ffn_up` M=1, FULL.]* This is my 8-by-8 array on a Spartan-7 FPGA, running one matmul of a transformer layer at decode size. Only the top row lights up: 6.95% utilization, from the hardware counters.

*[Same job, STRIP.]* Same hardware, rewired at runtime into a 4-by-16 shape. The bottom half now computes extra output columns instead of idling. Measured: 14.40%, 2.07 times higher, and every output matches my Rust reference bit for bit.

*[Scene 2.]* The split pays off one token at a time, and the modes converge once the batch fills the array. The hollow markers are my cycle model's predictions, written before the board was touched. They match all twelve hardware counters exactly, on every run.

*[Scene 4, press G.]* It also runs a real model: every linear layer of a small public language model executes on this array, bit-exact. On-chip, a token takes 295 microseconds. The two seconds you see are mostly the serial link.

*[Scene 3.]* It isn't free: 25% more operand bandwidth every cycle, and over two thousand more lookup tables, mostly in the wider output path. I measured that too.

*[Verified panel.]* 773 recorded matrix multiplies on this board, zero mismatches. Happy to go deeper.
<!-- /pitch -->

<!-- "over two thousand more lookup tables" = cost.json breakdown.delta.total.lut 2243 [B]. "25%" = 20 vs 16 B/cycle [S].
     "773" = committed results/hw GEMM records, three builds [HW]. "295 microseconds" = 29,540 cycles [HW].
     "two seconds" = 2.16 s/step at 1,562,500 baud with the 1 ms FTDI timer (KG3 Q-6) — say "about two" only on KG3 with the
     1 ms timer; on KG2 or the 16 ms timer, say "the seconds you see". -->

### 7.2 MatX variant

**Framing:** a small-scale, measured exploration of the splittable-array idea. Claim nothing about MatX's design. What MatX has said publicly (matx.com/research/series_b) is that it builds LLM chips based on a splittable systolic array, combining SRAM-first latency with HBM for long context. Reiner Pope is first author of *Efficiently Scaling Transformer Inference*.

<!-- pitch:matx -->
*[Scene 1.]* Decode and prefill want different array shapes. This is a small-scale, measured exploration of the splittable-array idea: an 8-by-8 int8 output-stationary array that rewires at row 4 into 4-by-16, per job, with no reconfiguration.

*[Run `ffn_up` M=1, FULL then STRIP.]* At one row, the mapping caps FULL at 12.5% and STRIP at 25%. Fixed per-tile setup, flush, and drain bring my model to 7.0% and 14.4%. The counters read 6.95% and 14.40%: 2.07×.

*[Scene 2.]* At prefill both shapes fill the array. STRIP is still slightly ahead, measured 57.65% against 55.65%, but that's drain, four rows per tile instead of eight, not mapping.

The method is model-first, in the spirit of *Efficiently Scaling Transformer Inference*. A cycle model predicted all twelve hardware counters before measurement. Error on the board: zero, on all 773 GEMMs. So the host lets the model pick the shape per job. On 125 shapes I'd measured both ways on the first build, 126 across all three, it never picked the slower one.

*[Scene 3.]* The cost of splitting: STRIP streams 20 operand bytes per cycle against 16, so 25% more. In area, post-route, it's +2,243 LUTs and 16 DSPs. Only about 670 LUTs are the split muxing. The rest is the 16-lane requantizer that a 16-wide tile needs.

*[Scene 4.]* Every linear layer of a small public language model runs here at M = 1, bit-exact. Next experiment: stream weights from DRAM, where FULL's lower bandwidth might win.
<!-- /pitch -->

<!-- 12.5/25 = SPEC §4 ceilings [S]; 7.0/14.4 [M]; 57.65/55.65 [HW]; 2,243 LUT, 16 DSP, "about 670" = 667 = 378+289 LUT
     (breakdown.delta array+sequencer) [B]; 125 = KG1 shapes measured in both modes, model pick had the lowest measured
     cycles on 125/125 (5 ties) [HW]; 126 = pooled across all three builds, 126/126 [HW]. -->

### 7.3 Annapurna Labs variant

**Framing:** utilization, SRAM feeding the array, explicit data movement, the area cost of flexibility, and DV rigor. The public facts about Trainium are a systolic tensor engine, a large on-chip SRAM, and software-controlled data movement. Claim nothing beyond those.

<!-- pitch:annapurna -->
*[Scene 1.]* A systolic engine is only as good as the fraction of cycles its PEs do useful work, and that depends on whether the SRAM keeps it fed. SplitSys is an 8-by-8 int8 array on a Spartan-7, with on-chip BRAM buffers and a sequencer that moves every operand explicitly: setup, stream, flush, drain, writeback.

*[Run `ffn_up` M=1, FULL then STRIP.]* On a one-row decode matmul, FULL keeps one row of eight busy: 6.95% measured. Rewired as 4-by-16: 14.40%, still bit-exact.

The price is feed bandwidth: 20 operand bytes per cycle instead of 16. The BRAM ports are sized for both, and I measured zero stall cycles on all 773 runs. Twelve hardware counters split every cycle into setup, stream, flush, drain, and stall. A cycle model predicted each one exactly.

*[Scene 3.]* Area, post-route: +2,243 LUTs and +16 DSPs, with timing met at 100 MHz in both builds. About 670 of those LUTs are the split itself. Most of the rest is the 16-lane requantizer the wider tile needs.

*[Scene 4.]* On a real language model, data movement wins: a token is 295 microseconds on-chip, but its weights overflow the 64-kilobyte weight buffer, so the link dominates.

Verification: a Rust golden model checks every run bit-exactly. Eleven suites are signed off, with injected mutants caught, and a pin-level system simulation passed 625 of 625 jobs. On this board: 773 GEMMs, zero mismatches.
<!-- /pitch -->

<!-- 64-kilobyte = WBUF capacity_bytes 65,536 (results/hw/lm wbuf_plan); 261,888 weight bytes per step [HW record]. 625/625 = tb/system
     KG2 and KG3 sign-off [V]. -->

---

## 8. Q&A bank (rehearse these)

Each answer takes under 30 seconds. [HW] numbers are from the 773 committed GEMM records in `results/hw`: 272 on KG1 (build 0x52A66216: H-5, the H-6 sweep, H-7 dashboard runs), 301 on KG2 (build 0x38A3950A: one check run, the H-5 re-run, 100 auto-mode runs) and 200 on KG3 (build 0x21DD6850, the live bitstream: the random batch). Same datapath RTL. Language-model runs are in `results/hw/lm/` and counted separately. If asked about anything not in those records, say "I didn't measure that".

### 8.1 Core

| Question | Answer (short) |
| --- | --- |
| Why not just batch decode requests? | Batching raises M and fixes utilization for throughput serving. It doesn't help latency-bound single-stream decode, or small per-head attention matmuls. STRIP targets small M, at the cost of more operand bandwidth. |
| What does the split cost? | Post-route: +2,243 LUTs (+43.6%), +1,590 FFs, +16 DSPs, BRAM unchanged [B]. The STRIP muxes in the array and sequencer are only +667 LUTs. Most of the rest is requant lanes 8–15, which the FULL-only build drops because FULL drains 8 lanes. Bandwidth: +25% operand bytes per cycle [S]. |
| Is the SRAM keeping up? | Yes, on-chip: `cycles_stall` was 0 on all 773 hardware runs [HW], and 0 on all 70 co-sim runs [SIM]. The BRAM ports supply 20 B/cycle, so the model assumes zero stalls [S]. With a narrower buffer or DRAM-fed weights, STRIP would stall first, because it needs 25% more bytes. |
| How do you know it's correct? | A Rust golden model checks every run bit-exactly. The invariants `mac_useful == M·N·K` and Σquadrants == total are checked on every run. Sign-off: 11 of 11 cocotb suites, with mutants caught, and a pin-level system simulation at 625 of 625 jobs for each demo bitstream [V]. Co-simulation with the real host: 70 runs, 0 mismatches [SIM]. On hardware: 773 GEMMs, 393 million MACs, 0 mismatches, plus 2,146 language-model GEMMs [HW]. |
| How accurate is the model? | On the board: exact. 0 of 9,276 counter values differ (12 counters × 773 runs, three builds), with no calibration [HW]. In simulation too, which is expected: the sequencer is deterministic and stall-free, so the model and RTL implement the same state lengths. The model's value is that it's independent code, so any disagreement is a bug. |
| Why output-stationary? | Accumulators stay in the DSP P register, there's no partial-sum traffic, and drain is simple. Weight-stationary favors weight reuse across many rows, which decode lacks. |
| Why int8? | One DSP48E1 per MAC, the same as common inference quantization practice, and the result stays bit-exact and checkable. |
| What would you do with more time? | Shadow accumulators to overlap drain. Load requant parameters during STREAM instead of a 16-cycle setup. INDEP mode for per-head attention. DDR3 weight streaming to study memory-bound decode. |
| How does this scale? | The utilization ceiling is about M over the array height, until M reaches the height. Bandwidth per MAC grows as arrays get shorter and wider. The model predicts both; show the T_tile formula (SPEC §4). |
| What was hardest? | The STRIP lower-half alignment, below. |

### 8.2 Questions the results raise

| Question | Answer (short) |
| --- | --- |
| STRIP is faster even at prefill. Isn't FULL pointless? | No. On the benchmark shapes STRIP is 1.013–1.036× at M ≥ 8 [HW], and that edge is drain, 4 rows per tile against 8. But on 31 of 97 random shapes, FULL was faster, by up to 1.32× [HW], because STRIP's 4×16 tiles pad worse. FULL also wins when bandwidth is the limit: 16 B/cycle instead of 20. I didn't build that regime (no DRAM). |
| Does auto mode help? | A little, and it's safe. On 100 random GEMMs on the board, the model picked FULL 38 times and STRIP 62, and ran FULL on all 33 shapes where it predicted STRIP slower [HW]. Measured cycles equaled its prediction on all 100. By the model, always-STRIP would have cost 1.6% more and always-FULL 5.4% more [MODEL; the un-chosen mode wasn't run]. On 125 shapes I'd measured both ways on KG1, its pick was never the slower one; 126 of 126 across all three builds [HW]. |
| Why is decode at 7% / 14%, not 12.5% / 25%? | Those are mapping ceilings [S]. Each tile also spends fixed cycles on setup (16), flush (17), and drain plus writeback. For `ffn_up` that's 115 cycles per FULL tile against 64 cycles of useful streaming, so 7.0% and 14.4% [M]. |
| Why does prefill top out around 56%? | The same per-tile overhead. Stages don't overlap, so the array idles during drain. With K = 64, the useful K cycles are about 56–58% of a tile [M]. `ffn_down` (K = 256) reaches 83.4% (FULL) and 84.5% (STRIP) [M]. |
| Your simulation matches the model exactly. Isn't that circular? | Partly, and that's why I don't lean on it. SIM == MODEL only shows the RTL implements the spec. The real test was the board: 0 counter differences on 773 runs [HW]. |
| Is the 2× just the mapping ceiling restated? | The ceiling explains the direction. The measured number includes every overhead cycle, and STRIP also halves the tile count (16 vs 32 for `ffn_up`) [M]. Measured: 2.07× for `ffn_up`, the same as the model, and 2.02–2.07× across all four matmuls [HW]. |
| Why +43.6% LUTs for a mux? | It isn't for the mux. The muxes are about 670 LUTs [B]. The FULL-only build also drops 8 of the 16 requant lanes (16 DSPs, about 1,500 LUTs). That's the honest bill for a 16-wide output tile. |
| Did splitting cost Fmax? | I can't say, and I don't claim it. Both builds meet 100 MHz: WNS +2.188 ns (FULL-only), +0.744 ns (split) [B]. The split build's worst path starts at a replicated reset flop: 0 logic levels, 95% routing. That's placement. Four builds of the same datapath closed between +0.473 and +1.143 ns [B]. |
| Zero error sounds too good. Is the model just reading the counters? | No. The model is a separate Rust crate, committed at 72da652 on Saturday at 10:10 PDT, before the board session (first hardware GEMM 13:26). It predicts from the job descriptor alone: shape, mode, requant. Zero error is what a deterministic, stall-free sequencer with on-chip SRAM should give. With DRAM it wouldn't. |
| Why does the KG3 data show a different model hash? | That hash is the last commit touching the spec or the model. The baud change edited the spec file, so the hash moved. The model's code and latencies are unchanged since 72da652, and its predictions on the repeated random batch are identical. |
| When is STRIP slower? | When it needs more tiles. STRIP tiles are 4×16 and FULL tiles are 8×8, so N = 17 pads to 32 in STRIP but only to 24 in FULL. On M = 22, K = 250, N = 17, STRIP took 3,566 cycles against 2,711 [HW]. The model predicted both, which is why auto mode works. |
| Is this a real model? | Yes: stories260K from karpathy/tinyllamas, MIT license, sha256-checked. It is tiny: dim 64, 5 layers, 8 heads with 4 KV heads, vocab 512. My float32 code reproduces its published sample text exactly. The int8 version matches it for 61 tokens, then diverges [golden model on the laptop, not board]. |
| What runs where? | On the FPGA: every linear layer, which is q, k, v, o and the three FFN matrices in each of 5 layers, plus the classifier. That's 37 GEMMs per token, int8 in, int32 out. On the laptop: embedding, RMSNorm, RoPE, attention scores and softmax, SwiGLU, quantization, and sampling. |
| Why so slow end to end? | On-chip a token step is 29,540 cycles, 295 µs [HW]. But each step sends about 245 KB over UART, because the weights don't fit in the 64 KiB weight buffer. KG2, at 115,200 baud with the default 16 ms FTDI latency timer, took 30.8 s per step. KG3, at 1,562,500 baud with the timer at 1 ms, takes 2.16 s. Both are laptop wall time, not accelerator speed. |
| So KG3 is 14× faster? | No: two things changed, the baud and the laptop's FTDI latency timer, so I don't quote a ratio. The chip itself didn't get faster; its cycle count per step is identical, 29,540 [HW]. |
| How long did the hardware runs take? | The on-chip cycles are in the counters. End to end, the 200-GEMM random batch took 49 s on KG3 (1,562,500 baud, 1 ms latency timer), and I don't report that as accelerator speed. |
| The live board's build id isn't the one in your sweep records. Why? | The sweep ran on KG1 (0x52A66216). KG2 fixed two display defaults and the unused-pin pulls; KG3 only raised the baud. The datapath RTL is identical. KG2 and KG3 each re-ran the 200 random GEMMs: same shapes, every counter identical to KG1, 0 mismatches [HW]. |
| Your LUT counts changed between builds. Which is right? | All of them: each is a separate synthesis and place-and-route run. The split build was 7,389 LUTs on KG1 and KG2 and 7,377 on KG3, which differs only in the baud divider constant [B]. I quote the cost.json build because it has a matching FULL-only twin. |
| Does the board need the laptop to load it? | No. KG3 is in the board's QSPI flash: erased, programmed and verified in 34 s. With jumper J14 fitted, the board configures itself at power-on in about 1 s; I timed that by eye. The laptop runs only the host and the dashboard. |
| Is the benchmark data real model weights? | No. The benchmark GEMMs use synthetic int8 from a fixed seed, with real LLM shapes. The language-model demo uses real weights. Utilization doesn't depend on the values, because the counters count in-bounds MACs, not nonzero values. |
| Your screen says more than 773 GEMMs? | The live counter includes every run on the board since the write-up, such as the demo you just watched. The 773 are the committed, audited records in `results/hw` [HW]. |

### 8.3 War stories (real, from the logs; pick one)

| Story | 30-second version |
| --- | --- |
| STRIP lower-half skew | The spec gave two formulas for the delay of W lanes 8–15 entering at row 4: `4 + c` in one section and `12 + c` in another. `12 + c` would have fed the lower half operands 8 cycles late. It was caught in spec review, and the spec, RTL, and tests were fixed together (D-008). The skew unit test now fails on the old formula, so it would catch a regression [V]. |
| UART pins | The vendor constraints file names the UART pins from the USB bridge's side. The schematic showed the FPGA's RX and TX had to be swapped, and I couldn't confirm that before the board. So I also prebuilt a bitstream with the pins swapped back. The schematic was right: the echo test passed with 10,000 and then 100,000 bytes, 0 mismatched, and the fallback was never used. |
| The baud wasn't the bottleneck | I raised the UART from 115,200 to 1,562,500 baud, 13.6×. The first 100 KB echo still took 25.4 s. The cause was a laptop setting, the FTDI latency timer: by default the USB-serial bridge holds a partly filled packet for up to 16 ms. Set to 1 ms, the same echo took 1.57 s. Now every wall time I quote names both settings. |
| Latency surprise | The drain-to-requant path measured 7 cycles, not the provisional 4 (D-016). Every latency lives in one spec file, so the RTL, the model, and the tests changed together, and the tests assert measured latency == the spec constant [V]. |
| The display read backwards | On the board, the 7-segment showed the build id with its digits reversed. I read off which digit is which and fixed the RTL default. Then I rebuilt the system simulation's display check from that board-confirmed rule: it fails 7 of 8 cases on the old RTL [V]. The RGB LEDs, which had glowed from a pull-up copied out of the vendor constraints, went dark in the same fix. |
| The harness that lied | Under `make -j`, cocotb could skip a simulation and still exit 0. I fixed it so every run must produce a fresh results file, and re-ran every suite [V]. |

---

## 9. Failure fallbacks (in order)

1. **Live on KG3** (1,562,500 baud; needs the 1 ms latency timer to be quick).
2. **Live on KG2** (115,200 baud), programmed over JTAG: `scripts/vivado -mode batch -source fpga/tcl/program.tcl -tclargs fpga/known_good/KG2/top.bit`, then `start_demo.ps1`, which auto-detects the rate. GEMM scenes work normally. Language-model generation takes about 30 s per step, so in Scene 4 pick the recorded story in the REPLAY view instead.
3. Board self-test (BTN0, LEDs) plus REPLAY mode of the recorded `hw` session (amber banner).
4. Backup video.

Say which mode you are in. Never present replayed or simulated data as live.

**What to say in each mode.**
- **REPLAY:** "This is a recording of Saturday's hardware runs. The board is running its self-test live." Show LED0 and LED1.
- **LM REPLAY:** "This story was generated on the board earlier. It's a recording, and replay speed is not board speed." Then say the build, baud and GEMM count shown on screen; don't quote numbers from memory.
- **SIM only:** use it only if the booth board fails: "these numbers are from co-simulation of the same RTL, labeled SIM." Never round SIM into HW. The SIM numbers you may say: `ffn_up` M=1 FULL 6.95% → STRIP 14.40%, and M=64 55.65% / 57.65%, from 70 co-sim runs with every counter equal to the model [SIM].
- **Board dead:** give the self-test and video pitch. The numbers come from the README tables with their source labels.

**Storyteller pre-judging checks** (in addition to the §6 booth checklist):
- [ ] `scripts/wsl 'python results/plots/make_plots.py --check'`: plots regenerated from the committed records; the source line under each plot lists records per build (KG1 52a66216, KG2 38a3950a, KG3 21dd6850).
- [ ] No HW placeholder left: `grep -c "$(printf '\342\237\246')HW" README.md docs/devpost.md docs/DEMO.md` prints 0 for all three (the printf emits the opening placeholder bracket, so this line doesn't count itself). Only HUMAN placeholders remain (table number, URLs, hero photo); fill them before submitting.
- [ ] Every spoken number has been re-read from `results/` that morning. If new HW records were committed overnight, the 773 / 393,129,309 / 9,276 counts change: recount with `git ls-files 'results/hw/*.json'`.
- [ ] Pitch word counts (each ≤ 280): `awk '/<!-- pitch:/{f=1;n=$0;next}/<!-- \/pitch -->/{print n, c; c=0; f=0} f{c+=NF}' docs/DEMO.md`
- [ ] Each pitch has been rehearsed once against a skeptical-engineer Q&A from §8.2.
