# SplitSys Build Plan: simulation-first, then one hardware session (all times Pacific)

**Fixed event deadlines (Cal Hacks 13.0 Hacker Guide):**
- **Devpost with a valid table number by Sun 09:00.** Edits are allowed until 10:00; the table number cannot change after 09:00. Submit a draft Saturday night.
- **Judging is Sun 10:30–12:30.** The whole team must stay at the table for the full window. Prepare a 2-minute pitch. **A live demo is required.**
- No soldering in the venue. No personal hotspots.

**Prizes in scope:** Best Hardware Hack, Most Technically Complex Hack, Overall, Best Solo Hack (if solo). Check the Live Site for an Annapurna Labs track.

---

## Strategy

The human asked for **Phase S** first: build and verify everything with testbenches and simulation, plus fully built, timing-clean bitstreams. Then comes **Phase H**: one guided hardware session that runs `docs/HW_CHECKLIST.md` in order. No agent asks for the board during Phase S.

**Why this works:** every hardware step becomes "program a prebuilt, timing-clean bitstream and check one expected observation." The real host program has already talked to the simulated RTL over the exact same protocol, so hardware surprises shrink to pins, the clock, and the USB/UART link.

**Optional early smoke (lead offers once; the human decides):** at any convenient moment, a 5-minute check that programs the heartbeat plus UART-echo bitstream (S1). It catches XDC, USB, or driver problems ~8 hours earlier. If the human declines, continue; it is checklist steps H-1 and H-2 anyway.

## How the lead runs this plan

1. Read `docs/STATUS.md`. Inspect the repo. Items that already exist become **verify**, not rebuild.
2. Run lanes in parallel (table at the bottom). Keep the critical path (RTL → verification → system sim → builds) always busy.
3. Work autonomously through Phase S. Do not stop for confirmation between tasks. Stop only for the single sudo request, a missing XDC that cannot be found on disk, or a genuine product decision.
4. At each gate, run `red-team-reviewer`. A gate passes only when its evidence re-runs green.
5. If a gate is late, apply its cut rule. Never trade away verification of anything that will be shown.

---

## Phase S — simulation and builds (no board)

### S0 — Environment and foundations · target Sat 03:30
- [ ] `toolchain-engineer`: WSL inventory, installs, `scripts/check_env.sh` all OK, Vivado batch smoke test, part `xc7s50csga324-*` confirmed, XDC found (or one request to the human). `docs/ENV.md` written.
- [ ] Repo skeleton; `git init`; `.gitignore` (Vivado outputs, `target/`, `.venv/`, `sim_build/`).
- [ ] `spec-steward`: `spec/spec.toml` plus codegen → `rtl/gen/splitsys_pkg.sv` and Rust consts.
- [ ] `golden-modeler`: GEMM, requant, packing, Fletcher-32, descriptor, with property tests.
- **Evidence:** `scripts/check_env.sh`; `cargo test --workspace`; Verilator lint of the generated package.
- **Cut rule:** if the Verilator source build is slow, start it in the background and continue with the Rust work.

### S1 — Units signed off · target Sat 07:30
- [ ] RTL plus testbenches for PE, skew, buffers, requant, uart_rx/tx, packet, counters. All SPEC §6 directed cases pass, plus the random suites.
- [ ] `fpga-integrator`: build scripts working. First bring-up bitstream `fpga/bringup/heartbeat_uart_echo` built and timing-clean (heartbeat LED, 7-segment test pattern, UART echo).
- [ ] `perf-modeler`: cycle model per SPEC §4 with the current latencies.
- [ ] `host-dashboard`: framing/CRC library and mock board; dashboard skeleton served on mock data (marked `MODEL`).
- **Evidence:** `make -C tb units`; `cargo test --workspace`; `fpga/reports/heartbeat_uart_echo/summary.json` with WNS ≥ 0.

### S2 — FULL mode, system simulation · target Sat 11:00
- [ ] Array (FULL) + sequencer + requant + buffers integrated into `top`. `tb/system` (driving the real UART pins) passes 200 random GEMMs up to 64×256×256, bit-exact. All counters equal the cycle model. Invariants hold.
- [ ] Self-test ROM and `selftest.sv` pass in simulation (FULL part).
- [ ] Full `top` bitstream (FULL-only build allowed at this stage) is timing-clean. Utilization report checked (64 DSPs or more, BRAM as expected).
- **Cut rule:** if FULL is not bit-exact in system sim by 12:30, drop INDEP and all stretch work. Everyone focuses on FULL.

### S3 — STRIP + co-simulation · target Sat 14:30
- [ ] STRIP mode bit-exact in system sim, including quadrant isolation, the lower-half `4 + c` alignment, and mode switching without reset. Self-test passes both modes in sim.
- [ ] **Co-simulation:** `tb/cosim` exposes the simulated `top`'s UART as a TCP socket. The real `splitsys-host` binary (`--transport tcp:127.0.0.1:<port>`) runs `ping`, `selftest`, `run`, and a reduced `bench` against simulated RTL. Results are saved under `results/sim/` with `source: "sim"`.
- [ ] Counters match the model exactly in sim, for every co-sim run.
- [ ] Dashboard Scenes 1–3 complete on sim/model data. Screenshots at 1920×1080 reviewed (`host-dashboard`).

### S4 — Everything built; ready for hardware · target Sat 16:00
- [ ] Staged bring-up bitstreams, all timing-clean, in `fpga/bringup/`: (1) heartbeat + UART echo; (2) packet layer + memory regions; (3) full `top` (ENABLE_STRIP=1); (4) cost build ENABLE_STRIP=0. `results/cost.json` written.
- [ ] `docs/HW_CHECKLIST.md` complete (format in CLAUDE.md). Each step names its bitstream, physical action, command, expected observation, and failure branch.
- [ ] `scripts/start_demo.*` and the Windows-side host build instructions ready (USB strategy b).
- [ ] `demo-storyteller`: README and Devpost skeletons with sim results clearly marked `SIM`; pitch drafts.
- [ ] **red-team-reviewer passes S4.** Only then does Phase H begin.

---

## Phase H — one guided hardware session (human present) · target start Sat 16:00

The lead walks through `docs/HW_CHECKLIST.md`. Each step is one physical action, one command, and one expected observation. On failure: stop; capture the evidence; reproduce in sim (`verification-engineer`); fix (owner); re-verify in sim; rebuild; resume at the failed step.

| Step | Bitstream | Check |
| --- | --- | --- |
| H-1 | heartbeat_uart_echo | LED15 blinks at 1 Hz; 7-seg test pattern |
| H-2 | heartbeat_uart_echo | `splitsys-host echo` 10,000 bytes, 0 errors (then raise baud if planned) |
| H-3 | packet_mem | `ping` returns build_id; WRITE/READ_MEM round trip of 10,000 frames on every region |
| H-4 | top | BTN0 self-test → LED0 (FULL) and LED1 (STRIP) on; `selftest` command agrees |
| H-5 | top | 100 random GEMMs per mode, bit-exact; counters == model (or a single calibration of provisional constants, then freeze) |
| H-6 | top | Full benchmark suite and M-sweep → `results/hw/`. Save **KG1** to `fpga/known_good/` |
| H-7 | top | Dashboard on the real monitor with HW data; cold-start drill under 60 s |

**Cut rules:**
- STRIP fails on hardware and isn't fixed by 20:00: demo FULL on hardware plus STRIP from co-simulation, clearly labeled `SIM`.
- The UART is unreliable at high baud: drop to 115200.

### G4 — Results + dashboard (hardware data) · target Sat 22:00
- [ ] Scenes 1–3 on the real monitor driven by `hw` data; every number audited by `red-team-reviewer`.
- [ ] README updated with HW results; **Devpost draft submitted.**

### G5 — Freeze + polish · target Sun 04:00
- [ ] RTL feature freeze. After this, only bug fixes with full sim regression, then rebuild, then the H-4/H-5 re-check.
- [ ] Stretch, only if G4 passed by 22:00, in this order: auto mode selection, QSPI flash of KG, tiny LM, INDEP.
- [ ] Backup video recorded; final README and `docs/devpost.md`; Q&A rehearsal.

### G6 — Ship · Sun 08:30 hard stop
- [ ] Final bitstream = latest KG; self-test passes after a power cycle.
- [ ] Three full pitch rehearsals on the real setup, including one cold start.
- [ ] **Devpost final with table number before 09:00.** Booth ready by 10:15. Team at the table 10:30–12:30.

---

## Parallel lanes

| Lane | Agents | Phase S work | Phase H work |
| --- | --- | --- | --- |
| Environment | toolchain-engineer | inventory, installs, check_env | Windows-side host build, COM port |
| Critical path | rtl-designer → verification-engineer → fpga-integrator | RTL, unit + system + co-sim, all bitstreams, checklist | failure triage through sim |
| Numerics | golden-modeler | golden, vectors, self-test ROM | — |
| Performance | perf-modeler | cycle model, sweep, roofline | calibration (once) and freeze |
| Host + dashboard | host-dashboard | protocol, mock board, TCP transport, dashboard | real serial, monitor polish |
| Story | demo-storyteller | README/Devpost skeletons, pitch drafts | final numbers, rehearsal |
| Audit | red-team-reviewer | every S gate | G4–G6 claims audit |

**Rest windows (suggested):** during Phase S the agents run autonomously, so the human can rest roughly Sat 03:30–10:00 once the single sudo command has been run and the XDC question is answered. Resume around S2 to check progress. Phase H needs the human from ~16:00. Second rest around Sun 04:30–07:00, after the freeze.
