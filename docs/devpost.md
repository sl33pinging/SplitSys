# Devpost submission: SplitSys (paste-ready)

<!--
Owner: demo-storyteller. red-team-reviewer audits before submission.
Full per-number derivations: README.md HONESTY KEY (same sources, same numbers). Summary:
[M] results/model/sweep.json @ model_git_hash 72da65248adf (MODEL);
[SIM] results/sim/*.json top level (not archive_dirty/): 70 co-sim records, all counters == model; NOT hardware;
[B] results/cost.json, committed 23acf6a: S4 freeze rebuild, git 38a3950a (split build = KG2):
      full_only/split lut 5146/7389; delta.total lut 2243 (43.6%), dsp 16; array 378 + sequencer 289 = 667 LUT.
      KG3 own report fpga/reports/kg3_top/summary.json: lut 7377, ff 7928, dsp 96, bram 26, wns 0.473 (not used for deltas).
[V] tb/SIGNOFF.md (11/11, 64/64; KG3 candidate 11/11, 66/66), tb/system/SIGNOFF_SYSTEM.md (617/617 at f402cc7; 625/625 KG2 + KG3).
Public source for "splittable systolic array" (Inspiration): https://matx.com/research/series_b, fetched 2026-10-24.
[HW] results/hw/*.json top level, git-tracked only: 773 GEMM records, source "hw", all golden_match, 0 mismatches,
      3,099,658 outputs, mac_useful sum 393,129,309, 0 of 9,276 counter values != model block, cycles_stall 0.
      KG1 52a66216: 272 (H-5 200 + H-6 56 + H-7 16). KG2 38a3950a: 301 (1 check + H-5 200 + auto 100).
      KG3 21dd6850: 200 (Q-5 random both). H-5 counters on KG2 and KG3 == KG1 on 200/200.
      ffn_up M=1: 20-40-41Z util 0.069527 / 0.143982 (2.0709); M=1 range 2.0247..2.0709; M=64 0.556484 / 0.576536.
      H-5 pairs 97: STRIP more cycles on 31 (max 3566/2711 = 1.315x, M=22 K=250 N=17).
      AUTO (results/hw/logs/KG2/G5_auto_random100_seed1.log): FULL 38 / STRIP 62; 33 predicted-STRIP-slower all FULL;
        measured == model 100/100; MODEL always-FULL 1.054x, always-STRIP 1.016x. Pick-check: 125/125 shapes on KG1
        (126/126 pooled across all three builds; KG2 H-5 97/97, KG3 97/97; recomputed, matches red-team G5).
      LM results/hw/lm/ (3 tracked): KG2 8 tok 444/444, KG2 30 tok 1258/1258, KG3 8 tok 444/444 bit-exact and
        counters == model; 29,540 cycles/step (295 us); wall 30.76 s/step (KG2, 115200, FTDI timer 16 ms),
        2.16 s/step (KG3, 1562500, 1 ms) per fpga/known_good/KG3/archive.txt lead_note.
        int8 vs f32: sw/crates/lm/tests/stories260k.rs int8_vs_f32_agreement: first divergent index 62 with BOS = 0
        -> 61 identical generated tokens; f32_greedy_reproduces_published_readme passes.
      Flash: results/hw/logs/KG2/H-8_flash_program.log, results/hw/logs/KG3/Q-8_flash_program.log ("ok in 34 s",
        FLASH_OK). Boot ~1 s: human (STATUS ~16:50, 22:08). Dashboard 2.1 s wall: STATUS 22:19 (laptop-side).
Tagline (176 chars, framing approved by human via lead, docs/WINNERS_RESEARCH.md §3 option A): "twice as many multipliers
        busy" = utilization ratio at M=1, util = mac_useful/(64*cycles_total) = mean fraction of the 64 MACs doing useful
        work; 2.0247..2.0709 [HW]. Verified 20-40-41Z ffn_up m1: 16384/(64*3682) = 0.069527, 16384/(64*1778) = 0.143982.
        "1 of 8 rows" / 12.5% = SPEC mapping ceiling at M=1 [S]. "2.16 s per step" = KG3 LM record [HW, laptop wall].
        "125 ... (126 across all three)" = pick-check above. "LLM" = Llama-architecture model; size disclosed as tiny.
[S] docs/SPEC.md constants. ⟦HUMAN: …⟧ = placeholder for the human; never guess.
Before pasting: strip these HTML comments and the [M]/[B]/[V]/[S]/[HW] tags if Devpost renders them
oddly. Keep the words "model" / "measured" in the prose, though.
Deadline: Devpost with a valid table number by Sun 09:00 (PLAN.md).
Voice: solo entry, first person singular. The "we" in four section headings is Devpost's fixed field labels; keep them.
-->

**Project name:** SplitSys

**Tagline (≤ 200 chars):** I built a tiny AI chip on an FPGA that rewires itself to keep twice as many multipliers busy while a language model writes one word at a time, and it's running a real LLM live.

**Table number:** ⟦HUMAN: table number (cannot change after Sun 09:00)⟧
**Team:** solo entry (one person); eligible for Best Solo Hack.

SplitSys is an 8×8 int8 systolic-array accelerator I designed and built solo on a Spartan-7 FPGA. In LLM decode (one token at a time, M = 1), the 8×8 array can fill only 1 of its 8 rows, a 12.5% mapping ceiling (measured: 6.95% for `ffn_up`), so SplitSys rewires itself into a 4×16 "STRIP" shape and measures **2.02–2.07× higher utilization** [HW].
A cycle model I wrote before the board session predicted **all 12 hardware counters exactly on 773 GEMMs** run on the board [HW], and the board runs the public stories260K model with **every linear layer on the FPGA**, live, at 2.16 s per step end to end (mostly the serial link, not the chip) [HW].

**Limits, up front:** attention, softmax and normalisation run on the laptop. stories260K is a tiny model. The gain is utilization inside the array, measured in on-chip cycles, not a speed claim against a CPU or GPU. STRIP isn't always better, so a cycle model picks the mode per matrix; on 125 shapes measured both ways on the first build (126 across all three) it never picked the slower one [HW].

---

## Inspiration

LLM accelerators are built around big systolic arrays. During decode a model generates one token at a time, so the matrix multiplies are one row tall (M = 1). An 8-row array can then keep at most 1 of its 8 rows busy. Some LLM-specific chip makers have publicly described splittable systolic arrays as a response. I wanted to measure that tradeoff myself at small scale, with hardware counters rather than slides, and see whether a simple cycle model could predict every counter.

## What it does

SplitSys is a GEMM accelerator on a RealDigital Urbana board (AMD Spartan-7 XC7S50).

- **Two shapes from one array.** It has 64 int8 processing elements, output-stationary, one DSP slice each. Per job, the array runs either as one 8×8 tile (FULL) or, by muxing at row 4, as a 4×16 tile (STRIP). Switching modes needs no reconfiguration.
- **Bit-exact checking.** A Rust host sends LLM-shaped matmuls over UART and checks every output bit-exactly against a Rust golden model.
- **Counters and a model.** Twelve hardware counters report cycles per phase, useful MACs per quadrant, and tiles. A cycle model predicts every one of them, and can pick the faster shape per job.
- **A real language model on the board.** Every linear layer of stories260K, a small public model, runs on the FPGA.
- **A live dashboard** shows the PE grid lighting up, model-vs-measured utilization, and the language model generating text.

**Results:**
- **Decode (M = 1), measured on the board's counters:**
  - `ffn_up` runs at 6.95% utilization in FULL and 14.40% in STRIP, which is 2.07× [HW].
  - Across all four matmuls of a transformer block, the gain is 2.02–2.07× [HW].
  - These are on-chip cycle counts, not wall-clock time.
- **The model was exact.** A cycle model, frozen before the hardware session, predicted all 12 counters on all 773 hardware GEMMs with 0 error. Not one of the 9,276 counter values differed [HW]. No calibration was applied.
- **Prefill (M = 64):** both modes fill the array. Measured: 55.65% FULL vs 57.65% STRIP for `ffn_up` [HW]. The small STRIP edge comes from a shorter drain (4 rows per tile instead of 8), not from the split.
- **STRIP isn't always better.** On 31 of 97 random shapes run in both modes, STRIP took more cycles, by up to 1.32× [HW]. Each of those shapes needs more STRIP tiles. The model predicted those exactly too.
- **So the model picks the mode.** In auto mode, on 100 random GEMMs on the board, the model chose FULL 38 times and STRIP 62 times. On all 33 shapes where it predicted STRIP would be slower, it ran FULL, and measured cycles equaled its prediction on 100 of 100 [HW]. By the model, always-FULL would have taken 1.054× as long and always-STRIP 1.016× [MODEL]. That is a small gain on this mix, but it never takes the slow-shape penalty. Checked against measurements: on 125 shapes measured both ways on the first build (126 across all three), the model's pick was never the slower one [HW].
- **A real model, on the board.** stories260K (karpathy/tinyllamas, MIT) runs with all 37 linear-layer GEMMs per token on the FPGA, int8 × int8 → int32. Embedding, RMSNorm, RoPE, attention softmax, SwiGLU and sampling run on the laptop.
  - Three runs on two bitstreams: 2,146 GEMMs, all bit-exact, with every counter equal to the model [HW].
  - The 30-token run wrote: *"Once upon a time, there was a little girl named Lily. She loved to play outside in the park. One day,"*
  - On-chip, a token step takes 29,540 cycles (295 µs) [HW].
  - End to end it takes 2.16 s per step on the laptop (1,562,500 baud, FTDI latency timer 1 ms), down from 30.8 s on the earlier bitstream (115,200 baud, 16 ms timer). Most of that is UART traffic: about 245 KB per step, mostly weights that don't fit in the 64 KiB weight buffer. Wall time is not accelerator speed.
- **Cost:** the split costs 25% more operand bandwidth per cycle (20 vs 16 bytes) [S] and, measured post-route, +2,243 LUTs (+43.6%) and +16 DSPs [B]. Most of that is the 16-lane output path. The muxing itself is 667 LUTs [B].
- **Verified on hardware:** 773 GEMMs, 393,129,309 MACs, 3,099,658 outputs compared, 0 mismatches, on three bitstreams with the same datapath [HW]. The language-model GEMMs are counted separately.

## How we built it

- **Spec first.** One spec (`spec/spec.toml`) generates both the SystemVerilog package and the Rust constants, so RTL and software cannot drift apart. Every ambiguity was resolved in a numbered decision log.
- **Rust:** a bit-exact golden model (GEMM, requantization, packing, CRC-8, Fletcher-32), a test-vector generator, a cycle model that predicts all 12 counters, the host (UART driver, benchmark runner, auto mode, dashboard server), and an int8 port of stories260K whose linear layers call the board.
- **SystemVerilog RTL:** PE, skew units, array with FULL/STRIP modes, BRAM buffers, sequencer, requantizer, counters, UART packet layer, self-test, and board I/O.
- **Verification:** cocotb + Verilator unit suites with expected values only from Rust, plus a system simulation. In co-simulation, the real host binary talks to the simulated RTL over a TCP-to-UART bridge.
- **Builds:** Vivado 2025.2.1 builds, timing-clean at 100 MHz. A second build with STRIP compiled out measures what the split costs.
- **Order of work:**
  - Everything was simulated first.
  - Then came one guided hardware session, following a written step-by-step checklist: UART echo, a 10,000-frame memory test (5,123,818 bytes, 0 errors), the on-chip self-test, 200 random GEMMs, and the benchmark sweep. No step needed a datapath change or a model calibration.
  - Bring-up found two board-I/O quirks: the RGB LEDs glowed (unused pins pulled up, a setting copied from the vendor constraints file), and the 7-segment display showed its text in reversed digit order. Neither touches the datapath, and one re-spin (KG2) fixed both.
  - KG2 ran the language model at 115,200 baud. To cut the UART time I raised the baud to 1,562,500, a rate the board's clock divider hits exactly. That build (KG3) was signed off again in simulation and re-qualified on the board (echo, self-test, memory test, 200 random GEMMs, language model) before it went into flash as the demo bitstream.

## Challenges we ran into

- **A spec bug, caught before silicon.** The first spec had two formulas for the delay of the weights injected at row 4. One gave `4 + c` cycles, the other `12 + c`. It was caught in spec review, and the spec, RTL, and tests were fixed together (decision D-008). The skew unit test now fails on the old formula. With the wrong delay, the lower half of the array would have multiplied misaligned operands.
- **UART pins named from the other chip's side.** The vendor constraints file names the UART pins from the USB bridge's point of view. Reading the schematic showed the FPGA's RX and TX had to be swapped. I couldn't confirm that before the board, so I also prebuilt a bitstream with them swapped back. The schematic reading was right: the UART echo test passed with 10,000 and then 100,000 bytes, 0 mismatched, and the fallback was never needed.
- **Latency assumptions.** The drain-to-requant path measured 7 cycles, not the provisional 4 (D-016). The constant lives in one place, so the model, RTL, and tests all moved together.
- **A test harness that lied.** Under `make -j`, cocotb could skip a simulation and still exit 0. I caught it, fixed it so every run must produce fresh results, and re-ran every suite.
- **The baud wasn't the whole bottleneck.** After the baud went up 13.6×, a 100 KB echo still took 25.4 s. The cause was on the laptop: the FTDI driver's 16 ms latency timer. Set to 1 ms, the same echo took 1.57 s. So I always quote both settings next to any wall time.
- **No datapath surprise on hardware.** The first bit-exact hardware run matched the model on every counter, so there was nothing to reproduce in simulation. The surprises were board I/O: LEDs glowing from a pull-up setting copied out of the vendor constraints file, and display digits in reverse order. I read off on the board which display digit is which and fixed the RTL default. Then I rebuilt the system simulation's display check from that board-confirmed rule: it fails 7 of 8 cases on the old RTL and passes on the fix.

## Accomplishments that we're proud of

- **Verification in simulation:**
  - 11 of 11 cocotb suites in eligible sign-off runs (64 of 64 tests, and 66 of 66 for the faster-UART build), including 262 directed jobs on the integrated core [V]. The skew, counters, packet, array and core suites record deliberate mutants, or a sensitivity check against the old RTL, and each one was caught [V].
  - A pin-level system simulation passed 617 of 617 jobs, and later 625 of 625 for each demo bitstream [V].
  - 70 co-simulation runs of the real host against the simulated chip (17.0 million MACs checked): 0 output mismatches, and every counter equal to the cycle model [SIM].
  - Matching the model in simulation is expected, since both use the same latencies. The real test was hardware, and the model passed it: 0 error on all 12 counters, across 773 board runs [HW].
- **On the board:** 773 GEMMs and 393.1 million MACs bit-exact, a measured 2.02–2.07× decode utilization gain, and zero stall cycles [HW].
- **A real language model** with every linear layer on my array, bit-exact on 2,146 of 2,146 GEMMs [HW].
- **It boots itself.** The demo bitstream (KG3) is in the board's QSPI flash (Micron MT25QL128): erased, programmed and verified in 34 s. On power-on the board configures itself from flash in about 1 s (observed by eye), with no Vivado or JTAG needed. One script detects the baud and brings up the dashboard in 2.1 s of laptop wall time. That is startup time, not accelerator speed.
- **Cost measured, not guessed,** including the uncomfortable part: most of the area goes to the wider output path, not to the split itself [B].
- **A live, offline dashboard** in which every number carries its source label (HW, SIM, or MODEL).

## What we learned

- **Utilization is set by mapping first, overhead second.** At M = 1, the mapping caps FULL at 12.5% and STRIP at 25% [S]. Fixed per-tile setup, flush, and drain cycles then bring the model down to 7.0% and 14.4% [M].
- **Flexibility costs bandwidth.** A shorter, wider array needs more operand bytes per MAC: +25% here [S].
- **The cost of a feature hides in its neighbours.** Most of the split's area was the output path that STRIP's 16-wide tiles need, not the muxes.
- **A cycle model is only useful if it is written independently** and compared counter by counter. Agreeing with your own simulation proves the RTL matches the spec. Only the board can show the model predicts real hardware. This one did, exactly, because the design is deterministic and has no DRAM. With DRAM-fed weights, stalls would make that much harder.
- **Shape beats mode.** STRIP wins on most shapes, but its 4×16 tile padding made it slower on 31 of 97 random shapes. A good mode selector has to count tiles, and an exact model can do that.
- **On a real model, data movement dominates.** A token step needs 295 µs on-chip, but its weights don't fit in the 64 KiB weight buffer, so most of the time goes to moving about 245 KB per step over the link.

## What's next for SplitSys

- Shadow accumulators, to overlap drain with the next tile.
- Loading requant parameters during streaming.
- INDEP mode: four independent 4×4 jobs, for per-head attention, which would also move attention onto the board.
- DDR3 weight streaming, so the language model's weights stay next to the array and memory-bound decode can be studied. There FULL's lower bandwidth need could make it the right choice, and auto mode would need a bandwidth term.

## Built with

Rust · SystemVerilog · Verilator · cocotb · Python · matplotlib · Vivado 2025.2.1 · AMD Spartan-7 XC7S50 (RealDigital Urbana) · vanilla JavaScript / SVG · karpathy/tinyllamas stories260K (MIT) · Claude Code (AI coding agents)

**Links:** ⟦HUMAN: repo URL⟧ · ⟦HUMAN: demo video URL⟧
**Hero image:** ⟦HUMAN: hero photo⟧
