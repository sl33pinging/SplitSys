# Devpost submission: SplitSys (paste-ready)

<!--
Owner: demo-storyteller. red-team-reviewer audits before submission.
Sources: [M] results/model/sweep.json @ model_git_hash 72da65248adf (MODEL);
[SIM] results/sim/*.json top level (not archive_dirty/): 70 co-sim records (results/sim/session.json gemms_run 70,
      macs_verified 17014784, mismatches 0): 66 at build 0f402cc7 / git f402cc7 (committed e2cea28) + 4 scripted-demo
      at build 05900507 / git 5900507 (committed fb89e37), same RTL; all counters == model; NOT hardware;
[B] results/cost.json, committed 8ccb333: S4 freeze builds, build_id 0x52A66216 (BUILD, Vivado post-route):
      delta.total lut 2247 (2247/5142 = 43.7%), dsp 16; array 376 + sequencer 290 = 666 LUT;
[V] tb/SIGNOFF.md (11/11, 64/64), tb/system/SIGNOFF_SYSTEM.md (617/617 eligible at f402cc7), DECISIONS D-016/D-017;
Public source for "splittable systolic array" (Inspiration): https://matx.com/research/series_b
  ("The MatX One chip is based on a splittable systolic array ... high utilization on smaller matrices
  with flexible shapes"), fetched 2026-10-24.
[HW] results/hw/*.json top level (source "hw"): 256 run records from one board session (COM3, Sat 13:00-13:42),
      build_id 52a66216, model_git_hash 72da65248adf: 200 from H-5 `run --random 100 --mode both --seed 1`
      (committed e180386) + 56 from H-6 `bench sweep --seed 1` (committed 2bacace). Derived from the records:
      golden_match 256/256, mismatches 0, 952,012 outputs compared, mac_useful sum 118,801,362, invariants_ok 256/256,
      0 of 3,072 counter values != model, cycles_stall 0 everywhere. results/session.json agrees (256 / 118801362 / 0).
      ffn_up M=1: 2026-10-24T20-40-41Z-ffn_up-m1-full.json util 0.069527, -strip 0.143982 (ratio 2.0709).
      M=1 ratio range over 4 GEMMs 2.0247 (ffn_down) .. 2.0709 (ffn_up). M=64 ffn_up 0.556484 / 0.576536.
      H-5 shape-matched pairs: 97; STRIP more cycles on 31 (max 3566/2711 = 1.315x, M=22 K=250 N=17), all with more
      STRIP tiles. Logs: results/hw/logs/H-3..H-6. H-2 UART echo (10,000 and 100,000 bytes, 0 mismatched): docs/STATUS.md
      Phase H log only (no file). KG1: fpga/known_good/KG1/summary.json build_id 0x52A66216.
[S] docs/SPEC.md v1.1 constants. ⟦HUMAN: …⟧ = placeholder for the human; never guess.
Before pasting: strip these HTML comments and the [M]/[B]/[V]/[S] tags if Devpost renders them
oddly. Keep the words "model" / "measured" in the prose, though.
Deadline: Devpost with a valid table number by Sun 09:00 (PLAN.md).
-->

**Project name:** SplitSys

**Tagline (≤ 200 chars):** An 8×8 int8 systolic array on a Spartan-7 FPGA that rewires into 4×16 for LLM decode: 2.07× the decode utilization, measured on-chip, and a cycle model that predicted every counter.

**Table number:** ⟦HUMAN: table number (cannot change after Sun 09:00)⟧
**Team / Best Solo Hack eligibility:** ⟦HUMAN: team size; solo yes/no⟧

---

## Inspiration

LLM accelerators are built around big systolic arrays. During decode a model generates one token at a time, so the matrix multiplies are one row tall (M = 1). An 8-row array can then keep at most 1 of its 8 rows busy. Some LLM-specific chip makers have publicly described splittable systolic arrays as a response. We wanted to measure that tradeoff ourselves at small scale, with hardware counters rather than slides, and see whether a simple cycle model could predict every counter.

## What it does

SplitSys is a GEMM accelerator on a RealDigital Urbana board (AMD Spartan-7 XC7S50).

- **Two shapes from one array.** It has 64 int8 processing elements, output-stationary, one DSP slice each. Per job, the array runs either as one 8×8 tile (FULL) or, by muxing at row 4, as a 4×16 tile (STRIP). Switching modes needs no reconfiguration.
- **Bit-exact checking.** A Rust host sends LLM-shaped matmuls over UART: the four matmuls of one transformer block, at decode and prefill sizes. It checks every output bit-exactly against a Rust golden model.
- **Counters and a model.** Twelve hardware counters report cycles per phase, useful MACs per quadrant, and tiles. A cycle model predicts every one of them.
- **A live dashboard** shows the PE grid lighting up and model-vs-measured utilization.

**Results:**
- **Decode (M = 1), measured on the board's counters:**
  - `ffn_up` runs at 6.95% utilization in FULL and 14.40% in STRIP, which is 2.07× [HW].
  - Across all four matmuls of the block, the gain is 2.02–2.07× [HW].
  - These are on-chip cycle counts, not wall-clock time.
- **The model was exact.** A cycle model, frozen before the hardware session, predicted all 12 counters on all 256 hardware GEMMs with 0 error. Not one of the 3,072 counter values differed [HW]. No calibration was applied.
- **Prefill (M = 64):** both modes fill the array. Measured: 55.65% FULL vs 57.65% STRIP for `ffn_up` [HW]. The small STRIP edge comes from a shorter drain (4 rows per tile instead of 8), not from the split.
- **STRIP isn't always better.** On 31 of 97 random shapes run in both modes, STRIP took more cycles, by up to 1.32× [HW]. Each of those shapes needs more STRIP tiles (STRIP tiles are 4×16, FULL tiles 8×8, so the padding differs). The model predicted those exactly too.
- **Cost:** the split costs 25% more operand bandwidth per cycle (20 vs 16 bytes) [S] and, measured post-route, +2,247 LUTs (+43.7%) and +16 DSPs [B]. Most of that is the 16-lane output path. The muxing itself is 666 LUTs [B].
- **Verified on hardware:** 256 GEMMs (200 random up to 64×256×256 in both modes, plus a 56-run benchmark sweep), 118,801,362 MACs, 952,012 outputs compared, 0 mismatches [HW].

## How we built it

- **Spec first.** One spec (`spec/spec.toml`) generates both the SystemVerilog package and the Rust constants, so RTL and software cannot drift apart. Every ambiguity was resolved in a numbered decision log.
- **Rust:** a bit-exact golden model (GEMM, requantization, packing, CRC-8, Fletcher-32), a test-vector generator, a cycle model that predicts all 12 counters, and the host (UART driver, benchmark runner, dashboard server).
- **SystemVerilog RTL:** PE, skew units, array with FULL/STRIP modes, BRAM buffers, sequencer, requantizer, counters, UART packet layer, self-test, and board I/O.
- **Verification:** cocotb + Verilator unit suites with expected values only from Rust, plus a system simulation. In co-simulation, the real host binary talks to the simulated RTL over a TCP-to-UART bridge.
- **Builds:** Vivado 2025.2.1 builds, timing-clean at 100 MHz. A second build with STRIP compiled out measures what the split costs.
- **Order of work:** everything was simulated first. Then came one guided hardware session, following a written step-by-step checklist: UART echo, a 10,000-frame memory test (5,123,818 bytes, 0 errors), the on-chip self-test, 200 random GEMMs, and the benchmark sweep. No step needed a datapath change, a re-spin, or a model calibration. Bring-up did find two board-I/O quirks in this bitstream: the RGB LEDs glow (unused pins pulled up, a setting copied from the vendor constraints file), and the 7-segment display shows its text in reversed digit order. Neither touches the datapath. Both fixes are in the next build candidate, which hasn't run on hardware yet.

## Challenges we ran into

- **A spec bug, caught before silicon.** The first spec had two formulas for the delay of the weights injected at row 4. One gave `4 + c` cycles, the other `12 + c`. It was caught in spec review, and the spec, RTL, and tests were fixed together (decision D-008). The skew unit test now fails on the old formula. With the wrong delay, the lower half of the array would have multiplied misaligned operands.
- **UART pins named from the other chip's side.** The vendor constraints file names the UART pins from the USB bridge's point of view. Reading the schematic showed the FPGA's RX and TX had to be swapped. We couldn't confirm that before the board, so we also prebuilt a bitstream with them swapped back. The schematic reading was right: the UART echo test passed with 10,000 and then 100,000 bytes, 0 mismatched, and the fallback was never needed.
- **Latency assumptions.** The drain-to-requant path measured 7 cycles, not the provisional 4 (D-016). The constant lives in one place, so the model, RTL, and tests all moved together.
- **A test harness that lied.** Under `make -j`, cocotb could skip a simulation and still exit 0. We caught it, fixed it so every run must produce fresh results, and re-ran every suite.
- **No datapath surprise on hardware.** The first bit-exact hardware run matched the model on every counter, so there was nothing to reproduce in simulation. The surprises were board I/O: LEDs glowing from a pull-up setting copied out of the vendor constraints file, and display digits in reverse order. We read off on the board which display digit is which and fixed the RTL default. Then we rebuilt the system simulation's display check from that board-confirmed rule: it fails 7 of 8 cases on the old RTL and passes on the fix.

## Accomplishments that we're proud of

- **Verification in simulation:**
  - 11 of 11 cocotb suites and 64 of 64 tests in an eligible sign-off run, including 262 directed jobs on the integrated core [V]. Deliberately injected RTL bugs were caught in the skew, counters, packet, array and core suites [V].
  - A pin-level system simulation passed 617 of 617 jobs in an eligible sign-off run [V].
  - 70 co-simulation runs of the real host against the simulated chip (17.0 million MACs checked): 0 output mismatches, and every counter equal to the cycle model [SIM].
  - Matching the model in simulation is expected, since both use the same latencies. The real test was hardware, and the model passed it: 0 error on all 12 counters, across 256 board runs [HW].
- **On the board:** 256 GEMMs and 118.8 million MACs bit-exact, a measured 2.02–2.07× decode utilization gain, and zero stall cycles [HW].
- **Cost measured, not guessed,** including the uncomfortable part: most of the area goes to the wider output path, not to the split itself [B].
- **A live, offline dashboard** in which every number carries its source label (HW, SIM, or MODEL).

## What we learned

- **Utilization is set by mapping first, overhead second.** At M = 1, the mapping caps FULL at 12.5% and STRIP at 25% [S]. Fixed per-tile setup, flush, and drain cycles then bring the model down to 7.0% and 14.4% [M].
- **Flexibility costs bandwidth.** A shorter, wider array needs more operand bytes per MAC: +25% here [S].
- **The cost of a feature hides in its neighbours.** Most of our split's area was the output path that STRIP's 16-wide tiles need, not the muxes.
- **A cycle model is only useful if it is written independently** and compared counter by counter. Agreeing with your own simulation proves the RTL matches the spec. Only the board can show the model predicts real hardware. Ours did, exactly, because the design is deterministic and has no DRAM. With DRAM-fed weights, stalls would make that much harder.
- **Shape beats mode.** STRIP wins on most shapes, but its 4×16 tile padding made it slower on 31 of 97 random shapes. A good mode selector has to count tiles.

## What's next for SplitSys

- Shadow accumulators, to overlap drain with the next tile.
- Loading requant parameters during streaming.
- INDEP mode: four independent 4×4 jobs, for per-head attention.
- DDR3 weight streaming, to study memory-bound decode, where FULL's lower bandwidth need could make it the right choice.
- Automatic mode selection, scored by the cycle model, which matched every hardware counter.

## Built with

Rust · SystemVerilog · Verilator · cocotb · Python · matplotlib · Vivado 2025.2.1 · AMD Spartan-7 XC7S50 (RealDigital Urbana) · vanilla JavaScript / SVG

**Links:** ⟦HUMAN: repo URL⟧ · ⟦HUMAN: demo video URL⟧
