# System and co-sim sign-off record (owner: verification-engineer, instance 2)

## What is tested and how

**tb/system** (`make -C tb/system`, SPEC §6 level 3). Each worker is one `tb/cosim` harness process:
Verilator `top` (all of `rtl/*.sv`, committed tree exported via `tb/Makefile rtl_export`), driven
**only through its pins**: TCP bytes are serialized onto `UART_RXD` (8N1, back-to-back), `UART_TXD` is
decoded mid-bit with a stop-bit check, BTN[1:0] are driven for reset/self-test, and LED[15:0] are read.
`system_test.py` speaks SPEC §3.8 frames over that socket.

Sim-only top overrides: `UART_DIV_OVERRIDE=1` (16 clocks/bit, 160/byte), `DEB_CYC=16`, `HB_HALF_CYC=500`,
`ACT_CYC=64`, `SCAN_CYC=64`; `RX_TIMEOUT_CYC` stays at the package value. `BUILD_ID = 0x0<HEAD short hash>`.
Hardware builds use `UART_DIV_OVERRIDE=0`.

Expected values come only from Rust:
- every response frame must be byte-identical to `splitsys_golden::packet::response_frame(status, payload)`
  (tb/oracle `rspframe`); requests are built by `request_frame` (`reqframe`);
- outputs: `splitsys-vec` job JSON `expected.obuf_hex`, compared on `compare_ranges` only (D-006); the whole
  C extent is first poisoned with the complement of the expected image, so an unwritten byte cannot pass;
- all 12 counters: `splitsys-cycle predict <job>` (exact equality), plus the SPEC §3.7 invariants
  (`mac_useful == M*N*K`, `sum(q) == mac_useful`, `cycles_total == Σ states + L_ACCEPT + L_DONE`);
- self-test: `sw/crates/vec/out/selftest_expected.json` (FULL 6c1d81af, STRIP 8b1c6e80).

| Suite | Content |
| --- | --- |
| full | `splitsys-vec gemm --mode full --seed SEED --n 200` up to 64×256×256 (first 8 are edge shapes incl. 64×256×256, 64×1×256, 1×256×1; ~15% int32 bypass; random bases), spread over 16–18 boards, many jobs back to back per board without reset |
| strip | same generator, `--mode strip --n 100` |
| directed | all 262 jobs of `sw/crates/vec/out/directed.json`, both modes: extreme products at K=256 (bypass + requant), zeros, identity W, all-ones K=37, **single nonzero at every PE position** (FULL 8×16, STRIP 4×16 → covers the lower-half `4+c` alignment), **quadrant isolation Q0..Q3** (counters check `mac_useful_q` per quadrant), requant ties/saturation, edge shapes |
| modeswitch | 4 chains F,S,F,S,…,F (13 jobs each) on one board, no reset |
| selftest | (a) power-on POLL=IDLE, LED0/1 off; SELFTEST opcode ×2 → payload `01 01 fl_full fl_strip` exact, counters == STRIP self-test prediction, LED[2:0]=011; user STRIP+FULL jobs then SELFTEST again; BTN1 reset → LEDs clear, PING ok, POLL=IDLE. (b) BTN0 press ×2 → LED0+LED1, counters == STRIP prediction; then SELFTEST opcode agrees. (c) 64×256×256 job running: RUN_GEMM/SELFTEST/WRITE_MEM/READ_MEM → BUSY, PING ok, POLL=BUSY, live cycles_total in (0, final); then DONE, final counters exact, SELFTEST passes |

Sensitivity (mutants, stand-in build, 2026-10-24): STRIP W-mid skew base 4→3 → 5/8 STRIP jobs fail with
`PE(4,c) Q2` hints, self-test STRIP fails (LED1 off, fletcher b10ae555), FULL unaffected; `tap_flush` also
counting DRAIN → every job fails `cycles_flush` diff + the cycles_total invariant.

**tb/cosim** (`make -C tb/cosim`, prints `COSIM_PORT`; `make -C tb/cosim check` = S3 acceptance):
same harness; the real `splitsys-host --transport tcp:127.0.0.1:<port>` runs ping, selftest,
`run --name ffn_up --m 1 --mode strip|full`, `run --name qkv --m 8 --mode both`,
`run --name attn_out --m 5 --mode both --int32`, `bench smoke`, ping. Records → `results/sim/` (`source: "sim"`).

## Record

| Date | RTL (HEAD / last rtl commit) | tb | Command | Result | Notes |
| --- | --- | --- | --- | --- | --- |
| 10-24 | worktree (3786ad1 + uncommitted top.sv) | 9e53f43 | `make -C tb/system RTL_REV=worktree WORKERS=18` SEED=1 | **617/617 PASS**, 415 s | pre-commit run |
| 10-24 | worktree (same) | 9e53f43 | same, SEED=2 RAND_RESET=1 | **617/617 PASS**, 295 s | random X-initial state |
| 10-24 | af13dcc / 7140392 | 9e53f43 + uncommitted Makefile edits | `make -C tb/system WORKERS=18` SEED=1 | **617/617 PASS**, 258 s | NOT sign-off eligible: tb/ inputs dirty (instance-1 edits to tb/Makefile, tb/common, tb/oracle in progress) |
| 10-24 | af13dcc / 7140392 | same | `make -C tb/cosim check TAG_SUFFIX=-chk` | **8/8 host steps PASS** | 10 GEMMs verified, 0 mismatches, all counters model err 0.000%; records committed in 6bff698 (git_hash `af13dcc-dirty`), superseded below |
| 10-24 | 6bff698 / adca4a3 (per-block registered resets) | 6bff698 | `make -C tb/system WORKERS=18 SEED=3 RAND_RESET=1` | **617/617 PASS**, 258 s | random X-initial state; still NOT sign-off eligible only because instance-1 tb/Makefile, tb/common, tb/oracle edits are uncommitted. Re-run with `SIGNOFF=1` once those land: it refuses on any dirty input |
| 10-24 | d1867c8 / adca4a3 | 6bff698 | `make -C tb/cosim check` (splitsys-host at d1867c8, includes 1a19134) | **8/8 host steps PASS** | 10 GEMMs verified, 0 mismatches, model err 0.000% on every run; records in results/sim regenerated: source "sim", build_id 0d1867c8, git_hash d1867c8, model_git_hash 72da65248adf; old af13dcc records removed |
