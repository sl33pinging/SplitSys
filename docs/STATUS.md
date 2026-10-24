# SplitSys Status (kept by the lead)

**Phase:** S (simulation-first, no board) · **Current gate:** S0 (target Sat 03:30) · **Last update:** Sat 01:55

States: `todo` · `running` · `done` (owner handoff, lead checked) · `PASS` (red-team re-ran it) · `FAIL` · `blocked` · `cut`

## Gate board

| Gate | Item | Owner | State | Evidence command |
| --- | --- | --- | --- | --- |
| S0 | Repo skeleton, git init, .gitignore/.gitattributes, Cargo workspace stubs | lead | done | `git log --oneline -1`; `ls sw/crates` |
| S0 | WSL inventory, installs, Vivado batch smoke, part confirmed, XDC located, docs/ENV.md | toolchain-engineer | running | `source scripts/env.sh && scripts/check_env.sh` |
| S0 | spec/spec.toml + codegen → rtl/gen/splitsys_pkg.sv + Rust consts | spec-steward | running | `cd sw && cargo test -p splitsys-spec`; `verilator --lint-only -Wall rtl/gen/splitsys_pkg.sv` |
| S0 | Golden: GEMM, requant, packing, Fletcher-32, descriptor + property tests; vectors | golden-modeler | running | `cd sw && cargo test -p splitsys-golden -p splitsys-vec` |
| S1 | Cycle model per SPEC §4 (started early, parallel lane) | perf-modeler | running | `cd sw && cargo test -p splitsys-cycle` |
| S0 | Gate review | red-team-reviewer | todo | — |
| S1 | Units: PE, skew, buffers, requant, uart_rx/tx, packet, counters (RTL + tb) | rtl-designer / verification-engineer | todo | `make -C tb units` |
| S1 | Build scripts + bringup/heartbeat_uart_echo timing-clean | fpga-integrator | todo | `fpga/reports/heartbeat_uart_echo/summary.json` WNS ≥ 0 |
| S1 | Host framing/CRC lib, mock board, dashboard skeleton (MODEL) | host-dashboard | todo | `cd sw && cargo test -p splitsys-host` |
| S2 | FULL system sim, 200 random GEMMs bit-exact, counters == model; FULL top bitstream | critical path | todo | `make -C tb system` |
| S3 | STRIP bit-exact, co-sim with real host, dashboard scenes 1–3 | critical path / host-dashboard | todo | `make -C tb cosim` + `splitsys-host --transport tcp:...` |
| S4 | All bring-up bitstreams, cost build, HW_CHECKLIST, start_demo, README/Devpost skeleton | fpga-integrator / demo-storyteller | todo | red-team S4 |

## Decisions and notes (lead)

- Sat 01:55 — Kit was unpacked into `splitsys-claude-code-kit (1)/`. Moved its contents to the repo root (`C:\Users\danie\FPGA_Projects\Calhack2026`, WSL `/mnt/c/Users/danie/FPGA_Projects/Calhack2026`) because the space/parens path would break Make and Vivado Tcl. The original zip is kept.
- Sat 01:55 — Repo lives on the Windows drive so Windows Vivado and Claude's file tools can reach it. Heavy build outputs (cargo target, Verilator obj dirs) should go to WSL-native paths for speed (toolchain-engineer decides via `scripts/env.sh`).
- Sat 01:55 — Lead probe: WSL Ubuntu 24.04, 24 cores, 15 GB RAM. gcc/make/curl/python3-venv present; socat, unzip, verilator, cargo missing. `vivado` on PATH is `/mnt/c/Xilinx/2025.1/...`; both 2025.1 and 2025.2.1 are installed. No `/usr/local/bin/vivado` wrapper exists (CLAUDE.md assumed one).
- Sat 01:55 — Lead installed rustup stable (user-level) in WSL to unblock the three Rust lanes; toolchain-engineer verifies and records it.

## Handoff log (one line each)

| Time | Agent | Status | Summary |
| --- | --- | --- | --- |
