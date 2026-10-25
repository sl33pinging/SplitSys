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
  [B]  BUILD  results/cost.json (source "hw-build"), committed 23acf6a: S4 freeze rebuild at git 38a3950a,
              build_id 0x38A3950A for both builds, Vivado 2025.2.1 post-route. The split build is the KG2 bitstream
              (fpga/known_good/KG2/summary.json: build_id 0x38A3950A, lut 7389, ff 7936, dsp 96, bram 26, wns 0.744).
              Datapath RTL identical to the SIM builds: git diff f402cc7 52a66216 -- rtl spec is empty;
              git diff 52a66216 38a3950a -- rtl spec touches only rtl/top.sv + rtl/board_io.sv
              (LEFT_IS_D1, AN0_IS_RIGHT display defaults and comments). The RGB PULLDOWN is an XDC change.
              KG3 (live demo bitstream): fpga/reports/kg3_top/summary.json = fpga/known_good/KG3/summary.json:
              build_id 0x21DD6850, lut 7377, ff 7928, dsp 96, bram 26, wns 0.473, whs 0.022, fmax_est 105.0.
              git diff 38a3950a 21dd6850 -- rtl spec fpga/xdc: rtl/gen/splitsys_pkg.sv BAUD 115200 -> 1562500 and
              BAUD_DIV 54 -> 4; spec/spec.toml baud (+ rx_timeout comment); XDC comment-only. D-020.
              7389-7377 = 12 LUT, 7936-7928 = 8 FF: separate implementation run, not attributed.
  [V]  VERIF  tb/SIGNOFF.md (11/11 suites, 64/64 tests, eligible run at b53783d),
              tb/system/SIGNOFF_SYSTEM.md (617/617, eligible, head f402cc7), docs/DECISIONS.md D-016/D-017.
              Live-bitstream RTL (KG2, last rtl/ commit dc1dea0; git diff dc1dea0 38a3950a -- rtl spec is empty),
              signed off in commit 9ec73e1: tb/SIGNOFF.md "Sign-off for KG2 candidate RTL dc1dea0" (11/11 suites,
              64/64 tests, eligible YES, run 20261024-134407-25872 at head 7b3af2c; also 11/11 64/64 at head dc1dea0)
              and tb/system/SIGNOFF_SYSTEM.md (625/625 = the previous 617 + display 8/8, eligible YES, head bee1cec).
              KG3 (D-020): tb/SIGNOFF.md "Sign-off for the KG3 candidate" 11/11 suites, 66/66 tests at head ac893cd
              (adds real-divisor UART tests); tb/system/SIGNOFF_SYSTEM.md "SIGN-OFF for the KG3 candidate" 625/625;
              cosim-check 8/8 host steps (10 GEMMs, 0 mismatches), eligible YES (SIGNOFF_SYSTEM.md; commits ac893cd, 087e504).
  [HW] HW     results/hw/*.json, top level, git-TRACKED only (git ls-files; all untracked booth records, in results/hw
              and results/hw/lm, are excluded), source "hw":
              773 GEMM run records on the board (COM3), from three bitstreams with the same datapath RTL (see [B]):
              KG1, build_id 52a66216 (fpga/known_good/KG1, bit_md5 6a85a822b28e7d9f2097a8a902d865bf): 272 records
                200 H-5 `run --random 100 --mode both --seed 1` (e180386), log results/hw/logs/H-5_random_both_seed1.log;
                 56 H-6 `bench sweep --seed 1` (2bacace), log results/hw/logs/H-6_bench_sweep_seed1.log;
                 16 H-7 dashboard scripted demo (cd75ccc). host git 3976ec0.
              KG2, build_id 38a3950a (fpga/known_good/KG2, bit_md5 63dea71fd7e8c151f836143c8bac2b2a): 301 records
                  1 H-1 check run + 200 H-5 re-run (b80c1ad, host 3976ec0), log results/hw/logs/KG2/H-5_random_both_seed1.log;
                100 G5 auto mode `run --random 100 --mode auto --seed 1` (1e37b2f, host c4c2a79; records carry mode_select),
                    log results/hw/logs/KG2/G5_auto_random100_seed1.log.
              KG3, build_id 21dd6850 (fpga/known_good/KG3, bit_md5 94bb7160ee61aff4b7f8cb9112cd7ed9; CURRENT -> KG3,
                the live demo bitstream): 200 records = Q-5 `run --random 100 --mode both --seed 1` at 1,562,500 baud
                (host 37221e9), log results/hw/logs/KG3/Q-5_random_both_seed1.log. (Lead brief said 201; tracked files: 200.)
                KG3 records carry model_git_hash 21dd68509f54, not 72da65248adf, because that hash is the last commit
                touching spec/ or the model crates, and D-020 changed spec.toml's baud: git diff 72da652 21dd685 --
                sw/crates/cycle sw/crates/spec spec changes only BAUD/BAUD_DIV/rx_timeout comment + spec tests.
                The cycle model code and latencies are unchanged.
              H-5 shapes on all three builds: the 200 KG2 H-5 and 200 KG3 Q-5 counter sets equal KG1's on 200/200 (by gemm name).
              Derived by the storyteller from the committed records (recomputed 2026-10-24 22:2x PDT). results/session.json is
              a live running total kept by the dashboard server and is not committed (lead cf9120b); no total here uses it:
                all 773: golden_match 773/773, mismatches 0, outputs_compared 3,099,658, mac_useful sum 393,129,309,
                  invariants_ok 773/773, 12 counters x 773 = 9,276 counter values, 0 differ from the record's model block,
                  cycles_stall 0 on all, latencies_provisional false on all. 59 are int32 bypass.
                KG1: 272 runs, 1,085,132 outputs, 127,321,042 MACs, 0 of 3,264 counter values differ.
                KG2: 301 runs, 1,208,818 outputs, 159,491,513 MACs, 0 of 3,612 differ
                     (201 non-auto: 805,964 / 106,333,138 / 2,412; 100 auto: 402,854 / 53,158,375 / 1,200).
                KG3: 200 runs, 805,708 outputs, 106,316,754 MACs, 0 of 2,400 differ.
              H-5 shape-matched pairs (KG1; identical on KG2 and KG3): 97; STRIP fewer cycles 61, equal 5, more 31
                (max 1.3154x, gemm_full_s1_56 vs gemm_strip_s1_56, M=22 K=250 N=17: 2711 vs 3566 cycles); all 31 have
                STRIP tiles > FULL tiles.
              AUTO (KG2, 100 records with mode_select; log lines "AUTO: ..."): chosen FULL 38 / STRIP 62; model_cycles_strip >
                model_cycles_full on 33, all 33 ran FULL; 5 ties ran FULL; counters.cycles_total == model on 100/100;
                measured total 1,225,768 cycles; MODEL (never run) always-FULL 1,292,418 (1.0544x), always-STRIP 1,244,981 (1.0157x).
              Model pick vs measured (storyteller recomputation of STATUS 20:44): on KG1 the 125 distinct (M,K,N,requant)
                shapes measured in both modes (97 H-5 + 28 H-6), the model's pick (min model cycles_total, ties -> FULL)
                had the lowest measured cycles_total on 125/125 (5 of them measured ties). KG2 and KG3 H-5: 97/97 each.
                Pooled across all three builds (tracked records, a shape counts if measured in both modes on one build):
                126/126; the extra shape is M=9 K=3 N=9 requant on KG2 (H-1 check run FULL 23-19-53Z + auto STRIP 03-46-15Z).
                Recomputed 2026-10-24 22:5x PDT (storyteller; agrees with red-team G5 recompute; lead reports the dashboard shows 126/126).
              LM (results/hw/lm/*.json, tracked: 3; per-step summaries; logs results/hw/logs/KG2/G5_lm_stories260k_{8,30}tok.log,
                results/hw/logs/KG3/Q-6_lm_8tok.log):
                04-07-28Z KG2 38a3950a @115200, host 5c59139: 8 tokens, 12 steps, gemms 444, bit_exact 444, counters_eq_model 444,
                  cycles_per_step 29540, wall_s_per_step 30.76, text "Once upon a time, there was a little girl".
                04-20-52Z KG2 @115200: 30 tokens, 34 steps, 1258/1258/1258, 29540, wall 30.48 s/step, text "Once upon a time,
                  there was a little girl named Lily. She loved to play outside in the park. One day,".
                04-59-16Z KG3 21dd6850 @1562500, host 37221e9: 8 tokens, 444/444/444, 29540, wall 2.16 s/step, same text as KG2 8.
                All steps: 37 GEMMs, all STRIP (auto), mac_useful 259,328/step -> 259328/(64*29540) = 13.7% util [HW, derived].
                bytes_tx_per_step_steady 245,623; wbuf_plan: 207,616 of 261,888 weight bytes streamed per step, 54,272 pinned.
                FTDI latency timer: fpga/known_good/KG3/archive.txt lead_note: KG2 LM at 115,200 with the 16 ms default;
                  timer set to 1 ms ~21:55 PDT; KG3 Q-6 at 1 ms. Both KG2 LM runs ended before 21:55 (logs end 21:13 / 21:38).
                Model facts: sw/crates/lm/tests/stories260k.rs (run 2026-10-24 ~22:25 PDT with the model files present):
                  config_matches_header ok (dim 64, hidden 172, 5 layers, 8 heads, 4 KV, vocab 512, seq 512);
                  f32_greedy_reproduces_published_readme ok (256-step greedy text == HF readme at revision 0bd21da7);
                  int8_vs_f32_agreement ok, prints: 50 steps first divergence None; 256 steps "first divergent token index:
                  Some(62) (of 257)" where index 0 is BOS -> 61 generated tokens identical; teacher-forced top-1 250/256.
              KG3 qualification logs results/hw/logs/KG3/: Q-1 echo 100,000 bytes 0 mismatched 25,385.5 ms (16 ms timer,
                kg3_heartbeat_uart_echo); Q-7 same 1,566.5 ms (1 ms timer); Q-2 PING build_id 21dd6850 baud 1562500;
                Q-3 selftest FULL/STRIP PASS, 0 of 128 + 0 of 128 OBUF bytes differ, human BTN0 -> LED0+LED1, RGB dark;
                Q-4 memtest 10,000 frames 5,123,818 bytes 0 errors (21:57:05-21:58:26 = 81 s); Q-5 BATCH PASS 200 runs
                (21:58:27-21:59:16 = 49 s); Q-6 LM; Q-8 flash: mt25ql128, "ok in 34 s", DONE '1', FLASH_OK KG3.bin md5 d0d64c60...
                Boot in ~1 s after power cycle: human, docs/STATUS.md 22:08 (no log). Dashboard 2.1 s wall: STATUS 22:19.
                Red-team review: docs/STATUS.md 22:06.
              KG2 H-8 flash: results/hw/logs/KG2/H-8_flash_program.log (same part, 34 s, FLASH_OK). KG2 boot ~1 s: STATUS ~16:50.
              Counters are on-chip cycles at 100 MHz, NOT wall clock.
              Architecture figure results/plots/architecture.png: crop of the Scene 3 SPEC panel from a live dashboard
                screenshot (2026-10-24 17:10 PDT, build 38a3950a).
              H-4 self-test --dump: KG1 results/hw/logs/H-4_selftest_dump.log; KG2 results/hw/logs/KG2/H-4_selftest_dump.log.
  [S]  SPEC   docs/SPEC.md v1.1 design constants (not results).
  ⟦HUMAN: …⟧ = not decided yet. Never replace with a guess.
-->

**Big systolic arrays leave most of their rows idle on one-token-at-a-time LLM decode. SplitSys is an 8×8 int8 array that rewires itself into a 4×16 array at runtime. Measured on the board's hardware counters, that doubles decode utilization: 2.02–2.07× across the four matmuls of a transformer block at M = 1 [HW]. A cycle model written before the board was touched predicted all 12 counters exactly, on every one of 773 hardware GEMMs [HW]. The same board runs every linear layer of a small public language model, bit-exact. The price is 25% more operand bandwidth per cycle and an area cost I measured.**

> ⟦HUMAN: GIF or photo of the board running, PE grid lit on the monitor⟧

## What it is

- **Array:** 64 int8 processing elements on a RealDigital Urbana board (AMD Spartan-7 XC7S50), output-stationary, one DSP48E1 per PE, 100 MHz target [S].
- **Two shapes from one array:**
  - **FULL** is one 8×8 tile.
  - **STRIP** splits the array at row 4 into two 4×8 slabs. Rows 4–7 reuse the activations of rows 0–3, and W lanes 8–15 enter at the row-4 boundary, so the array acts as one 4×16 tile [S].
  - The mode is a field in each job descriptor. Switching needs no reconfiguration and no reset. The host can also let the cycle model pick the mode per job (auto mode).
- **Around the array:**
  - on-chip SRAM buffers (WBUF, ABUF, OBUF)
  - a tile sequencer
  - a 16-lane int8 requantizer (bypassable for int32 output)
  - 12 hardware performance counters
  - a self-test ROM
  - a framed UART protocol (CRC-8, 1,562,500 baud on the live bitstream) to a Rust host, which verifies every output bit-exactly against a Rust golden model
- **Dashboard:** a live dashboard on a second monitor shows the PE grid, utilization, and model-vs-measured numbers. It is offline and built from vanilla JS and SVG.
- **Workloads:**
  - The four matmuls of one transformer block (d_model = 64): `qkv` 64×192, `attn_out` 64×64, `ffn_up` 64×256, `ffn_down` 256×64. I run each at M = 1 (decode) and M = 64 (prefill), plus an M sweep 1…64 [S].
  - Random GEMMs up to 64×256×256.
  - A real public model, stories260K, with every linear layer on the board (below).

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

**HW is the board.** The HW column is read over UART from the 12 on-chip performance counters of the Spartan-7. These are on-chip cycles at 100 MHz, not wall-clock time [HW]. Three bitstreams ran, all with the same datapath RTL:

- **KG1** (build 0x52A66216, `fpga/known_good/KG1`) produced the benchmark evidence: the H-5 random batch, the H-6 sweep behind this table, and the H-7 dashboard runs (272 records).
- **KG2** (build 0x38A3950A, `fpga/known_good/KG2`) changes only two 7-segment orientation defaults in `rtl/top.sv` and `rtl/board_io.sv`, plus a pulldown on unused pins in the constraints file. On KG2 I re-ran the self-test and the random batch, ran auto mode, and ran the language model (301 GEMM records). It is now the fallback bitstream.
- **KG3** (build 0x21DD6850, `fpga/known_good/KG3`) is the live demo bitstream. Its only RTL change is the UART speed: 1,562,500 baud instead of 115,200 (decision D-020). On KG3 I re-ran the full hardware qualification, including the random batch (200 records) and the language model.
- The random batch drew the same 200 shapes on all three builds, and every counter came back identical to KG1's [HW].

**SIM is co-simulation of the RTL, not hardware.** The SIM column comes from Verilator running the real top-level RTL. The real host binary drives it byte by byte through the UART pins, over a TCP bridge. Its counters equal the model **exactly**, on every counter of every one of the 70 committed runs [SIM]. That agreement is expected: the sequencer is deterministic, there is no DRAM, and the model and the RTL implement the same confirmed latencies (D-017). So it shows the RTL does what the spec says. It does not show that the model predicts a physical chip. **The HW column is that test, and it came out exact:** on all 773 hardware runs, all 12 counters equal the model [HW].

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
     18-22-03Z-ffn_down-m1-full 2458, 18-22-03Z-ffn_down-m1-strip 1214. Same cycles as the model. -->

- **Decode (M = 1):**
  - In FULL mode, one output row occupies an 8-row tile, so at most 1/8 of the PEs can do useful work. In STRIP mode the bound is 1/4. These mapping ceilings of 12.5% and 25% come from the spec [S].
  - Each tile also pays fixed setup, flush, and drain cycles, which is why the model lands at 7.0% and 14.4%, not 12.5% and 25% [M].
  - STRIP also needs half as many tiles for the same job (16 vs 32 for `ffn_up`) [M].
- **Prefill (M = 64):**
  - Both modes fill every PE, so the mapping is identical.
  - STRIP is still slightly ahead, measured and modeled alike (57.65% vs 55.65%, 1.036× [HW]; the same in [M]). The reason is drain length, not the split: a STRIP tile drains 4 rows instead of 8.
  - That faster drain relies on the 16-lane output path, which is part of the split's area cost (below). With drain overlapped (shadow accumulators, not built), the gap would mostly vanish.
- **Model vs counters:**
  - SIM: 70 committed co-sim runs (58 distinct shape/mode/requant combinations; two clean builds of the same RTL), with 0 counter differences and `cycles_stall` = 0 on every run [SIM].
  - HW: 773 runs on the board across three builds (KG1 272, KG2 301, KG3 200). 0 of 9,276 counter values differ from the model, so the maximum per-counter error is 0. `cycles_stall` = 0 on every run [HW]. No calibration was applied. The cycle model's code and latencies have not changed since commit 72da652; the KG3 records carry a newer model hash only because the spec file's baud constant changed.
- **STRIP is not always faster (measured).** In the H-5 random batch (KG1; KG2 and KG3 gave identical cycles), 97 job indices drew the same shape in both modes. STRIP took fewer cycles on 61, the same on 5, and **more on 31**, by up to 1.32× (M = 22, K = 250, N = 17: 3,566 vs 2,711 cycles) [HW]. Every one of those 31 needs more STRIP tiles than FULL tiles. STRIP pads M to a multiple of 4 and N to a multiple of 16, while FULL pads both to 8, and on those shapes the extra padding costs more than STRIP gains. The model predicted each of those cycle counts exactly, so a mode selector can rely on it (next section).

### Auto mode: let the model pick the shape

With `--mode auto` the host scores both modes with the cycle model and runs the one with fewer predicted cycles (ties go to FULL). The model is the same frozen code as above.

- **Checked against old data first.** On KG1, 125 distinct shapes were measured in both modes (97 random + 28 benchmark). On 125/125 the model's pick had the lowest measured cycle count; 5 of them were measured ties [HW]. Across all three builds the count is 126/126 [HW].
- **Then run live on the board (KG2):** 100 random GEMMs, `run --random 100 --mode auto --seed 1`. The model picked FULL 38 times and STRIP 62 times. It predicted STRIP slower on 33 shapes, and all 33 ran FULL. All 100 were bit-exact, and measured cycles equaled the model on 100 of 100 [HW].
- **What it bought:** auto took 1,225,768 cycles in total [HW]. Running the same 100 jobs always-FULL would take 1.054× that, and always-STRIP 1.016× [MODEL: the un-chosen mode was not run]. So on this mix auto beats always-STRIP by only 1.6%: picking STRIP every time is nearly as good, and auto's value is that it never takes the 31-shape penalty above.

<!-- [HW] results/hw/logs/KG2/G5_auto_random100_seed1.log "AUTO:" lines + 100 records with mode_select; recomputed:
     FULL 38 / STRIP 62; strip-slower 33 -> FULL 33; ties 5 -> FULL; measured == model 100/100; 1,225,768 cycles.
     MODEL always-FULL 1,292,418 / 1,225,768 = 1.0544; always-STRIP 1,244,981 / 1,225,768 = 1.0157.
     125/125: storyteller recomputation from KG1 records (see HONESTY KEY), reproduces STATUS 20:44.
     126/126 pooled across KG1/KG2/KG3 tracked records (HONESTY KEY). -->

### A real tiny language model, linear layers on the board

I ran a public pretrained model, **stories260K** from `karpathy/tinyllamas` (MIT license), with the weights fetched and sha256-checked by `models/fetch_stories260k.sh`. A test checks its header: dim 64, 5 layers, 8 attention heads with 4 KV heads, hidden 172, vocab 512.

- **What runs where:**
  - **On the FPGA:** every linear layer. That is q, k, v, o, w1, w2 and w3 in each of the 5 layers, plus the classifier: 37 GEMMs per token step, int8 × int8 → int32, M = 1.
  - **On the laptop:** token embedding, RMSNorm, RoPE, attention scores and softmax, SwiGLU, int8 quantize/dequantize, and greedy sampling.
- **Every board GEMM is checked:** bit-exact against the golden model, with all 12 counters compared against the cycle model.

| Run | Build | UART / FTDI latency timer | Tokens | Linear-layer GEMMs bit-exact | Counters == model | On-chip cycles per step [HW] | Laptop wall time per step |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| 8 tokens | KG2 | 115,200 baud / 16 ms | 8 | 444 / 444 | 444 / 444 | 29,540 (295 µs) | 30.8 s |
| 30 tokens | KG2 | 115,200 baud / 16 ms | 30 | 1,258 / 1,258 | 1,258 / 1,258 | 29,540 (295 µs) | 30.5 s |
| 8 tokens | KG3 | 1,562,500 baud / 1 ms | 8 | 444 / 444 | 444 / 444 | 29,540 (295 µs) | 2.16 s |

- **Output:** prompt "Once upon a time", greedy decoding. The 30-token run on the board wrote: *"Once upon a time, there was a little girl named Lily. She loved to play outside in the park. One day,"* [HW]. Both 8-token runs, on KG2 and KG3, wrote the same text, *"Once upon a time, there was a little girl"*, which is also what the golden model generates (`docs/STATUS.md`, 21:13).
- **Utilization:** each step does 259,328 useful MACs in 29,540 cycles, which is 13.7% [HW]. Auto mode ran all 37 GEMMs in STRIP, because every one has M = 1.
- **On-chip vs wall time:** on-chip the board needs 295 µs per step, from the counters. End to end a step takes seconds, because each step sends about 245 KB over UART. Only 54,272 of the 261,888 weight bytes stay resident in the 64 KiB weight buffer; the rest are re-sent every step.
  - The two wall times differ in two settings, the baud rate and the laptop's FTDI USB latency timer, so the ratio is not a baud-only effect.
  - Wall time is laptop-side and is never accelerator speed.
- **int8 vs float:** I checked the quantized model against float32, in the golden model on the laptop, not on the board. The float32 path reproduces the model's published 256-token greedy sample exactly. The int8 path generates the same first 61 tokens and then diverges. With both paths fed the same tokens, the top-1 next token agrees on 250 of 256 steps.

<!-- [HW] results/hw/lm/2026-10-25T04-07-28Z-lm.json, 04-20-52Z, 04-59-16Z (summary blocks; see HONESTY KEY).
     30.8 = wall_s_per_step 30.761; 30.5 = 30.484; 2.16 = 2.161. 295 us = 29,540 cycles / 100 MHz = 295.4 us.
     13.7% = 259,328 / (64 x 29,540) = 0.1372. 245 KB = bytes_tx_per_step_steady 245,623.
     Timer settings: fpga/known_good/KG3/archive.txt lead_note. int8/f32: sw/crates/lm/tests/stories260k.rs output. -->

### Plots

Plots, regenerated from the JSON by `results/plots/make_plots.py`, in dark and light variants:

| | |
| --- | --- |
| ![Utilization vs M](results/plots/util_vs_m_light.png) | ![Benchmark bars](results/plots/bench_bars_light.png) |
| ![Roofline](results/plots/roofline_light.png) | ![Cost of splitting](results/plots/cost_light.png) |

Every plot is titled with its finding and labeled with its source.
- **What is drawn:** the utilization and benchmark plots show the 56 hardware benchmark points (filled markers and bars) against the model (hollow rings and ticks), and their titles say `HW`. Where the board measured a point, the identical SIM point is not drawn.
- **Roofline:** the title stays `MODEL`, because arithmetic intensity and the roofs are model quantities. Its filled points are HW.
- **Records loaded:** the plots load all 714 committed int8 hardware records, per build 255 (KG1), 276 (KG2) and 183 (KG3), and skip the 59 int32-bypass ones (`results/plots/manifest.json`). The source line under each plot gives the count per build.
- **Which record per point:** each benchmark point shows the newest record for its shape. 55 of the 56 are KG1 records, and `ffn_up` M = 1 STRIP is the KG2 check run. Repeated runs of a shape never differed on any counter, on any build [HW].

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

- **Full diagram:** [`results/plots/architecture.png`](results/plots/architecture.png), the architecture panel of dashboard Scene 3 (drawn from `docs/SPEC.md` §2 and §3.2), cropped from a screenshot of the live dashboard. The dashboard draws it with JS, so there is no standalone SVG.

  ![SplitSys block diagram: row-4 mux rewires 8×8 into 4×16](results/plots/architecture.png)
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
- **Each bitstream's RTL was signed off again before it was built:**
  - **KG2** (display-orientation fix, commit 9ec73e1): 11 of 11 suites and 64 of 64 tests; `tb/system` 625 of 625 jobs, which is the 617 above plus 8 new pin-level display cases [V].
  - **KG3** (UART at 1,562,500 baud, head ac893cd): 11 of 11 suites and 66 of 66 tests, adding tests of the UART with the real baud divider; `tb/system` 625 of 625 jobs [V].
- **Co-simulation (SIM):** the real `splitsys-host` binary drives the simulated top over a TCP-to-UART bridge. At build f402cc7 the host ran a bench sweep (all 4 GEMMs × M = 1…64 × both modes, 56 runs) plus the 10-run cosim check. At build 5900507 (same RTL) it ran the 4-run scripted demo. Results: 70 committed run records, 217,088 outputs compared, 17,014,784 MACs verified, 0 mismatches, 0 counter differences from the model [SIM].
- **On hardware [HW].** H-3 ran on the `packet_mem` link-test bitstream (build 0x52A66216). KG1 ran H-4 through H-7. KG2 re-ran H-4 and H-5, ran auto mode and the language model, and was written to flash (H-8). KG3 passed its own qualification, Q-1 to Q-8, and replaced KG2 in flash:
  - **UART link:** on KG1 (H-3), `memtest` wrote and read back 10,000 random frames (5,123,818 bytes) across WBUF, ABUF, OBUF and the requant parameters, with 0 errors and 0 retries. KG3 repeated it at 1,562,500 baud (Q-4): 0 errors, in 81 s. An echo bitstream built from the KG3 sources returned 100,000 bytes with 0 mismatched (Q-1, Q-7).
  - **Self-test:** the on-chip ROM jobs passed in both modes, and the read-back OBUF matched golden, with 0 of 256 bytes different, on all three builds (`results/hw/logs/H-4_selftest_dump.log`, `results/hw/logs/KG2/H-4_selftest_dump.log`, `results/hw/logs/KG3/Q-3_selftest_dump.log`).
  - **Random GEMMs:** 200 runs (100 per mode, up to 64×256×256, 17 with int32 output), with 0 output mismatches, 0 counter mismatches, and 0 invariant failures, on each build (`results/hw/logs/H-5_random_both_seed1.log`, `results/hw/logs/KG2/H-5_random_both_seed1.log`, `results/hw/logs/KG3/Q-5_random_both_seed1.log`).
  - **Benchmark sweep (H-6, KG1):** 56 of 56 runs passed, with a model error of +0.000% at every point (`results/hw/logs/H-6_bench_sweep_seed1.log`).
  - **Auto mode (KG2):** 100 of 100 bit-exact, with counters equal to the model (`results/hw/logs/KG2/G5_auto_random100_seed1.log`).
  - **Language model:** 2,146 linear-layer GEMMs across three runs (KG2 444 + 1,258, KG3 444), all bit-exact, with counters equal to the model (`results/hw/lm/`). These are stored separately and are not in the 773.
  - **Boots from flash:** Vivado wrote each live image to the board's QSPI flash (Micron MT25QL128, 128 Mbit, part per the schematic) and verified it. Erase, blank check, program and verify took 34 s, and the FPGA then booted from flash with DONE high (`results/hw/logs/KG2/H-8_flash_program.log`, `results/hw/logs/KG3/Q-8_flash_program.log`). With jumper J14 fitted, the board configures itself at power-on in about 1 s (observed by eye, no log). Neither Vivado nor JTAG is needed at the booth.
  - **Cold start to dashboard:** on KG3, `demo\start_demo.ps1` detected the 1,562,500 baud rate and build 21dd6850, and brought up the dashboard in 2.1 s of laptop wall time (`docs/STATUS.md`, 22:19). This is laptop-side startup time, not accelerator speed.
  - **Totals over all 773 GEMM run records:** 393,129,309 MACs verified, 3,099,658 outputs compared, 0 mismatches. Per build: KG1 272 GEMMs, 127,321,042 MACs; KG2 301 GEMMs, 159,491,513 MACs; KG3 200 GEMMs, 106,316,754 MACs. I computed these from the committed records. `results/session.json` is a live running total kept by the dashboard server and is not committed; all totals here are counted from the committed per-run records.

## Cost of splitting

Two post-route builds of the same top, with `ENABLE_STRIP = 1` and `ENABLE_STRIP = 0` (Vivado 2025.2.1, xc7s50csga324-2, S4 freeze rebuild at git 38a3950a, build 0x38A3950A for both) [B]:

| | FULL-only | Split | Delta |
| --- | ---: | ---: | ---: |
| LUT | 5,146 | 7,389 | +2,243 (+43.6%) |
| FF | 6,346 | 7,936 | +1,590 (+25.1%) |
| DSP48E1 | 80 | 96 | +16 (+20%) |
| BRAM | 26 | 26 | 0 |
| WNS at 100 MHz | +2.188 ns | +0.744 ns | both meet timing |

**Where the delta lives** (`breakdown.delta` in cost.json) [B]:

| Block | LUT | FF | DSP |
| --- | ---: | ---: | ---: |
| Array STRIP muxes | +378 | +264 | 0 |
| Sequencer STRIP control | +289 | +70 | 0 |
| Requant lanes 8–15 | +1,503 | +1,216 | +16 |
| Everything else | +73 | +40 | 0 |

<!-- [B] results/cost.json (git 38a3950a, committed 23acf6a): full_only/split lut 5146/7389, ff 6346/7936,
     dsp 80/96, bram 26/26, wns_ns 2.188/0.744, fmax_mhz 128.0/108.0; breakdown.delta array 378/264/0,
     sequencer 289/70/0, requant 1503/1216/16, total 2243/1590/16.
     "Everything else" = total - (array + requant + sequencer) = 2243-2170 = 73 LUT, 1590-1550 = 40 FF.
     % = delta / full_only: 2243/5146 = 43.6%, 1590/6346 = 25.1%, 16/80 = 20%.
     Split proper = 378+289 = 667 LUT, 264+70 = 334 FF.
     worst_path.split: rst_core_reg_rep__7/C -> u_core/u_rq/g_lane[0].q_e5_reg[39]/R, logic_levels 0, route_pct 94.967.
     Same datapath, other builds: git 8bbd7ba6 split WNS +1.143; git 52a66216 (KG1) +0.523; 38a3950a (KG2) +0.744;
     21dd6850 (KG3, fpga/reports/kg3_top/summary.json) +0.473, lut 7377, ff 7928, dsp 96, bram 26.
     Split WNS spread across the four: 1.143 - 0.473 = 0.67 ns. -->

**Read it honestly:**
- **Most of the delta is the 16-wide output path,** not the split itself. Requant lanes 8–15 are 1,503 of the 2,243 LUTs and all 16 DSPs. The FULL-only build trims them, because FULL drains only 8 lanes per row.
- **The split proper** (array muxes plus sequencer control) costs +667 LUT and +334 FF.
- **Don't read the Fmax estimate as a cost of splitting.** From WNS at the 100 MHz constraint, the estimates are 128.0 MHz (FULL-only) and 108.0 MHz (split). They are not a frequency sweep. The split build's worst path starts at a replicated reset flop (`rst_core_reg_rep__7`, ending at a requant lane-0 register reset): 0 logic levels, 95% routing. That is placement, not STRIP logic. Four builds of the same datapath RTL closed the split design at WNS +1.143 ns (8bbd7ba6), +0.523 ns (52a66216, KG1), +0.744 ns (38a3950a, KG2) and +0.473 ns (21dd6850, KG3): a 0.67 ns spread with no datapath change. I headline the LUT, FF and DSP counts, not Fmax.
- **Which build these numbers describe.** They come from the S4 freeze rebuild at 38a3950a (cost.json committed in 23acf6a), whose split build is KG2.
  - **KG3, the bitstream on the board now,** differs from KG2 only in the UART baud divider constant. Its own post-route report (`fpga/reports/kg3_top/summary.json`) shows 7,377 LUTs, 7,928 FFs, 96 DSPs and 26 BRAM, which is 12 LUTs and 8 FFs fewer than KG2.
  - That is a separate implementation run with a different divider constant, and I don't attribute the difference to anything. I did not rebuild a FULL-only twin at KG3, so the delta table stays at 38a3950a.

## Reproduce

Run from the repo root. On Windows, use Git Bash and run Linux tools through `scripts/wsl`.

```bash
scripts/wsl 'scripts/check_env.sh'                                   # toolchain present
scripts/wsl 'cd sw && cargo test --workspace'                        # golden, cycle model, host, LM
scripts/wsl 'make -C tb -j12 signoff'                                # unit + core sign-off suites
scripts/wsl 'make -C tb system'                                      # whole top through its pins
scripts/wsl 'make -C tb cosim-check'                                 # real host vs simulated top -> results/sim
scripts/wsl 'cd sw && cargo run -p splitsys-cycle -- all'            # regenerate results/model
scripts/wsl 'python results/plots/make_plots.py --check'             # regenerate plots (deterministic)
bash fpga/tcl/s4_rebuild.sh --no-commit                              # all 4 bitstreams at one hash + results/cost.json
bash models/fetch_stories260k.sh                                     # stories260K weights (pinned revision + sha256)
scripts/wsl 'cd sw && SPLITSYS_LM_REQUIRE_MODEL=1 cargo test -p splitsys-lm --test stories260k'  # fail, not SKIP, if weights missing
```

On hardware (Windows; the board enumerated as COM3 in my session). `--baud auto` probes 1,562,500 (KG3) and then 115,200 (KG2):

```bash
splitsys-host --transport serial:COM3 --baud auto memtest --frames 10000 --seed 1        # link test
splitsys-host --transport serial:COM3 --baud auto selftest --dump                        # on-chip self-test
splitsys-host --transport serial:COM3 --baud auto run --random 100 --mode both --seed 1  # random batch
splitsys-host --transport serial:COM3 --baud auto run --random 100 --mode auto --seed 1  # model picks the mode
splitsys-host --transport serial:COM3 --baud auto bench sweep --seed 1                   # benchmark sweep
splitsys-host --transport serial:COM3 --baud auto lm --tokens 8                          # stories260K, linear layers on the board
scripts/vivado -mode batch -source fpga/tcl/flash.tcl -tclargs fpga/known_good/CURRENT   # write + verify QSPI flash (J14 fitted)
```

The ordered bring-up and qualification scripts, with expected output and failure branches, are in `docs/HW_CHECKLIST.md`. `serve` runs the live dashboard.

## Limits and non-goals

- **No speed claim against CPUs or GPUs.** The claim is about utilization, operand bandwidth per MAC, and whether the model predicts the counters.
- **End-to-end time is UART-bound** (1,562,500 baud on KG3, 115,200 on KG2). Wall-clock time is never reported as accelerator performance. On-chip cycles come from the counters.
- **Tiny models.** The benchmark block has d_model = 64 and uses synthetic int8 data with a fixed seed. The language model is real but tiny (dim 64, 5 layers), and only its linear layers run on the board. The laptop runs embedding, norms, RoPE, attention scores and softmax, SwiGLU, and sampling. No DDR3: weights live in on-chip BRAM, and the language model's weights are re-sent over UART every token.
- **int8 changes the model's output.** The int8 path matches float32 for 61 generated tokens, then diverges (golden model, not board).
- **No overlap between stages.** STREAM, FLUSH, and DRAIN run back to back, and the array idles during drain. That fixed per-tile overhead is why prefill utilization tops out near 56–58% for K = 64 [M].
- **Zero stalls, but only because there is no DRAM.** The model assumes zero stall cycles. Co-simulation measured 0 on every run [SIM], and so did the board, on all 773 runs [HW]. That shows only that on-chip BRAM keeps up at 20 B/cycle. There is no DRAM path, so a memory-bound regime was never tested.
- **INDEP mode** (four independent 4×4 jobs) is specified but not built.

## What's next

- Shadow accumulators, so that drain overlaps the next tile's stream.
- Loading requant parameters during STREAM instead of in a 16-cycle setup state [S].
- INDEP mode for per-head attention, so attention scores can move onto the board too.
- DDR3 weight streaming, to study memory-bound decode, where FULL's 16 B/cycle (against STRIP's 20) could make it the right pick. Auto mode would then need a bandwidth term, not just tile counts.

## Built at Cal Hacks 13.0

San Francisco, October 2026. Solo entry. All project code was written during the event.

Built with: Rust · SystemVerilog · Verilator · cocotb · Python · matplotlib · Vivado 2025.2.1 · AMD Spartan-7 XC7S50 (RealDigital Urbana) · vanilla JavaScript / SVG · karpathy/tinyllamas stories260K (MIT) · Claude Code (AI coding agents)
