# SplitSys: a splittable int8 systolic array for LLM matmuls, on a Spartan-7 FPGA

<!--
HONESTY KEY (red-team: audit every number in this file against this key).
  [M]  MODEL  results/model/sweep.json + roofline.json, model_git_hash 72da65248adf
              (latencies confirmed by RTL, D-017). Values also in results/plots/manifest.json.
  [SIM] SIM   results/sim/*.json committed at f70dec9: 46 per-run records (Verilator co-sim of the
              real top, driven through its UART pins by the real splitsys-host), builds 034cbc27,
              0b53783d, 03f3b9d1. Every record: golden_match, 0 mismatches, invariants_ok, all 12
              counters == model. SIM is NOT hardware.
  [B]  BUILD  results/cost.json (source "hw-build"), Vivado 2025.2.1 post-route, git 8bbd7ba6.
  [V]  VERIF  tb/SIGNOFF.md (11/11 suites, 64/64 tests, eligible run at b53783d),
              tb/system/SIGNOFF_SYSTEM.md (617/617), docs/DECISIONS.md D-016/D-017.
  [S]  SPEC   docs/SPEC.md v1.1 design constants (not results).
  ⟦HW: …⟧ / ⟦SIM: …⟧ / ⟦HUMAN: …⟧ = not measured / not decided yet. Never replace with a guess.
  Phase H (hardware) has not started; there are NO hardware results in this file yet.
-->

**Big systolic arrays leave most of their rows idle on one-token-at-a-time LLM decode. SplitSys is an 8×8 int8 array that rewires itself into a 4×16 array at runtime. Our cycle model predicts this roughly doubles decode utilization (MODEL 2.02–2.07× [M]; co-simulation of the RTL gives the same 2.02–2.07× [SIM]; on hardware ⟦HW: measured ratio⟧). The price is 25% more operand bandwidth per cycle and an area cost we measured.**

> ⟦HUMAN: GIF or photo of the board running, PE grid lit on the monitor⟧

## What it is

- **Array:** 64 int8 processing elements on a RealDigital Urbana board (AMD Spartan-7 XC7S50), output-stationary, one DSP48E1 per PE, 100 MHz target [S].
- **Two shapes from one array:**
  - **FULL** is one 8×8 tile.
  - **STRIP** splits the array at row 4 into two 4×8 slabs. Rows 4–7 reuse the activations of rows 0–3, and W lanes 8–15 enter at the row-4 boundary, so the array acts as one 4×16 tile [S].
  - The mode is a field in each job descriptor. Switching needs no reconfiguration and no reset.
- **Around the array:**
  - on-chip SRAM buffers (WBUF, ABUF, OBUF)
  - a tile sequencer
  - a 16-lane int8 requantizer
  - 12 hardware performance counters
  - a self-test ROM
  - a framed UART protocol (CRC-8) to a Rust host, which verifies every output bit-exactly against a Rust golden model
- **Dashboard:** a live dashboard on a second monitor shows the PE grid, utilization, and model-vs-measured numbers. It is offline and built from vanilla JS and SVG.
- **Workload:** the four matmuls of one transformer block (d_model = 64): `qkv` 64×192, `attn_out` 64×64, `ffn_up` 64×256, `ffn_down` 256×64. We run each at M = 1 (decode) and M = 64 (prefill), plus an M sweep 1…64 [S].

## Headline results

Utilization = `mac_useful / (64 · cycles_total)`, read from the performance counters (SPEC §3.7). Int8 output, requant on.

| GEMM | M | Mode | **Measured (HW)** | SIM [SIM] | Model [M] | HW model error |
| --- | ---: | --- | ---: | ---: | ---: | ---: |
| ffn_up | 1 | FULL | ⟦HW: ffn_up M=1 FULL util⟧ | 6.95% | 6.95% | ⟦HW: err⟧ |
| ffn_up | 1 | STRIP | ⟦HW: ffn_up M=1 STRIP util⟧ | 14.40% | 14.40% | ⟦HW: err⟧ |
| ffn_up | 64 | FULL | ⟦HW: ffn_up M=64 FULL util⟧ | 55.65% | 55.65% | ⟦HW: err⟧ |
| ffn_up | 64 | STRIP | ⟦HW: ffn_up M=64 STRIP util⟧ | 57.65% | 57.65% | ⟦HW: err⟧ |
| ffn_down | 1 | FULL | ⟦HW: ffn_down M=1 FULL util⟧ | 10.41% | 10.41% | ⟦HW: err⟧ |
| ffn_down | 1 | STRIP | ⟦HW: ffn_down M=1 STRIP util⟧ | 21.09% | 21.09% | ⟦HW: err⟧ |

**SIM is co-simulation of the RTL, not hardware.** The SIM column comes from Verilator running the real top-level RTL. The real host binary drives it byte by byte through the UART pins, over a TCP bridge. Its counters equal the model **exactly**, on every counter of every one of the 46 committed runs [SIM]. That agreement is expected: the sequencer is deterministic, there is no DRAM, and the model and the RTL implement the same confirmed latencies (D-017). So it shows the RTL does what the spec says. It does not yet show the model predicts a physical chip. **The real test of the model is the HW column.**

<!-- [M] values: results/model/sweep.json points (ffn_up,1,FULL) util_model 0.069527 / cycles_total 3682 / tiles 32;
     (ffn_up,1,STRIP) 0.143982 / 1778 / 16; (ffn_up,64,FULL) 0.556484 / 29442; (ffn_up,64,STRIP) 0.576536 / 28418;
     (ffn_down,1,FULL) 0.104149 / 2458; (ffn_down,1,STRIP) 0.210873 / 1214.
     [SIM] values: newest committed record per shape (build 034cbc27 for ffn_up M=1/64:
     results/sim/2026-10-24T17-39-15Z-ffn_up-m1-full.json cycles_total 3682, -17-39-19Z-ffn_up-m1-strip 1778,
     -17-39-49Z-ffn_up-m64-full 29442, -17-40-13Z-ffn_up-m64-strip 28418; build 0b53783d for ffn_down:
     -17-32-48Z-ffn_down-m1-full 2458, -17-32-48Z-ffn_down-m1-strip 1214). Same cycles as the model. -->

- **Decode (M = 1):**
  - In FULL mode, one output row occupies an 8-row tile, so at most 1/8 of the PEs can do useful work. In STRIP mode the bound is 1/4. These mapping ceilings of 12.5% and 25% come from the spec [S].
  - Each tile also pays fixed setup, flush, and drain cycles, which is why the model lands at 7.0% and 14.4%, not 12.5% and 25% [M].
  - STRIP also needs half as many tiles for the same job (16 vs 32 for `ffn_up`) [M].
- **Prefill (M = 64):**
  - Both modes fill every PE, so the mapping is identical.
  - The model still puts STRIP slightly ahead (57.7% vs 55.6%, 1.036× [M]). The reason is drain length, not the split: a STRIP tile drains 4 rows instead of 8.
  - That faster drain relies on the 16-lane output path, which is part of the split's area cost (below). With drain overlapped (shadow accumulators, not built), the gap would mostly vanish.
- **Model vs counters:**
  - SIM: 46 committed co-sim runs across 30 shapes, with 0 counter differences and `cycles_stall` = 0 on every run [SIM].
  - HW: ⟦HW: per-counter model error across N runs, max |err|⟧.

Plots, regenerated from the JSON by `results/plots/make_plots.py`, in dark and light variants:

| | |
| --- | --- |
| ![Utilization vs M](results/plots/util_vs_m_light.png) | ![Benchmark bars](results/plots/bench_bars_light.png) |
| ![Roofline](results/plots/roofline_light.png) | ![Cost of splitting](results/plots/cost_light.png) |

Every plot is titled with its finding and labeled with its source. A plot shows hardware points only when `results/hw/*.json` exists, and currently none does.

## Architecture

```
 host ──USB-UART──► uart_rx ─► packet (framing + CRC-8) ─► regfile / memory mux
                                                                │
   sequencer: descriptor → tiles → STREAM → FLUSH → DRAIN → WRITEBACK
                                                                ▼
 ABUF (64b, 8 lanes) ─► A skew ─► ┌── 8×8 PE array ──┐ ─► drain mux ─► requant ×16 ─► OBUF
 WBUF (128b, 16 lanes)─► W skew ─►│ Q0 │ Q1          │
                                  │────┼──── row-4 mux ◄── W lanes 8..15 (STRIP, delayed 4 + c)
                                  │ Q2 │ Q3          │
                                  └──────────────────┘   perf counters ◄── array + FSM taps
```

- **Full diagram:** `docs/SPEC.md` §2 and dashboard Scene 3. ⟦TODO: export the Scene 3 SVG to `results/plots/architecture.svg` and link it here⟧.
- **The split:**
  - A mux at row 4 selects W from the PE above (FULL) or from W lanes 8–15 (STRIP).
  - A source mux feeds rows 4–7 with the activations of rows 0–3.
  - W lanes 8–15 are delayed `4 + c` cycles, so the lower slab stays in lockstep with the upper one [S].
- **The price in bandwidth:** STRIP streams 4 A bytes + 16 W bytes = 20 B/cycle, against 8 + 8 = 16 B/cycle in FULL. That is 25% more operand bandwidth [S].

## Verification

All counts below are from `tb/SIGNOFF.md` and `tb/system/SIGNOFF_SYSTEM.md` [V] unless noted.

- **Sign-off:** `make -C tb signoff` passed 11 of 11 cocotb + Verilator suites and 64 of 64 tests, in a run marked sign-off eligible at b53783d. That covers the eight units and the level-2 core, each with and without STRIP. The earlier unit run passed 8 of 8 suites and 50 of 50 tests at seeds 1 and 2. The suites cover:
  - PE: 10,000 random MAC streams per seed
  - requant: 100,152 cases, including 14,724 rounding ties
  - packet layer: 10,000 random frames
  - array: 500 random FULL tiles and 500 random STRIP tiles, with a single-nonzero test at every output position in both modes
- **Expected values come only from Rust:** the golden model, the vector generator, and the cycle model. Python never re-derives them.
- **Mutation testing:** every unit suite includes deliberate RTL mutants, and each one was caught (STATUS 12:00, tb/SIGNOFF.md). In the array suite these were a wrong row-4 skew base, a wrong lower-half column mask, a wrong lower-half drain, and a removed A-source mux.
- **Core (level 2):** `tb/core` runs all 262 directed jobs, including a single nonzero at every PE position and quadrant isolation, plus random jobs. The output buffer is bit-exact and all 12 counters equal the cycle model [V]. Before that, the RTL designer's own smoke run passed 283 golden jobs (DECISIONS D-017).
- **System (level 3):** `tb/system` drives the whole top only through its pins (UART, buttons, LEDs). It passed 617 of 617 jobs in each of three runs:
  - 200 random FULL jobs up to 64×256×256
  - 100 random STRIP jobs
  - the 262 directed jobs
  - mode-switch chains
  - self-test, including two runs at seeds 2 and 3 from a random initial state
  The latest run, at RTL adca4a3, is recorded as not yet sign-off eligible because test-bench edits were uncommitted at the time [V].
- **Co-simulation (SIM):** the real `splitsys-host` binary drives the simulated top over a TCP-to-UART bridge. Results: 46 committed run records, 30 distinct shapes, 181,632 outputs compared, 0 mismatches, 0 counter differences from the model [SIM].
- **On hardware:** ⟦HW: GEMMs verified, MACs verified, mismatches (results/session.json)⟧.

## Cost of splitting

Two post-route builds of the same top, with `ENABLE_STRIP = 1` and `ENABLE_STRIP = 0` (Vivado 2025.2.1, xc7s50csga324-2, git 8bbd7ba6) [B]:

| | FULL-only | Split | Delta |
| --- | ---: | ---: | ---: |
| LUT | 5,149 | 7,388 | +2,239 (+43%) |
| FF | 6,346 | 7,936 | +1,590 (+25%) |
| DSP48E1 | 80 | 96 | +16 |
| BRAM | 26 | 26 | 0 |
| WNS at 100 MHz | +2.441 ns | +1.143 ns | both meet timing |

**Where the delta lives** (`breakdown.delta` in cost.json) [B]:

| Block | LUT | FF | DSP |
| --- | ---: | ---: | ---: |
| Array STRIP muxes | +376 | +264 | 0 |
| Sequencer STRIP control | +287 | +70 | 0 |
| Requant lanes 8–15 | +1,503 | +1,216 | +16 |

**Read it honestly:**
- **Most of the delta is the 16-wide output path,** not the split itself. The FULL-only build trims requant lanes 8–15, because FULL drains only 8 lanes per row.
- **The split proper** (array muxes plus sequencer control) costs about +663 LUT.
- **The Fmax figures are estimates,** derived from WNS at the 100 MHz constraint: 132.3 → 112.9 MHz. They are not a frequency sweep. Both builds' worst paths are routing-dominated (89% and 93% routing, 2 logic levels each), so the Fmax delta is not a clean measure of what splitting costs.
- **These are the pre-freeze builds.** If KG1 is built from a later commit, refresh the figures from that build. ⟦BUILD: confirm cost.json git hash == KG1 build⟧.

## Reproduce

Run from the repo root. On Windows, use Git Bash and run Linux tools through `scripts/wsl`.

```bash
scripts/wsl 'scripts/check_env.sh'                                   # toolchain present
scripts/wsl 'cd sw && cargo test --workspace'                        # golden, cycle model, host
scripts/wsl 'make -C tb -j12 signoff'                                # unit + core sign-off suites
scripts/wsl 'make -C tb system'                                      # whole top through its pins
scripts/wsl 'make -C tb cosim-check'                                 # real host vs simulated top -> results/sim
scripts/wsl 'make -C tb cosim'   # then: splitsys-host --transport tcp:127.0.0.1:<port> bench smoke
scripts/wsl 'cd sw && cargo run -p splitsys-cycle -- all'            # regenerate results/model
scripts/wsl 'python results/plots/make_plots.py --check'             # regenerate plots (deterministic)
scripts/vivado -mode batch -source fpga/tcl/build.tcl -tclargs top   # bitstream (+ top_nostrip for cost)
```

On hardware, run `splitsys-host ping | selftest | bench bench | bench sweep | serve` against the board's COM port. The ordered bring-up script is `docs/HW_CHECKLIST.md`. ⟦HW: confirm the exact Windows command line used at H-6⟧.

## Limits and non-goals

- **No speed claim against CPUs or GPUs.** The claim is about utilization, operand bandwidth per MAC, and whether the model predicts the counters.
- **End-to-end time is UART-bound** (115200 baud). Wall-clock time is never reported as accelerator performance. On-chip cycles come from the counters.
- **Tiny model.** d_model = 64, a single transformer block, and synthetic int8 data with a fixed seed. No DDR3: weights live in on-chip BRAM. Softmax, norms, and attention-score matmuls run on the host.
- **No overlap between stages.** STREAM, FLUSH, and DRAIN run back to back, and the array idles during drain. That fixed per-tile overhead is why prefill utilization tops out near 56–58% for K = 64 [M]. The model assumes zero stall cycles, and co-simulation measured 0 on every run [SIM]. On hardware: ⟦HW: measured `cycles_stall`⟧.
- **INDEP mode** (four independent 4×4 jobs) is specified but not built.

## What's next

- Shadow accumulators, so that drain overlaps the next tile's stream.
- Loading requant parameters during STREAM instead of in a 16-cycle setup state [S].
- INDEP mode for per-head attention.
- DDR3 weight streaming, to study memory-bound decode.
- Automatic mode selection, driven by bandwidth and not only by shape. On-chip, the model never picks FULL: STRIP is never slower in any of the 28 points it scores (2.02–2.07× at M ≤ 4, 1.013–1.036× at M ≥ 8) [M]. FULL matters only when operand bandwidth is the constraint, since it needs 16 B/cycle against STRIP's 20.

## Built at Cal Hacks 13.0

San Francisco, October 2026. All project code was written during the event. ⟦HUMAN: team members / solo⟧.
