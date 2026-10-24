# SplitSys — Claude Code Operating Manual

SplitSys is a splittable int8 systolic-array GEMM accelerator on the RealDigital Urbana board (AMD Spartan-7 XC7S50), built at Cal Hacks 13.0 (Oct 23–25, 2026, Palace of Fine Arts, San Francisco).

**Thesis (memorize it):** Big systolic arrays waste most of their rows on skinny LLM decode matmuls. An 8×8 array that can rewire itself as 4×16 roughly doubles decode utilization, at a measured cost in operand bandwidth and area, and our cycle model predicts the hardware counters.

**Audience that matters:** hardware engineers from MatX (splittable systolic arrays, LLM-specific silicon) and Annapurna Labs (AWS Trainium: systolic TensorEngine, large on-chip SRAM, explicit data movement). Cal Hacks judges score Best Hardware Hack, Most Technically Complex Hack, Overall, and possibly Best Solo Hack. The judging rules require a **live demo**.

## Read before doing anything

| File | What it is |
| --- | --- |
| `docs/SPEC.md` | The contract: numerics, architecture, interfaces, counters, verification. It is the source of truth. |
| `docs/PLAN.md` | Gates with clock deadlines, parallel lanes, cut rules, and event deadlines. |
| `docs/DEMO.md` | Monitor dashboard spec, booth setup, pitch variants, Q&A bank. |
| `docs/STATUS.md` | Running log kept by the lead: the last state of every gate. Create it if missing. |
| `docs/ENV.md` | Machine-specific facts verified on this laptop (`toolchain-engineer`). Create it if missing. |
| `docs/HW_CHECKLIST.md` | The ordered physical test script for Phase H (`fpga-integrator`). Built up during Phase S. |
| `spec/spec.toml` | Machine-readable constants. Generated files are never hand-edited. |

## Hard rules (non-negotiable)

1. **Hackathon rule.** All project code is written during the event. Never paste in pre-existing project code. Third-party libraries and tools are fine.
2. **Never fabricate a result.** Every number on the dashboard, README, Devpost, or pitch comes from a file in `results/` tagged `source: hw | sim | model`. The source label is shown wherever the number appears. Placeholder data is visibly marked `DEMO DATA`, and none of it may remain at judging time.
3. **Never weaken, skip, or delete a test to get green.** If you believe a test is wrong, stop, explain why, and get the lead's agreement first.
4. **Bit-exact is the bar for numerics.** "Close" counts as a failure.
5. **The spec wins.** If code and `docs/SPEC.md` disagree, stop and route the question to `spec-steward`. Change the spec, regenerate, then update both sides. Never diverge silently.
6. **Ownership.** Each agent edits only the files it owns (table below). Verification never edits RTL; RTL never edits tests. A change needed outside your area goes back to the lead as a request.
7. **Small green steps.** After every change, run the narrowest relevant test. Commit when green: `git commit -m "[agent] what — <test cmd> passed"`.
8. **Protect the demo.** `fpga/known_good/` always holds the last bitstream that passed hardware tests, with its build id, git hash, and reports. Never overwrite it with an untested build.
9. **Offline at demo time.** The dashboard and host run with no network: no CDN, no web fonts, no remote calls. Personal Wi-Fi hotspots are banned at the venue.
10. **The human does physical actions only,** and only in Phase H (below): cables, buttons, reading LEDs, power cycles, Windows-side approvals. During Phase S, never ask for the board. Add the check you would have asked for to `docs/HW_CHECKLIST.md` instead.
11. **No reuse of the human's earlier project code.** While exploring WSL you may find old projects (a RISC-V CPU, an FMA unit, UART modules, coursework). Do not copy, import, or adapt their code: the hackathon requires everything to be written during the event. You may read vendor files (the Urbana master XDC, Xilinx docs) and use installed tools.

## Working mode: simulation-first, then one hardware session

The human asked for this explicitly. Follow it.

- **Phase S (simulation and builds, no board needed).** Build everything, and verify everything with testbenches and simulation:
  - every RTL module, the full system, and the self-test
  - the host program talking to the simulated RTL over a TCP-to-UART bridge (co-simulation)
  - the dashboard running on simulated or model data
  - every bitstream, synthesized, placed, routed, and timing-clean

  Work autonomously. Do not stop to ask for confirmation between tasks. Stop only for a decision that is genuinely the human's, or a `sudo` password (see Environment).
- **Phase H (hardware, with the human).** It starts only when `red-team-reviewer` passes gate S4. The lead walks the human through `docs/HW_CHECKLIST.md` step by step: one physical action at a time, the exact command, and what they should see. A hardware failure is first reproduced in simulation, then fixed, then re-verified in sim, before the board is touched again.
- **`docs/HW_CHECKLIST.md`** is written during Phase S. `fpga-integrator` owns it; every agent contributes rows. Each row has: step number, bitstream to program, physical action, command to run, expected observation (LEDs, 7-segment, console output), and what to do on failure. Order it from least to most complex: heartbeat → UART echo → memory round trip → self-test → random GEMMs → benchmarks → dashboard.

## Environment (verify on first session; record results in `docs/ENV.md`)

- **Laptop:** ASUS ROG Zephyrus G14 (2025). Vivado 2025.2.1 is installed on Windows at `C:\Xilinx\2025.2.1`. From WSL it is called through the wrapper at `/usr/local/bin/vivado`. Target part: `xc7s50csga324-2` (confirm against the board).
- **Simulation:** Verilator 5.x and cocotb in WSL. **Rust:** stable via rustup. Do not upgrade toolchains mid-event.
- **USB hazard.** The Urbana's single micro-USB carries both JTAG and UART. WSL2 sees USB devices only after `usbipd attach`, and attaching to WSL takes the device away from Windows Vivado. Choose one strategy in `docs/ENV.md` and keep it:
  - **(b) Recommended:** build and run `splitsys-host` natively on Windows, pointed at the board's COM port. Vivado on Windows programs over JTAG. Nothing moves between WSL and Windows during the demo.
  - (a) Program from Windows Vivado, then `usbipd attach` the device to WSL for UART. Detach before each reprogram.
- **Board facts come from the official Urbana master XDC and schematic, never from memory:** oscillator frequency and pin, UART pins, LEDs, switches, buttons, 7-segment, HDMI. First search WSL and the Windows drive for an existing copy (see below). If none is found, ask the human once to download it from RealDigital.

### Exploring WSL and installing software (owner: `toolchain-engineer`)
- **Explore first, install second.** Inventory what already exists:
  - in WSL: `$HOME`, `/usr/local/bin`, `/opt`, and tool versions on `PATH`
  - on the Windows side, read-only: `/mnt/c/Xilinx`, plus `Downloads`, `Documents`, and `Desktop` under `/mnt/c/Users/*`
  - files to look for: an Urbana master XDC, Vivado board files, and the existing `vivado` wrapper

  Use bounded searches (`find` with `-maxdepth` and name patterns). Never crawl the whole Windows drive. Record every finding in `docs/ENV.md`.
- **Install what's missing, preferring user-level installs** that need no password: `rustup` (stable), a Python venv at `.venv/` with `cocotb` and `pytest`. Install Verilator into `~/.local` if apt's version is too old: check cocotb's documented minimum Verilator version for the installed cocotb, and build from the latest stable tag with `make -j$(nproc)` if needed.
- **System packages needing `sudo`** (build-essential, git, autoconf, flex, bison, help2man, libfl-dev, zlib1g-dev, python3-venv, socat, unzip, gtkwave if wanted): collect all of them into **one** `sudo apt-get install -y ...` command, and ask the human to run it once. Never retry sudo in a loop or try to bypass it.
- Never modify Windows-side files. Never change the Vivado install. Never upgrade tools that already work.
- Write `scripts/setup_env.sh` (idempotent) so the environment can be recreated, and `scripts/check_env.sh`, which prints every tool version and exits non-zero if anything required is missing.

## Repository layout

```
spec/spec.toml              constants: dims, latencies, opcodes, field widths, counter map
sw/                         Cargo workspace
  crates/spec/              parses spec.toml; emits rtl/gen/splitsys_pkg.sv and Rust consts
  crates/golden/            bit-exact reference GEMM, requant, packing/unpacking
  crates/cycle/             cycle model predicting every hardware counter
  crates/vec/               test vectors (.hex/.mem + .json), self-test ROM image
  crates/host/              UART driver, CLI, benchmark runner, dashboard server (bin: splitsys-host)
rtl/                        synthesizable SystemVerilog
  gen/                      GENERATED — do not edit
tb/                         cocotb + Verilator testbenches, one directory per module + system
tb/cosim/                   TCP<->UART bridge so the real splitsys-host talks to simulated RTL
fpga/xdc/ fpga/tcl/ fpga/reports/ fpga/known_good/ fpga/bringup/   (staged bring-up tops)
scripts/                    setup_env.sh, check_env.sh, start_demo scripts
demo/                       dashboard static assets (embedded into the host binary)
results/                    run records (JSON) tagged hw|sim|model, plots
docs/                       SPEC, PLAN, DEMO, ENV, STATUS, DECISIONS, devpost
```

## Agent roster and ownership

| Agent | Owns (may edit) | Delegate when |
| --- | --- | --- |
| `spec-steward` | `spec/`, `sw/crates/spec/`, `rtl/gen/` (generated), `docs/SPEC.md`, `docs/DECISIONS.md` | Any interface, constant, layout, or contract change; codegen; spec questions |
| `golden-modeler` | `sw/crates/golden/`, `sw/crates/vec/` | Reference numerics, packing, test vectors, self-test ROM image, stretch-LM quantization |
| `perf-modeler` | `sw/crates/cycle/`, `results/model/` | Counter predictions, utilization ceilings, roofline, model-vs-hardware reconciliation |
| `rtl-designer` | `rtl/` except `rtl/gen/` | All SystemVerilog design work and RTL bug fixes |
| `verification-engineer` | `tb/` | Testbenches, sign-off runs, coverage, minimal bug reproductions against RTL |
| `toolchain-engineer` | `scripts/`, `docs/ENV.md`, `.venv/` | Exploring WSL, installing and verifying tools, the Vivado wrapper, USB strategy, environment breakage |
| `fpga-integrator` | `fpga/`, `docs/HW_CHECKLIST.md` | Vivado builds, XDC, timing closure, reports, staged bring-up bitstreams, the Phase H checklist, programming, `known_good`, QSPI |
| `host-dashboard` | `sw/crates/host/`, `demo/` | UART driver, CLI, benchmark runner, the monitor dashboard |
| `demo-storyteller` | `README.md`, `docs/DEMO.md`, `docs/devpost.md`, `results/plots/` | Pitch, Q&A, Devpost text, README, plots, booth checklist |
| `red-team-reviewer` | nothing (read-only) | Every gate, before any public claim, after any scary bug fix |

**The lead (main session) orchestrates.** It picks the next task from `docs/PLAN.md`, delegates with a complete prompt, checks the handoff, runs the reviewer at gates, and keeps `docs/STATUS.md` current. The lead may do trivial cross-cutting edits itself (fixing a path, a typo). Anything substantive goes to the owner.

**Parallelism.** Independent lanes run in parallel: RTL plus verification on the critical path; golden, perf, host, and dashboard against sim or model data. Never let two agents edit the same files at once.

## Delegation protocol (lead → agent)

Subagents do not see this conversation. Every delegation prompt contains:

1. **Goal** in one sentence, and the gate it serves.
2. **Scope:** exact files to create or edit, and files not to touch.
3. **Inputs:** SPEC sections, vectors, previous results, known bugs.
4. **Acceptance criteria** as runnable commands with expected outcomes.
5. **Time box** (for example, 45 minutes). If exceeded, stop and report partial progress.

## Handoff contract (agent → lead)

Every agent ends its work with exactly this block:

```
STATUS: done | partial | blocked
CHANGED: <files>
VERIFIED: <command> -> <observed result>      (one line per check actually run)
NOT VERIFIED: <what was not run, and why>
RISKS/OPEN: <bugs, spec questions, assumptions made>
NEXT: <recommended next step and which agent>
```

The lead copies a one-line summary into `docs/STATUS.md` with a timestamp.

## Gates

A gate in `docs/PLAN.md` passes only when `red-team-reviewer` has re-run the evidence commands itself and agrees. Record the verdict in `docs/STATUS.md`.

## Canonical commands (keep this list accurate as they become real)

```
source scripts/env.sh && scripts/check_env.sh            # toolchain OK?
cd sw && cargo test --workspace && cargo clippy --workspace -- -D warnings
make -C tb units | make -C tb system | make -C tb cosim     # cosim prints the TCP port
splitsys-host --transport tcp:127.0.0.1:<port> ping|selftest|run|bench   # host vs simulated RTL
cd sw && cargo run -p splitsys-spec --bin gen          # regenerate rtl/gen + consts
verilator --lint-only -Wall -Irtl -Irtl/gen rtl/gen/splitsys_pkg.sv rtl/*.sv
make -C tb <module>        # e.g. make -C tb pe ; make -C tb all
vivado -mode batch -source fpga/tcl/build.tcl -tclargs <tag>
vivado -mode batch -source fpga/tcl/program.tcl -tclargs <bitfile>
splitsys-host ping | selftest | run <gemm.json> | bench <suite> | serve
```

## Coding standards

- **SystemVerilog:** `logic`, `always_ff`/`always_comb`, signedness declared at ports and never cast mid-expression, parameters from the generated package, every register reset (synchronous, active-high `rst`), no latches, one clock domain (the optional HDMI pixel clock is the only exception, crossed through dual-port BRAM). Lint clean with `-Wall` or with documented waivers.
- **Rust:** no `unsafe`. Clippy clean. Randomized tests use explicit seeds that are printed on failure.
- **Python (cocotb):** glue only. Expected values come from Rust-generated JSON and are never re-derived in Python.
- **Commits:** one logical change, with the passing test command in the message.
