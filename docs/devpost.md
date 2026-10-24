# Devpost submission: SplitSys (paste-ready)

<!--
Owner: demo-storyteller. red-team-reviewer audits before submission.
Sources: [M] results/model/sweep.json @ model_git_hash 72da65248adf (MODEL);
[SIM] results/sim/*.json committed at f70dec9 (46 co-sim records, all counters == model; NOT hardware);
[B] results/cost.json @ git 8bbd7ba6 (BUILD, Vivado post-route);
[V] tb/SIGNOFF.md (11/11, 64/64), tb/system/SIGNOFF_SYSTEM.md (617/617), DECISIONS D-016/D-017;
[S] docs/SPEC.md v1.1 constants. ⟦…⟧ = placeholder: fill from results/ only, never guess.
Before pasting: strip these HTML comments and the [M]/[B]/[V]/[S] tags if Devpost renders them
oddly. Keep the words "model" / "measured" in the prose, though.
Deadline: Devpost with a valid table number by Sun 09:00 (PLAN.md).
-->

**Project name:** SplitSys

**Tagline (≤ 200 chars):** An 8×8 int8 systolic array on a Spartan-7 FPGA that rewires itself into 4×16 for skinny LLM decode matmuls. Hardware counters, a cycle model, and the measured cost of the split.

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
- **Decode (M = 1), `ffn_up`:**
  - The model predicts utilization of 7.0% in FULL and 14.4% in STRIP, which is 2.07× [M].
  - Across all four matmuls the model predicts 2.02–2.07× [M].
  - Co-simulation of the RTL, driven by the real host software, gives the same 6.95% → 14.40% [SIM]. That's simulation, not silicon.
  - Measured on the board: ⟦HW: ffn_up M=1 FULL util⟧ → ⟦HW: ffn_up M=1 STRIP util⟧, with model error ⟦HW: max per-counter error⟧.
- **Prefill (M = 64):** both modes fill the array. The model predicts 55.6% vs 57.7% [M]. The small STRIP edge comes from a shorter drain (4 rows per tile instead of 8), not from the split.
- **Cost:** the split costs 25% more operand bandwidth per cycle (20 vs 16 bytes) [S] and, measured post-route, +2,239 LUTs (+43%) and +16 DSPs [B]. Most of that is the 16-lane output path. The muxing itself is about 663 LUTs [B].
- **Verified on hardware:** ⟦HW: GEMMs verified⟧ GEMMs, ⟦HW: MACs verified⟧ MACs, ⟦HW: mismatches⟧ mismatches.

## How we built it

- **Spec first.** One spec (`spec/spec.toml`) generates both the SystemVerilog package and the Rust constants, so RTL and software cannot drift apart. Every ambiguity was resolved in a numbered decision log.
- **Rust:** a bit-exact golden model (GEMM, requantization, packing, CRC-8, Fletcher-32), a test-vector generator, a cycle model that predicts all 12 counters, and the host (UART driver, benchmark runner, dashboard server).
- **SystemVerilog RTL:** PE, skew units, array with FULL/STRIP modes, BRAM buffers, sequencer, requantizer, counters, UART packet layer, self-test, and board I/O.
- **Verification:** cocotb + Verilator unit suites with expected values only from Rust, plus a system simulation. In co-simulation, the real host binary talks to the simulated RTL over a TCP-to-UART bridge.
- **Builds:** Vivado 2025.2.1 builds, timing-clean at 100 MHz. A second build with STRIP compiled out measures what the split costs.
- **Order of work:** everything was simulated first. Then came one guided hardware session, following a written step-by-step checklist. ⟦HW: confirm the session happened as planned; note any step that needed a re-spin⟧

## Challenges we ran into

- **A spec bug, caught before silicon.** The first spec had two formulas for the delay of the weights injected at row 4. One gave `4 + c` cycles, the other `12 + c`. A smoke test caught the mismatch, and the spec, RTL, and tests were fixed together (decision D-008). With the wrong delay, the lower half of the array would have multiplied misaligned operands.
- **UART pins named from the other chip's side.** The vendor constraints file names the UART pins from the USB bridge's point of view. Reading the schematic showed the FPGA's RX and TX had to be swapped. We also prebuilt a bitstream with them swapped back, in case that reading was wrong.
- **Latency assumptions.** The requantizer pipeline measured 7 cycles, not the provisional 4 (D-016). The constant lives in one place, so the model, RTL, and tests all moved together.
- **A test harness that lied.** Under `make -j`, cocotb could skip a simulation and still exit 0. We caught it, fixed it so every run must produce fresh results, and re-ran every suite.
- ⟦HW: the real hardware surprise, if any, and how it was reproduced in simulation⟧

## Accomplishments that we're proud of

- **Verification in simulation:**
  - 11 of 11 cocotb suites and 64 of 64 tests in an eligible sign-off run, including 262 directed jobs on the integrated core [V]. The unit suites catch deliberately injected RTL bugs.
  - A pin-level system simulation passed 617 of 617 jobs [V].
  - 46 co-simulation runs of the real host against the simulated chip: 0 output mismatches, and every counter equal to the cycle model [SIM].
  - Matching the model in simulation is expected, since both use the same latencies. The real test is hardware: ⟦HW: model error per counter⟧.
- **Cost measured, not guessed,** including the uncomfortable part: most of the area goes to the wider output path, not to the split itself [B].
- **A live, offline dashboard** in which every number carries its source label (HW, SIM, or MODEL).

## What we learned

- **Utilization is set by mapping first, overhead second.** At M = 1, the mapping caps FULL at 12.5% and STRIP at 25% [S]. Fixed per-tile setup, flush, and drain cycles then bring the model down to 7.0% and 14.4% [M].
- **Flexibility costs bandwidth.** A shorter, wider array needs more operand bytes per MAC: +25% here [S].
- **The cost of a feature hides in its neighbours.** Most of our split's area was the output path that STRIP's 16-wide tiles need, not the muxes.
- **A cycle model is only useful if it is written independently** and compared counter by counter. Agreeing with your own simulation proves the RTL matches the spec. Only the board can show the model predicts real hardware.

## What's next for SplitSys

- Shadow accumulators, to overlap drain with the next tile.
- Loading requant parameters during streaming.
- INDEP mode: four independent 4×4 jobs, for per-head attention.
- DDR3 weight streaming, to study memory-bound decode, where FULL's lower bandwidth need could make it the right choice.
- Automatic mode selection.

## Built with

Rust · SystemVerilog · Verilator · cocotb · Python · matplotlib · Vivado 2025.2.1 · AMD Spartan-7 XC7S50 (RealDigital Urbana) · vanilla JavaScript / SVG

**Links:** ⟦HUMAN: repo URL⟧ · ⟦HUMAN: demo video URL⟧
