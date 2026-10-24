# tb/ — SplitSys verification (cocotb + Verilator)

Owners: verification-engineer instance 1 owns `tb/` except `tb/system/` and `tb/cosim/`,
which belong to instance 2. Everyone reuses `tb/common/`.

## Commands (inside WSL, `scripts/env.sh` sourced, e.g. `scripts/wsl '...'`)

| Command | What it does |
| --- | --- |
| `make -C tb units` | Runs every unit suite plus the level-2 `core` suite. |
| `make -C tb signoff` | Same as `units`, but refuses to start if the tree isn't clean. See "Run conventions" below. |
| `make -C tb quick` | Smoke run, SCALE=0.1. |
| `make -C tb all` | SCALE=4 over SEEDS 1, 2 and 3. |
| `make -C tb <suite>` | Runs one suite: pe, skew, uart, counters, buffers, requant, packet, packet_ns, array or core. |
| `make -C tb system` | Delegates to `tb/system`. |
| `make -C tb cosim` | Delegates to `tb/cosim`. |
| `make -C tb provenance` | Prints the hashes and dirty flags for this checkout. |

Knobs:
- **Run control:** `SEED`, `SCALE`, `WAVES=1`, `COCOTB_TEST_FILTER=<test>`.
- **RTL under test:** `RTL_REV` = `HEAD` | `<rev>` | `worktree`.
- **Namespace:** `SPLITSYS_CACHE_TAG`.

## Run conventions (all tb/ Makefiles; implemented in `tb/common/provenance.mk`)

1. **RTL is a commit.**
   - Suites test `git archive <RTL_REV> rtl`, not the working tree.
   - `RTL_REV=worktree` is a designer's pre-commit check, never a sign-off.
2. **Provenance on every run.**
   - Each run prints `head`, `rtl`, and the tree hashes of `tb/`, `sw/crates/golden`, `vec`, `cycle`, `spec` and `spec/`. A path whose working tree differs from HEAD gets a `+dirty` suffix.
   - It also prints `sign-off eligible: YES/NO`.
   - Copies go into `$(RUN_LOG_DIR)/provenance.txt` and the first line of every suite log.
3. **Sign-off refuses dirty inputs.**
   - `make -C tb signoff` exits 1 if `tb/`, a Rust reference crate, or `spec/` is dirty, or if `RTL_REV=worktree`.
   - SIGNOFF.md entries quote the provenance line of a `signoff` run.
4. **Per-agent namespaces.** Set `SPLITSYS_CACHE_TAG` (e.g. `ve1`, `ve2`, `redteam`) so concurrent agents never share these:
   - Verilator build dirs: `$(TB_NS)/sim_build`
   - Tool builds:
     - `$(TB_NS)/oracle-target`: the tb oracle, built from this checkout's golden
     - `$(TB_NS)/sw-target`: splitsys-cycle and splitsys-vec
   - Logs: `$(TB_NS)/logs/<RUN_ID>/<suite>.log`. Every run gets a new directory, so nothing is overwritten.

   `TB_NS = ~/.cache/splitsys/tb/<tag>`. The tag defaults to `default`.

   Content-addressed caches stay shared, and are written atomically (tmp dir, then `mv`):
   - `~/.cache/splitsys/rtl_rev/<commit>`
   - `~/.cache/splitsys/vec/sw-<tree>/seed<S>`, or a `-dirty-<tag>` variant when the Rust crates are dirty
5. **No false passes.**
   - Every suite deletes its `results.xml` and forces a fresh simulator run.
   - The Makefile refuses to report a pass if no fresh `results.xml` with test cases appears.
6. **Expected values only from Rust.** The sources are:
   - `tb/oracle`, a thin CLI over `splitsys_golden`: `gemm`, `requant`, `crc8`, `fletcher32`, `reqframe`/`rspframe`, `descvalid`, `job`
   - `sw/crates/vec` JSON
   - `splitsys-cycle predict`

   Python is glue only.

To adopt this in another tb Makefile, set `TB_ROOT` and `RTL_SHA`, then
`include $(TB_ROOT)/common/provenance.mk`. Then use `$(TB_NS)`, `$(RUN_LOG_DIR)`,
`$(TB_SIM_BUILD)`, `$(PROV_LINE)` and `$(PROV_ELIGIBLE)`, and add the `provenance` target
as a prerequisite.

## Layout

- `common/`: shared files.
  - `sim.mk`: the cocotb/Verilator include.
  - `splitsys_tb.py`: helpers: package constants, seeds, oracle, cycle model, clock and reset.
  - `provenance.mk`: the run conventions above.
- `oracle/`: Rust CLI over `splitsys_golden`. It is not part of the `sw/` workspace.
- `<suite>/`: one directory per suite, holding a `Makefile`, `test_<suite>.py`, and an optional `<suite>_tb.sv` wrapper.
- `SIGNOFF.md`: the sign-off record.
- `BUGS.md`: open and closed findings.
