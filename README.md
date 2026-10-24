# SplitSys: a splittable int8 systolic array for LLM matmuls, on a Spartan-7 FPGA

<!--
HONESTY KEY (red-team: audit every number in this file against this key).
  [M]  MODEL  results/model/sweep.json + roofline.json, model_git_hash 72da65248adf
              (latencies confirmed by RTL, D-017). Values also in results/plots/manifest.json.
  [SIM] SIM   results/sim/*.json (top level only; never results/sim/archive_dirty/): 70 per-run records
              (results/sim/session.json: gemms_run 70, macs_verified 17,014,784, mismatches 0), from two
              clean builds of the same RTL (git diff f402cc7 5900507 -- rtl spec is empty):
                66 at build_id 0f402cc7 / git f402cc7, committed e2cea28 (56-run bench sweep + 10 cosim-check);
                 4 at build_id 05900507 / git 5900507, committed fb89e37 (scripted demo: ffn_up M=1,64 x2 modes).
              model_git_hash 72da65248adf on all 70. 58 distinct shape/mode/requant combinations;
              2 records are int32 bypass (requant off). Verilator co-sim of the real top, driven through
              its UART pins by the real splitsys-host. Every record: golden_match, 0 mismatches,
              invariants_ok, all 12 counters == model, cycles_stall 0; 217,088 outputs compared in total.
              SIM is NOT hardware.
  [B]  BUILD  results/cost.json (source "hw-build"), committed 8ccb333: S4 freeze builds, build_id 0x52A66216
              (git 52a66216) for both, Vivado 2025.2.1 post-route. RTL identical to the SIM builds above.
  [V]  VERIF  tb/SIGNOFF.md (11/11 suites, 64/64 tests, eligible run at b53783d),
              tb/system/SIGNOFF_SYSTEM.md (617/617, eligible, head f402cc7), docs/DECISIONS.md D-016/D-017.
  [HW] HW     results/hw/*.json (top level, source "hw"): 256 per-run records from the Phase H session on the board
              (COM3, Sat 13:00-13:42), all at build_id 52a66216, host git 3976ec0, model_git_hash 72da65248adf:
                200 from H-5 `run --random 100 --mode both --seed 1` (100 FULL + 100 STRIP, up to M/K/N 64/256/256,
                    17 of them int32 bypass), committed e180386, log results/hw/logs/H-5_random_both_seed1.log;
                 56 from H-6 `bench sweep --seed 1` (4 GEMMs x M 1..64 x FULL/STRIP), committed 2bacace,
                    summary results/hw/bench_sweep_2026-10-24T20-41-07Z.json, log results/hw/logs/H-6_bench_sweep_seed1.log.
              Derived by the storyteller from the 256 records (not copied from session.json, which agrees):
                golden_match 256/256, mismatches 0, outputs_compared 952,012, mac_useful sum 118,801,362,
                invariants_ok 256/256, 12 counters x 256 runs = 3,072 counter values, 0 differ from the model,
                cycles_stall 0 on all 256, latencies_provisional false on all 256.
              results/session.json: gemms_verified 256, macs_verified 118801362, mismatches 0.
              H-5 shape-matched pairs (same job index, same M/K/N/requant in both modes): 97; STRIP fewer cycles 61,
                equal 5, more 31 (max 1.3154x, gemm_full_s1_56 vs gemm_strip_s1_56, M=22 K=250 N=17: 2711 vs 3566 cycles, H-5 log lines 116-117);
                all 31 have STRIP tiles > FULL tiles.
              Counters are on-chip cycles at 100 MHz, NOT wall clock.
              Bitstream archived as fpga/known_good/KG1 (CURRENT -> KG1, commit 7b3af2c):
                summary.json build_id 0x52A66216, archive.txt bit_md5 6a85a822b28e7d9f2097a8a902d865bf.
  [S]  SPEC   docs/SPEC.md v1.1 design constants (not results).
  ⟦HUMAN: …⟧ = not decided yet. Never replace with a guess.
-->

**Big systolic arrays leave most of their rows idle on one-token-at-a-time LLM decode. SplitSys is an 8×8 int8 array that rewires itself into a 4×16 array at runtime. Measured on the board's hardware counters, that doubles decode utilization: 2.02–2.07× across the four matmuls of a transformer block at M = 1 [HW]. A cycle model written before the board was touched predicted all 12 counters exactly, on every one of 256 hardware GEMMs [HW]. The price is 25% more operand bandwidth per cycle and an area cost we measured.**

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
| ffn_up | 1 | FULL | **6.95%** | 6.95% | 6.95% | 0 (12 of 12 counters equal) |
| ffn_up | 1 | STRIP | **14.40%** | 14.40% | 14.40% | 0 (12 of 12 counters equal) |
| ffn_up | 64 | FULL | **55.65%** | 55.65% | 55.65% | 0 (12 of 12 counters equal) |
| ffn_up | 64 | STRIP | **57.65%** | 57.65% | 57.65% | 0 (12 of 12 counters equal) |
| ffn_down | 1 | FULL | **10.41%** | 10.41% | 10.41% | 0 (12 of 12 counters equal) |
| ffn_down | 1 | STRIP | **21.09%** | 21.09% | 21.09% | 0 (12 of 12 counters equal) |

At M = 1 the measured STRIP/FULL ratio is 2.07× for `qkv`, `attn_out` and `ffn_up`, and 2.02× for `ffn_down` [HW]. Every one of these runs was bit-exact against the Rust golden model.

**HW is the board.** The HW column is read from the 12 on-chip performance counters of the Spartan-7 at build 0x52A66216 (archived as `fpga/known_good/KG1`), in one hardware session over UART [HW]. These are on-chip cycles at 100 MHz, not wall-clock time.

**SIM is co-simulation of the RTL, not hardware.** The SIM column comes from Verilator running the real top-level RTL. The real host binary drives it byte by byte through the UART pins, over a TCP bridge. Its counters equal the model **exactly**, on every counter of every one of the 70 committed runs [SIM]. That agreement is expected: the sequencer is deterministic, there is no DRAM, and the model and the RTL implement the same confirmed latencies (D-017). So it shows the RTL does what the spec says. It does not show that the model predicts a physical chip. **The HW column is that test, and it came out exact:** on all 256 hardware runs, all 12 counters equal the model [HW].

<!-- [HW] values: results/hw/2026-10-24T20-40-41Z-ffn_up-m1-full.json util 0.069527 / cycles_total 3682 / tiles 32;
     20-40-41Z-ffn_up-m1-strip 0.143982 / 1778 / 16; 20-40-54Z-ffn_up-m64-full 0.556484 / 29442;
     20-40-57Z-ffn_up-m64-strip 0.576536 / 28418; 20-40-59Z-ffn_down-m1-full 0.104150 / 2458;
     20-40-59Z-ffn_down-m1-strip 0.210873 / 1214. Each record: model block identical to counters, model_err_total 0.0,
     golden_match true, mismatches 0, invariants_ok true, cycles_stall 0. Also in bench_sweep_2026-10-24T20-41-07Z.json
     and the H-6 log (err% +0.000 on all 56 lines).
     M=1 STRIP/FULL util ratios [HW]: qkv 0.143928/0.069515 = 2.0705; attn_out 0.143498/0.069414 = 2.0673;
     ffn_up 0.143982/0.069527 = 2.0709; ffn_down 0.210873/0.104150 = 2.0247 -> "2.02–2.07×".
     M=64 ratios [HW]: 1.0132 (ffn_down) to 1.0360 (ffn_up) -> "1.013–1.036×".
     [M] values: results/model/sweep.json points (ffn_up,1,FULL) util_model 0.069527 / cycles_total 3682 / tiles 32;
     (ffn_up,1,STRIP) 0.143982 / 1778 / 16; (ffn_up,64,FULL) 0.556484 / 29442; (ffn_up,64,STRIP) 0.576536 / 28418;
     (ffn_down,1,FULL) 0.104149 / 2458; (ffn_down,1,STRIP) 0.210873 / 1214.
     [SIM] values (build 0f402cc7): results/sim/2026-10-24T18-21-15Z-ffn_up-m1-full.json cycles_total 3682,
     18-21-17Z-ffn_up-m1-strip 1778, 18-21-49Z-ffn_up-m64-full 29442, 18-21-58Z-ffn_up-m64-strip 28418,
     18-22-03Z-ffn_down-m1-full 2458, 18-22-03Z-ffn_down-m1-strip 1214. Same cycles as the model.
     The 4 scripted-demo records at build 05900507 (19-16-16Z ffn_up m1 full/strip, 19-16-28Z m64-full,
     19-16-38Z m64-strip) give the same cycles_total 3682 / 1778 / 29442 / 28418. -->

- **Decode (M = 1):**
  - In FULL mode, one output row occupies an 8-row tile, so at most 1/8 of the PEs can do useful work. In STRIP mode the bound is 1/4. These mapping ceilings of 12.5% and 25% come from the spec [S].
  - Each tile also pays fixed setup, flush, and drain cycles, which is why the model lands at 7.0% and 14.4%, not 12.5% and 25% [M].
  - STRIP also needs half as many tiles for the same job (16 vs 32 for `ffn_up`) [M].
- **Prefill (M = 64):**
  - Both modes fill every PE, so the mapping is identical.
  - STRIP is still slightly ahead, measured and modeled alike (57.65% vs 55.65%, 1.036× [HW]; the same in [M]). The reason is drain length, not the split: a STRIP tile drains 4 rows instead of 8.
  - That faster drain relies on the 16-lane output path, which is part of the split's area cost (below). With drain overlapped (shadow accumulators, not built), the gap would mostly vanish.
- **Model vs counters:**
  - SIM: 70 committed co-sim runs (58 distinct shape/mode/requant combinations; two clean builds of the same RTL, 66 runs at f402cc7 and 4 at 5900507), with 0 counter differences and `cycles_stall` = 0 on every run [SIM].
  - HW: 256 runs on the board (200 random GEMMs up to 64×256×256 in both modes, plus the 56-run benchmark sweep). 0 of 3,072 counter values differ from the model, so the maximum per-counter error is 0. `cycles_stall` = 0 on every run [HW]. No calibration was applied: the model is frozen at `model_git_hash` 72da65248adf.
- **STRIP is not always faster (measured).** In the H-5 random batch, 97 job indices drew the same shape in both modes. STRIP took fewer cycles on 61, the same on 5, and **more on 31**, by up to 1.32× (M = 22, K = 250, N = 17: 3,566 vs 2,711 cycles) [HW]. Every one of those 31 needs more STRIP tiles than FULL tiles. STRIP pads M to a multiple of 4 and N to a multiple of 16, while FULL pads both to 8, and on those shapes the extra padding costs more than STRIP gains. The model predicted each of those cycle counts exactly, so a mode selector can rely on it.

Plots, regenerated from the JSON by `results/plots/make_plots.py`, in dark and light variants:

| | |
| --- | --- |
| ![Utilization vs M](results/plots/util_vs_m_light.png) | ![Benchmark bars](results/plots/bench_bars_light.png) |
| ![Roofline](results/plots/roofline_light.png) | ![Cost of splitting](results/plots/cost_light.png) |

Every plot is titled with its finding and labeled with its source. The utilization and benchmark plots show the 56 hardware benchmark points (filled markers and bars) against the model (hollow rings and ticks), and their titles say `HW`. Where the board measured a point, the identical SIM point is not drawn. The roofline title stays `MODEL`, because arithmetic intensity and the roofs are model quantities; its filled points are HW. The plots use the 239 int8 hardware records and skip the 17 int32-bypass ones (`results/plots/manifest.json`).

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
- **Mutation testing:** `tb/SIGNOFF.md` records deliberate mutants, or a sensitivity check against the old RTL, for the skew, counters, packet, array and core suites. Each one was caught. In the array suite these were a wrong row-4 skew base, a wrong lower-half column mask, a wrong lower-half drain, and a removed A-source mux.
- **Core (level 2):** `tb/core` runs all 262 directed jobs, including a single nonzero at every PE position and quadrant isolation, plus random jobs. The output buffer is bit-exact and all 12 counters equal the cycle model [V]. Before that, the RTL designer's own smoke run passed 283 golden jobs (DECISIONS D-017).
- **System (level 3):** `tb/system` drives the whole top only through its pins (UART, buttons, LEDs). The sign-off-eligible run at head f402cc7 passed 617 of 617 jobs [V]:
  - 200 random FULL jobs up to 64×256×256
  - 100 random STRIP jobs
  - the 262 directed jobs
  - 52 mode-switch jobs
  - 3 self-test checks
- **Co-simulation (SIM):** the real `splitsys-host` binary drives the simulated top over a TCP-to-UART bridge. At build f402cc7 the host ran a bench sweep (all 4 GEMMs × M = 1…64 × both modes, 56 runs) plus the 10-run cosim check. At build 5900507 (same RTL) it ran the 4-run scripted demo. Results: 70 committed run records, 217,088 outputs compared, 17,014,784 MACs verified, 0 mismatches, 0 counter differences from the model [SIM].
- **On hardware (one session on the board, build 0x52A66216) [HW]:**
  - UART link (H-3): `memtest` wrote and read back 10,000 random frames (5,123,818 bytes) across WBUF, ABUF, OBUF and the requant parameters, with 0 errors and 0 retries (`results/hw/logs/H-3_memtest_seed1.log`).
  - Self-test (H-4): the on-chip ROM jobs passed in both modes, and the read-back OBUF matched golden, with 0 of 256 bytes different (`results/hw/logs/H-4_selftest_dump.log`).
  - Random GEMMs (H-5): 200 runs (100 per mode, up to 64×256×256, 17 with int32 output), with 0 output mismatches, 0 counter mismatches, and 0 invariant failures (`results/hw/logs/H-5_random_both_seed1.log`).
  - Benchmark sweep (H-6): 56 of 56 runs passed, with a model error of +0.000% at every point (`results/hw/logs/H-6_bench_sweep_seed1.log`).
  - Totals over the 256 run records: 256 GEMMs and 118,801,362 MACs verified, 952,012 outputs compared, 0 mismatches. `results/session.json` agrees.

## Cost of splitting

Two post-route builds of the same top, with `ENABLE_STRIP = 1` and `ENABLE_STRIP = 0` (Vivado 2025.2.1, xc7s50csga324-2, S4 freeze, build 0x52A66216 for both) [B]:

| | FULL-only | Split | Delta |
| --- | ---: | ---: | ---: |
| LUT | 5,142 | 7,389 | +2,247 (+43.7%) |
| FF | 6,346 | 7,936 | +1,590 (+25.1%) |
| DSP48E1 | 80 | 96 | +16 (+20%) |
| BRAM | 26 | 26 | 0 |
| WNS at 100 MHz | +2.097 ns | +0.523 ns | both meet timing |

**Where the delta lives** (`breakdown.delta` in cost.json) [B]:

| Block | LUT | FF | DSP |
| --- | ---: | ---: | ---: |
| Array STRIP muxes | +376 | +264 | 0 |
| Sequencer STRIP control | +290 | +70 | 0 |
| Requant lanes 8–15 | +1,503 | +1,216 | +16 |
| Everything else | +78 | +40 | 0 |

<!-- [B] results/cost.json: full_only/split lut 5142/7389, ff 6346/7936, dsp 80/96, bram 26/26, wns_ns 2.097/0.523,
     fmax_mhz 126.5/105.5; breakdown.delta array 376/264/0, sequencer 290/70/0, requant 1503/1216/16,
     total 2247/1590/16. "Everything else" = total - (array + requant + sequencer) = 78 LUT, 40 FF.
     % = delta / full_only: 2247/5142 = 43.7%, 1590/6346 = 25.1%, 16/80 = 20%.
     worst_path.split: u_board/rst_r_reg_rep__1/C -> rst_core_reg_rep__29/D, logic_levels 0, route_pct 96.189.
     worst_path.full_only: u_core/u_seq/FSM_sequential_state_reg[1]/C -> u_core/u_seq/cnt_reg[6]/R,
     logic_levels 7, route_pct 77.319. Earlier split WNS +1.143 ns: results/cost.json at 8aeb9b0 (git 8bbd7ba6);
     git diff 8bbd7ba6 52a66216 -- rtl spec is empty. -->

**Read it honestly:**
- **Most of the delta is the 16-wide output path,** not the split itself. Requant lanes 8–15 are 1,503 of the 2,247 LUTs and all 16 DSPs. The FULL-only build trims them, because FULL drains only 8 lanes per row.
- **The split proper** (array muxes plus sequencer control) costs +666 LUT and +334 FF.
- **Don't read the Fmax estimate as a cost of splitting.** From WNS at the 100 MHz constraint, the estimates are 126.5 MHz (FULL-only) and 105.5 MHz (split). They are not a frequency sweep. The split build's worst path is a replicated reset net (`u_board/rst_r_reg_rep__1` → `rst_core_reg_rep__29`): 0 logic levels, 96% routing. That is placement, not STRIP logic. The same RTL built earlier at 8bbd7ba6 closed with WNS +1.143 ns, so the split build's slack moved by 0.62 ns between two builds of identical RTL. We headline the LUT, FF and DSP counts, not Fmax.
- **These are the S4 freeze builds** (cost.json committed in 8ccb333). The split build is the bitstream that ran on the board. It is archived as `fpga/known_good/KG1`: `summary.json` gives build_id 0x52A66216 and the same LUT, FF, DSP, BRAM and WNS values as cost.json. So the cost numbers and the hardware numbers describe the same bitstream.

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
bash fpga/tcl/s4_rebuild.sh --no-commit                              # all 6 bitstreams at one hash + results/cost.json
```

On hardware (Windows; the board enumerated as COM3 in our session), these commands produced the HW evidence:

```bash
splitsys-host --transport serial:COM3 memtest --frames 10000 --seed 1        # H-3
splitsys-host --transport serial:COM3 selftest --dump                        # H-4
splitsys-host --transport serial:COM3 run --random 100 --mode both --seed 1  # H-5
splitsys-host --transport serial:COM3 bench sweep --seed 1                   # H-6
```

The ordered bring-up script, with expected output and failure branches, is `docs/HW_CHECKLIST.md`. `serve` runs the live dashboard.

## Limits and non-goals

- **No speed claim against CPUs or GPUs.** The claim is about utilization, operand bandwidth per MAC, and whether the model predicts the counters.
- **End-to-end time is UART-bound** (115200 baud). Wall-clock time is never reported as accelerator performance. On-chip cycles come from the counters.
- **Tiny model.** d_model = 64, a single transformer block, and synthetic int8 data with a fixed seed. No DDR3: weights live in on-chip BRAM. Softmax, norms, and attention-score matmuls run on the host.
- **No overlap between stages.** STREAM, FLUSH, and DRAIN run back to back, and the array idles during drain. That fixed per-tile overhead is why prefill utilization tops out near 56–58% for K = 64 [M]. The model assumes zero stall cycles. Co-simulation measured 0 on every run [SIM], and so did the board, on all 256 runs [HW]. That shows only that on-chip BRAM keeps up at 20 B/cycle. There is no DRAM path, so a memory-bound regime was never tested.
- **INDEP mode** (four independent 4×4 jobs) is specified but not built.

## What's next

- Shadow accumulators, so that drain overlaps the next tile's stream.
- Loading requant parameters during STREAM instead of in a 16-cycle setup state [S].
- INDEP mode for per-head attention.
- DDR3 weight streaming, to study memory-bound decode.
- Automatic mode selection, driven by the cycle model, which matched every hardware counter. On the 28 benchmark shapes STRIP is never slower (2.02–2.07× at M ≤ 4, 1.013–1.036× at M ≥ 8 [HW]). On 31 of 97 random shapes it was slower [HW], so the selector has to score tile counts, not just M. Bandwidth matters too: FULL needs 16 B/cycle against STRIP's 20.

## Built at Cal Hacks 13.0

San Francisco, October 2026. All project code was written during the event. ⟦HUMAN: team members / solo⟧.
