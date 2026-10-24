# SplitSys Status (kept by the lead)

**Phase:** S (simulation-first, no board) · **Current gate:** S4 prep (target 16:00) · S0 **PASS** (~09:35) · S1 **PASS** (~10:15) · **Last update:** Sat 11:20 (real clock)

> **Correction (11:20):** the times in the handoff log from 09:20 to "16:10" were lead estimates, not clock readings, and are wrong (they ran ~5 h fast). Real times are the commit times in `git log`. Actual sequence: S0 PASS ≈ 09:35, S1 PASS ≈ 10:15, clean system sign-off 10:52, last commit before a Wi-Fi outage 10:55. Log entries after this note use the real clock.

## ▶ RESUME HERE (paused Sat 12:12 for an account switch)

**State at pause:** S0 PASS, S1 PASS. S2/S3 evidence complete and clean (tb signoff 11/11 64/64; tb/system SIGNOFF=1 617/617 at f402cc7; 66 clean sim records, 0 mismatches, counters == model), but red-team phase 2 re-runs not done. S4 FAIL as-is (fixable). RTL frozen at adca4a3. HEAD = 52a6621 (fpga hardened s4_rebuild.sh: pins BUILD_ID to start HEAD, fails if HEAD moves). Windows host build works (human installed Rust; lead verified mock ping/selftest).

**Working tree at pause (uncommitted, owned by agents that were paused):**
- `fpga/reports/*` modified = leftovers of the FAILED first S4 rebuild — will be overwritten by the rerun (don't commit them).
- `README.md`, `docs/devpost.md`, `docs/DEMO.md` = demo-storyteller's partial red-team fixes. `demo/app.js`, `demo/style.css` = host-dashboard's red-team fixes (q0..q3 rows, int32 labels). `docs/ENV.md` = toolchain-engineer's WSL-restart notes. `docs/STATUS.md` = lead. Check each agent's pause reply (below) before committing.

**Next steps, in order (new session: respawn agents as general-purpose with "You are <role>; read .claude/agents/<role>.md", or as named agents if registered):**
1. ~~S4 rebuild~~ **DONE (12:12): 8ccb333**, all 6 bitstreams at build_id 0x52A66216, every tag pass/clean/DIV_OVERRIDE 0; top WNS +0.523 / nostrip +2.097 / heartbeat +5.340 / heartbeat_uartswap +5.229 / packet_mem +3.194 / packet_mem_uartswap +1.936 ns. Manifest in HW_CHECKLIST, cost.json regenerated. **Caveat:** top WNS has varied +1.143 → +0.849 → +0.523 across rebuilds with no RTL change (placement noise ~0.6 ns; worst paths reset / int32_q fan-out) — any future rebuild may land near zero; consider `max_fanout` on sequencer int32_q only if a rebuild is needed anyway. Commit freeze is lifted. Leftover `fpga/reports/*` diffs listed above are gone (rebuild committed them). toolchain-engineer's uncommitted `scripts/wsl_keepalive` (new, needs +x), `scripts/check_env.sh`, `docs/ENV.md` are ready to commit (WSL idle-shutdown fix; human may add `C:\Users\danie\.wslconfig` `[general] instanceIdleTimeout=-1`, applies after `wsl --shutdown`). After the switch: `scripts/wsl_keepalive start`.
2. After the rebuild commit: host-dashboard commits its app.js fixes (all 12 counters in the model-error table; int32-bypass runs labeled and excluded from headlines) and re-shoots demo/screens against `make -C tb cosim` (results under ~/.cache, never /tmp).
3. demo-storyteller finishes red-team items (mutant claim limited to suites SIGNOFF.md records; SIM provenance = the 66 clean records at f402cc7, archive_dirty never cited; MatX HBM+SRAM cited or dropped; D-008 "caught in spec review"; UART swap "unconfirmed until H-2"), refreshes cost from the rebuilt cost.json, regenerates plots, commits.
4. toolchain-engineer: WSL self-restart mitigation (keepalive / .wslconfig snippet for the human), commit.
5. red-team-reviewer: S2+S3 re-runs (`SPLITSYS_CACHE_TAG=redteam make -C tb -j12 signoff`, `make -C tb/system SIGNOFF=1 SEED=1 WORKERS=18`, `make -C tb cosim-check RESULTS=~/.cache/...`) + S4 hardware-readiness + claims re-audit → verdicts.
6. Optional: run the Windows `sw\target\release\splitsys-host.exe --transport tcp:127.0.0.1:5555 ping|selftest` against `make -C tb cosim` in WSL (exact demo binary vs RTL).
7. On S4 PASS: begin Phase H with the human using docs/HW_CHECKLIST.md (one physical action per message).

**Open human items:** solo vs team (Best Solo eligibility), repo/video URLs, table number before Sun 09:00. Always read the real clock (`date`) before logging times.

**Canonical RTL lint until top.sv exists:** `scripts/wsl 'verilator --lint-only -Wall -Wno-MULTITOP -Irtl -Irtl/gen rtl/gen/splitsys_pkg.sv rtl/*.sv'`

States: `todo` · `running` · `done` (owner handoff, lead checked) · `PASS` (red-team re-ran it) · `FAIL` · `blocked` · `cut`

## Gate board

| Gate | Item | Owner | State | Evidence command |
| --- | --- | --- | --- | --- |
| S0 | Repo skeleton, git init, .gitignore/.gitattributes, Cargo workspace stubs | lead | done | `git log --oneline -1`; `ls sw/crates` |
| S0 | WSL inventory, installs, Vivado batch smoke, part confirmed, XDC located, docs/ENV.md | toolchain-engineer | done (09:25) | `scripts/wsl 'scripts/check_env.sh'` |
| S0 | spec/spec.toml + codegen → rtl/gen/splitsys_pkg.sv + Rust consts | spec-steward | done (09:35) | `cd sw && cargo test -p splitsys-spec`; `verilator --lint-only -Wall rtl/gen/splitsys_pkg.sv` |
| S0 | Golden: GEMM, requant, packing, Fletcher-32, descriptor + property tests; vectors | golden-modeler | done (10:10) | `cd sw && cargo test -p splitsys-golden -p splitsys-vec` |
| S1 | Cycle model per SPEC §4 (started early, parallel lane) | perf-modeler | done (09:20; latencies provisional) | `scripts/wsl 'cd sw && cargo test -p splitsys-cycle'` |
| S0 | Gate review | red-team-reviewer | **PASS** (11:00, clean clone @ d4453d0) | all S0 evidence re-run; findings routed (see log) |
| S1 | RTL datapath: pe, skew, array (FULL+STRIP), buffers, requant | rtl-designer (A) | done (10:25; awaiting verif sign-off) | `verilator --lint-only -Wall -Irtl -Irtl/gen rtl/gen/splitsys_pkg.sv rtl/*.sv` |
| S2 | buffers port-A → word+BE; sequencer.sv; core.sv (buffers+seq+array+requant) | rtl-designer (A) | done (12:10; 2148101, 3786ad1) | core smoke: 283 golden jobs bit-exact, counters == model |
| S2 | RTL board_io (LEDs, 7-seg, reset root) | rtl-designer (B) | done (10:45; e3f6bc5) | lint + smoke |
| S2 | selftest.sv + top.sv | rtl-designer (B) | done (12:05; 6171191, 35fb363) | top smoke through UART pins (80 checks) |
| S4 | packet_mem bring-up top + schematic facts (UART dir, BTN polarity); build.tcl ready for top | fpga-integrator | done (11:30; packet_mem WNS +2.692 ns) | `fpga/reports/packet_mem/summary.json` |
| S4 | UART-swap fallback bitstream, board-repo warning fix, HW_CHECKLIST H-4..H-7 drafts, full top + cost build when core is wired | fpga-integrator | done (13:00; top WNS +0.526, nostrip +1.834; cost.json) | `fpga/reports/top/summary.json`, `results/cost.json` |
| S1 | RTL host link: uart_rx/tx, packet, counters | rtl-designer (B) | done (10:00; awaiting verif sign-off) | same lint |
| S1 | Unit testbenches + sign-off (8 units) | verification-engineer | done (12:00; 8/8 suites, 50/50, SEED 1+2) | `scripts/wsl 'make -C tb -j8 units'` |
| S1 | Gate review | red-team-reviewer | **PASS** (12:40, clean clone @ 35fb363) | units 50/50 SEED 1+2, heartbeat rebuild WNS 5.426, XDC audit, cargo --locked |
| S2 | tb/core (level-2: core without UART, counters vs model), oracle descvalid | verification-engineer (1) | done (14:30) | `scripts/wsl 'SPLITSYS_CACHE_TAG=<you> make -C tb -j12 signoff'` |
| S1 | Build scripts + bringup/heartbeat_uart_echo timing-clean, HW_CHECKLIST skeleton | fpga-integrator | done (10:30; WNS +5.671 ns) | `fpga/reports/heartbeat_uart_echo/summary.json` WNS ≥ 0 |
| S1 | Host framing/CRC lib, transports, mock board, dashboard skeleton (MODEL) | host-dashboard | done (11:15) | `scripts/wsl 'cd sw && cargo test -p splitsys-host'` (41 pass) |
| S2/S3 | tb/system (200 FULL GEMMs, STRIP, mode switch, self-test; counters == model) + tb/cosim TCP bridge | verification-engineer (2) | done (14:05; 617/617, cosim 8/8) — clean SIGNOFF=1 pending verif-1 commits | `make -C tb/system`; `make -C tb/cosim` + `splitsys-host --transport tcp:...` |
| S2 | FULL system sim, 200 random GEMMs bit-exact, counters == model; FULL top bitstream | critical path | todo | `make -C tb system` |
| S3 | STRIP bit-exact, co-sim with real host, dashboard scenes 1–3 | critical path / host-dashboard | done (14:45) — awaiting verif-2 clean SIGNOFF, then red-team S2+S3 | `make -C tb cosim` + `splitsys-host --transport tcp:...` |
| S4 | README/Devpost/pitch skeletons + plots (MODEL/BUILD only, ⟦placeholders⟧) | demo-storyteller | done (13:35; b33c984, 72cf56d) | `scripts/wsl 'python results/plots/make_plots.py --check'` |
| S4 | All bring-up bitstreams at freeze, cost build, HW_CHECKLIST, start_demo | fpga-integrator | in progress | red-team S4 |

## Decisions and notes (lead)

- Sat 01:55 — Kit was unpacked into `splitsys-claude-code-kit (1)/`. Moved its contents to the repo root (`C:\Users\danie\FPGA_Projects\Calhack2026`, WSL `/mnt/c/Users/danie/FPGA_Projects/Calhack2026`) because the space/parens path would break Make and Vivado Tcl. The original zip is kept.
- Sat 01:55 — Repo lives on the Windows drive so Windows Vivado and Claude's file tools can reach it. Heavy build outputs (cargo target, Verilator obj dirs) should go to WSL-native paths for speed (toolchain-engineer decides via `scripts/env.sh`).
- Sat 01:55 — Lead probe: WSL Ubuntu 24.04, 24 cores, 15 GB RAM. gcc/make/curl/python3-venv present; socat, unzip, verilator, cargo missing. `vivado` on PATH is `/mnt/c/Xilinx/2025.1/...`; both 2025.1 and 2025.2.1 are installed. No `/usr/local/bin/vivado` wrapper exists (CLAUDE.md assumed one).
- Sat 01:55 — Lead installed rustup stable (user-level) in WSL to unblock the three Rust lanes; toolchain-engineer verifies and records it.

- Sat 08:55 — **Laptop was in Modern Standby 02:02–08:53** (Windows Kernel-Power 506/507). All six agents paused ~7 h and resumed at 08:53. Every gate shifts by ~7 h; S0 (03:30) and S1 (07:30) are already late. Asked the human to disable sleep while plugged in.

## Handoff log (one line each)

| Time | Agent | Status | Summary |
| --- | --- | --- | --- |
| 09:20 | perf-modeler | done | Cycle crate (8aeedea) + results/model/{sweep,roofline,README} (482acd8). 20 tests, clippy clean, walk == closed form over 10k descriptors. Latencies provisional. Bypass-flag + FULL int32 drain questions routed to spec-steward. |
| 09:25 | toolchain-engineer | done | check_env ALL REQUIRED OK. Verilator 5.052, cocotb 2.1.0, Vivado 2025.2.1 via scripts/vivado (xc7s50csga324-2), Urbana XDC from Downloads (100 MHz N15, UART B16/A16). Venv + cargo target in ~/.cache/splitsys. scripts/screenshot (headless Edge). |
| 09:35 | spec-steward | done | spec.toml + codegen, SPEC v1.1, DECISIONS D-001..D-015 (219fbc2). 36 tests, clippy clean, Verilator -Wall lint clean, deterministic. Ruled: flags 0x01/0x02 only; int32 drain 16 both modes; phase→counter mapping normative; BENCH table exported; 921600 baud impossible at 100 MHz (D-014). Latencies all provisional. Follow-ups routed to perf, golden, rtl A/B, host, verif. |
| 09:50 | perf-modeler | done | Migrated to SPEC v1.1 (37d0b2a, results 925307f): int32 drain 16/16, BENCH_SHAPES from spec, flags 0x01/0x02 only. 21 tests, clippy clean. Int8 numbers unchanged (still provisional latencies). Pending for spec-steward: delete legacy INT32_DRAIN_FACTOR (batch with latency confirmations). |
| 10:00 | rtl-designer B | done | uart_rx/tx, counters, packet (35f5504, 55fbe69, 0cfa381). Verilator -Wall 0 warnings; Verilator packet smoke 578 checks 0 errors; byte-enable mutation caught. Not synthesized yet. Notes: packet strictly request/response; top must wire run_accept and st_done. Next: board_io.sv. |
| 10:10 | golden-modeler | done | Golden + vec (e1fa7ae..7f160d3): 24 golden + 6 vec tests, clippy clean, 14 output files byte-identical across runs; requant vs exact i128 reference (400k incl. ties). Self-test ROM format defined (464 words, 1 BRAM36; FULL 6c1d81af, STRIP 8b1c6e80). Bench data synthetic (fixed seed) — must be labeled. |
| 10:25 | rtl-designer A | done | pe/skew/array/buffers/requant (f473464..36771b6). Verilator -Wall clean; smoke tests incl. quadrant isolation, mode switch, 112k requant checks (mutation caught). Vivado OOC: 64 DSP array + 32 DSP requant, WBUF 16 RAMB36, positive slack. L_PE=3, L_RD=2 confirmed; L_RQ=7 (not 4). Lead decision: buffers host port changes to word+BE to match packet.sv. |
| 10:30 | fpga-integrator | done | build.tcl/program.tcl, urbana_splitsys.xdc, heartbeat_uart_echo: sim smoke pass; Vivado exit 0, WNS +5.671 ns, 0 DRC, 190 LUT. Clock 100 MHz N15 confirmed. HW_CHECKLIST H-1/H-2 written. Open: UART direction, BTN polarity, 7-seg order → reading schematic. |
| 10:45 | rtl-designer B | done | board_io.sv (e3f6bc5): Verilator -Wall clean, smoke pass (iverilog + Verilator). Reset root (POR + BTN1 + ext). ASSUMED params: BTN_ACTIVE_LOW=0 (if wrong → board stuck in reset), LEFT_IS_D1=1, AN0_IS_RIGHT=1 — pending schematic/H-1. 7-seg shows host SET_DISPLAY value (no HW divider). |
| 10:50 | spec-steward | done | D-016 (d6e3bcf): L_PE=3, L_RD=2 confirmed; L_RQ=7 (sequencer-level; requant.sv alone = 6); INT32_DRAIN_FACTOR removed. BUG-001 resolved by D-016. Cycle-model regeneration deferred until the sequencer fixes L_SETUP/L_ACCEPT/L_DONE/L_WB (one regen, not two). |
| 11:00 | red-team-reviewer | **S0 PASS** | check_env, Vivado smoke, XDC verbatim vendor copy, gen --check (mutation proves it real), spec/golden/vec/cycle tests, workspace test + clippy all reproduced. Findings: SEV-2 results/model stale vs D-016 → perf-modeler regenerating; SEV-2 requant test must use L_RQ-1 → verif; SEV-3 drain-boundary zero slack → rtl A + verif + perf; Cargo.lock + mock results location → host; .gitignore → lead fixed (9fd6577); run.sh uncommitted → fpga; SPEC 921600 text, Fletcher check consts, SV/Rust name diffs, vec provenance header → batched for spec-steward/golden with sequencer latencies. |
| 11:10 | perf-modeler | done | results/model regenerated for D-016 (6da3f98, model hash 8efa5ac): ffn_up M=1 FULL 7.0% / STRIP 14.4% (2.07×), M=64 55.6/57.7% — MODEL, provisional latencies. Drain boundary: model arrival now s+L_RD+1+k (matches sequencer lane-select reg); check is acc_final <= first DRAIN cycle (matches array.sv); test proves zero slack at PE(7,7) and that FLUSH−1 is caught. Batch for spec-steward: state "last word reaches array in first FLUSH cycle" in SPEC §3.5. |
| 11:15 | host-dashboard | done | splitsys-host + demo (95d56b1, fbf7c94, a7bc8a5): 41 tests, clippy clean, --locked ok; mock ping/selftest/run/bench/serve work; scenes 1–3 + disconnected screenshots reviewed (lead viewed scene1: honest MODEL/DEMO DATA labels). Mock records → results/mock (gitignored, never counted as verified). SET_DISPLAY after every run. Not verified: Windows build, start_demo.ps1, real serial, cosim. |
| 11:30 | fpga-integrator | done | Schematic facts (via pypdf text): **UART pins were swapped** — FPGA RX = B16, TX = A16 (master XDC names are bridge-side; FT2232H pin function from memory → fallback swap bitstream ordered); buttons/LEDs active high; 7-seg active low; 100 MHz N15. heartbeat_uart_echo rebuilt (WNS +5.726). packet_mem: sim 328 checks pass, build WNS +2.692, 25 BRAM. build.tcl ready for top (trial correctly fails DSP 0<64 — core not yet wired). H-3 written; needs host `memtest` (ordered). |
| 11:40 | host-dashboard | done | `splitsys-host memtest --frames N --seed S` (5d1abe2): mock 2000 frames + tcp 300 frames pass; injected bit flip caught; 46 tests, clippy clean. WRITE_CHUNK now 1019. Lead removed a stray self-referential symlink sw/sw → sw (created 09:24, untracked). |
| 11:50 | toolchain-engineer | done | 6d55b84: board facts in ENV.md (UART direction caveat: FT2232H pin function from memory, unverified until H-2); scripts/screenshot fixed (root cause: virtual-time budget never elapses on SSE pages) — per-run profile, 30 s hard timeout, narrow kill, --kill-stale. |

| 12:00 | verification-engineer 1 | done | **S1 units signed off**: pe, skew, uart, buffers (new word+BE port), requant, counters, packet, array FULL+STRIP (ENABLE_STRIP 1 and 0). 50/50 tests SEED 1 and 2, 83 s. Mutants caught in every suite. Harness bug fixed (make -j could skip sim and exit 0). Expected values from Rust oracle/vec/cycle only. No open RTL bugs. Spec questions (request/response, RX timeout boundary) → spec-steward. |

| 12:05 | rtl-designer B | done | selftest.sv (6171191) + top.sv (35fb363) with core wired: Verilator -Wall clean, no MULTITOP (also ENABLE_STRIP=0). tb_top through UART pins: 80 checks 0 errors — PING, BTN0 self-test 6c1d81af/8b1c6e80 pass both, SELFTEST opcode, 2 golden directed jobs bit-exact (FULL 9x33x17, STRIP 7x17x15), counter invariants, SET_DISPLAY. Not synthesized yet. |

| 12:10 | rtl-designer A | done | buffers word+BE port (2148101), sequencer + core (3786ad1): Verilator tb_core 283 golden jobs (FULL/STRIP, int8/int32, edges, K=1/256, boundary) OBUF bit-exact + all 12 counters == splitsys-cycle predict; STRIP_EN=0 175 jobs. Real latencies = spec values (L_SETUP 16, L_RQ 7, L_WB 1, L_ACCEPT 1, L_DONE 1). Drain margin worst 3 cycles (mutation-confirmed). Core OOC: 96 DSP, 25 RAMB36, 5.4k LUT, WNS +3.98. Cost caveat: STRIP_EN=0 also trims 16 requant DSP → cost.json must break down the delta (routed). Possible later optimization: overlap RQ load with STREAM (L_SETUP 16→~2, ~12% of decode T_tile) — NOT before G4. |

| 12:20 | spec-steward | done | D-017 all 7 latencies provisional=false (values = RTL); D-018 SV/Rust name parity enforced by test; D-019 strict request/response + exact RX timeout; OPERAND_PORT_OFFSET = L_RD+1 in spec; 921600 text fixed; Fletcher ABCDEFGH const (7140392, eef4493). 38 tests. |
| 12:20 | golden-modeler | done | packet vectors: back_to_back labeled parser-level, new request_response set (928ed42); canonical vec outputs re-stamped at 7140392, data sha256 unchanged (0c516e0). |
| 12:20 | lead | — | Launched demo-storyteller (README/Devpost/pitch skeletons with ⟦placeholders⟧, MODEL-only plots). perf-modeler regenerating with latencies_provisional=false. |

| 12:30 | perf-modeler | done | Model uses spec OPERAND_PORT_OFFSET (90939f4); results regenerated with latencies_provisional=false (4617b16), numbers unchanged (ffn_up M=1 7.0%/14.4%, M=64 55.6%/57.7% — MODEL). Freeze after sim reconciliation, before hw benchmarks. |

| 12:40 | red-team-reviewer | **S1 PASS** | Reproduced in clean clone with isolated cache: 8/8 suites 50/50 (SEED 1, 2); harness re-sim + missing-results refusal verified; heartbeat rebuilt WNS 5.426, io.rpt UART RX=B16 TX=A16; 43 other pins match master XDC; cargo --locked ok; all S0 follow-ups addressed. SEV-2: sign-off pins rtl only, not tb (+ shared cache collisions) → verif 1. SEV-3: UART direction confirm at H-2 (fallback ready); model_git_hash should be last input commit → perf. |

| 12:50 | perf-modeler | done | model_git_hash = last commit touching model inputs (72da65248adf, d21988b); results-only commits no longer stale it. 24 tests, numbers unchanged. |

| 13:00 | fpga-integrator | done | **Full top timing-clean @35fb363f**: WNS +0.526 / WHS +0.025, 7378 LUT, 7778 FF, 96 DSP, 26 BRAM, self-test ROM read OK (RAMB18). top_nostrip: WNS +1.834, 5181 LUT, 80 DSP. results/cost.json (hw-build) with breakdown: ΔLUT +2197 (requant +1488, array +374, sequencer +289), ΔDSP +16 (all requant); Fmax delta flagged as placement noise, not to be headlined. Worst top paths = reset fan-out (1 flop → 7.8k) → rtl B fixing in top.sv. UART-swap fallbacks built (heartbeat, packet_mem); board-repo warnings gone; HW_CHECKLIST H-1..H-7 drafted. Must rebuild top at S4 freeze. |

| 13:15 | rtl-designer B | done | Reset fan-out fix (adca4a3): 5 per-block reset regs in top.sv (keep, max_fanout 256, INIT 1), board_io rst_r max_fanout 50; all blocks leave reset together 1 cycle after board_io rst. Lint clean both builds; tb_top 80/80, selftest, board_io smokes pass. Timing gain pending fpga rebuild. |

| 13:25 | host-dashboard | done | H-5 batch: `run --random N --mode full|strip|both --seed S` (93f0944) — same generator as splitsys-vec gemm, OBUF poisoning, bit-exact + 12 counters vs model + invariants, first-failure line carries the sim repro command. 50 tests; mock 40 runs + tcp 5 runs BATCH PASS; fault injection caught. memtest prints per-region frame counts. (sw/sw symlink confirmed gone.) |

| 13:35 | demo-storyteller | done | README, docs/devpost.md, DEMO §7–§9 (pitches 246/224/242 words), plots (21 files deterministic). Every number traced to MODEL @72da65248adf, BUILD cost.json @35fb363f, SIGNOFF, or SPEC; HW/SIM as ⟦placeholders⟧. Cost told honestly (split muxing ≈663 LUT; rest is 16-lane requant). Open: host model_git_hash mismatch → host; matplotlib pins → toolchain; red-team audit at S4; ⟦HUMAN⟧ items (team size, table #, repo/video URLs, photo). |
| 13:35 | verification-engineer 2 | (commit, handoff pending) | 6bff698: tb/system 617/617 SEED=1 at rtl 7140392 (258 s); tb/cosim 8/8 real-host steps (ping, selftest, run strip/full/both/int32, bench smoke); results/sim committed. |

| 13:45 | toolchain-engineer | done | Plotting deps pinned (d1867c8): requirements.txt = full venv freeze (matplotlib 3.9.2, numpy 2.5.4, …); check_env requires them; fresh venv reproduces exactly. check_env ALL REQUIRED OK. |

| 13:55 | host-dashboard | done | Hash provenance (1a19134): git_hash = host build-time HEAD (build.rs), model.model_git_hash = splitsys_cycle::report::model_git_hash() (72da65248adf). 51 tests. results/sim from 6bff698 must be regenerated → verif 2. |

| 14:05 | verification-engineer 2 | done | tb/system 617/617 at rtl adca4a3 (SEED 1, 2, 3; RAND_RESET): 200 FULL + 100 STRIP random up to 64x256x256, 262 directed both modes (single nonzero every PE incl. 4+c, quadrant isolation, sat, ties, int32), 52 mode-switch, self-test opcode+BTN0+BTN1, BUSY behaviour; every frame, output, and all 12 counters from Rust; 2 mutants caught. tb/cosim: real splitsys-host 8/8 (ping, selftest, run both modes, int32, bench smoke) — 10 GEMMs 0 mismatches, model err 0.000% every counter; results/sim regenerated (3f3b9d1) with correct hashes. SIGNOFF=1 refuses until verif-1 commits tb edits. |

| 14:30 | verification-engineer 1 | done | Clean signoff @b53783d (tb ddc358f): 11/11 suites 64/64, eligible YES, refuses dirty inputs. tb/core: 262 directed + 64 random + mode-switch chains + PE(7,7) boundary — OBUF bit-exact, 12/12 counters == model on every job (independent D-017 confirmation); FLUSH±1 mutants caught; core_ns passes. Red-team SEV-2 fixed (provenance.mk, SPLITSYS_CACHE_TAG). Packet: golden descvalid, RX-timeout exact edge, strict req/resp discard. |

| 14:45 | host-dashboard | done | Dashboard on cosim RTL (7134a65): 37 sim records, all bit-exact, model err 0.0 on every counter; ffn_up M=1 FULL 6.95% / STRIP 14.40%, M=64 55.65% / 57.65% (SIM). Scenes 1–3 screenshots reviewed (lead viewed scene 2: SIM badges, model markers, decode shading). Cost panel BUILD-badged with breakdown, no Fmax headline; VERIFIED says "in RTL simulation". Note for S4 claims audit: error table omits mac_useful_q0..3 rows. |

| 15:00 | fpga-integrator | done | Pre-freeze rebuild @8bbd7ba6 (RTL adca4a3) with PHYS_OPT=AggressiveFanoutOpt: top WNS +1.143 / WHS +0.021, 7388 LUT, 96 DSP, 26 BRAM; nostrip WNS +2.441. cost.json refreshed (ΔLUT +2239, ΔDSP +16). Next limiter: sequencer int32_q fan-out. HW_CHECKLIST H-3/H-4/H-5 updated. |
| 15:00 | lead | decision | **RTL frozen at adca4a3 for Phase H** (bug fixes only). No int32_q fan-out tweak (+1.1 ns slack is enough; avoids a re-verify cycle). Keep "every register reset" coding rule (no reset removal on data regs). Final S4 rebuild of ALL bitstreams at one commit after S4 docs land, so build_ids match. Launched S4 prep: verif-1 checklist triage rows, toolchain Windows build instructions, storyteller SIM fill + cost refresh. |

| 15:15 | verification-engineer 1 | done | HW_CHECKLIST triage T-1..T-4 (30475dc): self-test fail, GEMM mismatch (exact sim repro via RTL_REV=<build_id>, ONLY=<job>), counters-only mismatch (never tweak; perf reconciliation), sim-pass/HW-fail suspects in order. Two repro commands verified at 8bbd7ba6. Gap → host `dump`/self-test diff command (ordered). |

| 15:25 | toolchain-engineer | done | Windows host build (918af35): Windows has VS Build Tools 2026 + SDK already; only Rust missing. Pinned 1.99.1-x86_64-pc-windows-msvc (~124 MB total). scripts/win_build_host.ps1 builds to sw	arget (matches start_demo.ps1), checks --help, lists COM ports. WSL cross `cargo check --target x86_64-pc-windows-msvc` passes. Human asked to install Windows Rust now. |

| 15:35 | host-dashboard | done | Triage tools (e7ad5bf): `dump --region … --addr --len --out`, `selftest --dump` (per-byte diff → C[m,n], tile, PE, quadrant; read-back checksum vs ROM). Cosim: both self-test jobs 0 differing bytes. Self-test OBUF ranges proven non-overlapping (test). Batch first-failure now carries PE/quadrant. 56 tests. |

| 15:50 | demo-storyteller | done | S4 pass (4de1602): ⟦SIM⟧ slots filled from committed results/sim (46 records, 30 shapes, 181,632 outputs, 0 mismatches, 0 counter diffs), cost refreshed to 8bbd7ba6, SIM≠HW framing everywhere, plots use tracked records only. Found: all committed sim records are git_hash -dirty → lead ordered a final clean evidence run (verif 2): clean tree, rebuilt host, SIGNOFF=1, regenerated sweep, dirty records git-mv’d to results/sim/archive_dirty/. |

| 16:00 | verification-engineer 2 | done (sign-off) | **Clean tb/system SIGNOFF=1: 617/617, eligible YES** (head 30475dc, rtl adca4a3, tb a96ca75, 234 s). cosim-check 8/8 eligible. TB-001 (test race on BUSY window under load) fixed f70dec9 without loosening. Bare `make -C tb/cosim` now starts the bridge. Now running final clean evidence regeneration (clean host build, sweep, dirty records → archive_dirty). Red-team S2+S3 held until it finishes (CPU). |

| 16:10 | fpga-integrator | done (prep) | HW_CHECKLIST: H-4 `selftest --dump` + `dump` triage, P-5a Windows host build, generated Bitstream manifest (manifest.py) so build_ids/md5s never go stale (df76e45, f402cc7). One-shot `bash fpga/tcl/s4_rebuild.sh` (6 tags, clean-tree + DIV_OVERRIDE checks, cost + manifest, path-limited commit) ready, held for lead GO. Self-corrected git sweep (955f881 reset; 49 staged archive_dirty files untouched). Lead sequence: verif-2 clean evidence → S4 rebuild → red-team S2+S3+S4 combined. |

| 11:20 | lead | — | Wi-Fi outage ~10:56–11:18 killed verification-engineer 2 mid evidence run (no sims left running; 50 staged archive_dirty renames intact). Resumed it. Schedule: ahead of plan (S4 target 16:00). |

| 11:24 | verification-engineer 2 | done | **Final clean S3 evidence** (e2cea28, 1cc3776): 66 sim records from a clean host build (git_hash f402cc7, build 0f402cc7, model 72da65248adf): bench sweep 56/56 (4 GEMMs × M 1..64 × both modes) + cosim-check 8/8 eligible; 183,808 outputs, 0 mismatches, 0 counter diffs, invariants OK. tb/system SIGNOFF=1 617/617 eligible at f402cc7. 49 dirty records git-mv’d to archive_dirty. session.json untracked (concurrent-writer race) → host fix ordered. Lead sent GO S4 REBUILD. |

| 11:24 | lead | — | Launched red-team combined S2+S3+S4 review (phase 1 read-only audits now; phase 2 sims + bitstream checks after the S4 rebuild commit). |

| 11:27 | host-dashboard | done | Session totals derived from run records (84f48f0): `session --rebuild --source sim` → 66 run, 66 verified, 0 mismatches; atomic writes; archive_dirty/mock never counted as verified. 59 tests. results/sim/session.json re-committed. |

| 11:29 | human + lead | done | Windows host build (USB strategy b) works: rustup 1.99.1 msvc, `win_build_host.ps1` OK in 52 s → sw	argeteleasesplitsys-host.exe; lead ran it: mock ping OK, mock selftest FULL 6c1d81af / STRIP 8b1c6e80 PASS. No COM port yet (board not connected — expected). |

| 11:32 | red-team-reviewer | phase 1 | S2 PARTIAL, S3 PARTIAL (re-runs pending rebuild), **S4 FAIL as-is**: SEV-2 dashboard error table omits q0..q3 yet says 'every counter' → host; SEV-2 'mutants in every unit suite' unsupported by SIGNOFF → storyteller; SEV-2 docs/plots cite archived dirty SIM records → storyteller after rebuild; SEV-3 int32 run headlined unlabeled in Scene 1 → host; SEV-3 MatX HBM+SRAM uncited, D-008 wording → storyteller; SEV-3 rebuild HEAD drift (84f48f0 landed mid-rebuild) → fpga warned; SEV-3 WSL VM self-restart ~11:26 wiped /tmp → toolchain. Model/SIM/cost numbers all trace; checklist complete; no CPU/GPU claims; no external URLs. |

| 11:39 | fpga-integrator | FAIL (provenance) | S4 rebuild: all 6 builds exit 0 with positive slack (top WNS +0.849) but wave 2 stamped 84f48f0 (commit landed mid-rebuild) → script refused to commit. Lead froze commits; fpga hardening s4_rebuild.sh (pin build_id to start HEAD, verify HEAD unchanged) and rerunning. |

## Pre-Phase-H human checklist
- [x] Install Windows Rust + build splitsys-host natively (done by human ~11:30; release build 52 s; lead verified mock ping + selftest PASS from the Windows exe).
- [ ] Confirm USB strategy (b).
- [ ] Laptop: sleep disabled on AC (done 09:10); keep lid open.
