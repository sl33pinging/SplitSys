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
| display | 7-segment image rebuilt from the **real `top` D0/D1 AN/SEG pins** (harness `seg`, stepped 1 clock at a time over > 2 scan frames), with the **H-1 hardware-confirmed rule** (Phase H step H-1, 2026-10-24, build 52a66216 showed `6.126 6A25` for 0x52A66216): D0 = left display, AN[0] = leftmost digit; segments/anodes active low per the XDC. Expected images are golden board/spec data (SPEC §3.9 `F  9.3` / `S 21.4` format): power-on `----\|----`, `S   \| 21.4` (214, STRIP), `F   \|  9.3`, `F   \|  0.0`, `S   \| 99.9`, `F   \|100.0`, SET_DISPLAY mode 2 -> NACK_RANGE and the image is unchanged, `S   \|  0.5`. Also checks one anode at a time and a stable pattern per cell. Sensitivity: RTL 80954ef (old defaults D1-left/AN0-right) fails 7/8 (power-on dashes are orientation-symmetric) |

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
| 10-24 | 34cbc27 / adca4a3 | tree ae76c83 | `make -C tb cosim-check` (SPLITSYS_CACHE_TAG=ve2) | **8/8 host steps PASS**, sign-off eligible YES | provenance: `head=34cbc27 rtl=34cbc27 tb=ae76c83 golden=382bda8 vec=8b48b12 cycle=c8b8125 spec_rs=d4fda53 spec=8a4691f run=20261024-103714-11338 ns=ve2`. 10 GEMMs, 0 mismatches, model err 0.000%. Records committed in f70dec9 (build_id 034cbc27; host git_hash `34cbc27-dirty` because sw/crates/host/src/server.rs had uncommitted host-dashboard edits, which are not an expected-value input) |
| 10-24 | 34cbc27 / adca4a3 | tree ae76c83 | `make -C tb/system SIGNOFF=1 SEED=1 WORKERS=18` | 616/617: `selftest_busy` FAIL | TB-001 (test wall-clock race, see BUGS_SYSTEM.md), fixed in f70dec9 |

### SIGN-OFF (S2/S3 system level), 2026-10-24
`scripts/wsl 'make -C tb/system SIGNOFF=1 SEED=1 WORKERS=18'` -> **617/617 PASS** (selftest 3, strip 100, full 200,
directed 262, modeswitch 52), 234 s, `sign-off eligible: YES`.
Provenance: `head=30475dc rtl=30475dc tb=a96ca75 golden=382bda8 vec=8b48b12 cycle=c8b8125 spec_rs=d4fda53 spec=8a4691f run=20261024-104800-4142 ns=ve2`
(last commit touching rtl/: adca4a3). Cycle model latencies L_PE 3, L_RD 2, L_SETUP 16, L_RQ 7, L_WB 1, L_ACCEPT 1,
L_DONE 1 (provisional=false), model_git_hash 72da65248adf. Every one of the 12 counters was equal to the model on every job.

### SIGN-OFF re-run for the final clean evidence set, 2026-10-24
`scripts/wsl 'make -C tb/system SIGNOFF=1 SEED=1 WORKERS=18'` -> **617/617 PASS** (selftest 3, strip 100, full 200,
directed 262, modeswitch 52), 226 s, `sign-off eligible: YES`.
Provenance: `head=f402cc7 rtl=f402cc7 tb=880b60c golden=382bda8 vec=8b48b12 cycle=c8b8125 spec_rs=d4fda53 spec=8a4691f run=20261024-105551-850 ns=ve2`
(last commit touching rtl/: adca4a3; between 30475dc and f402cc7 only docs/fpga/tb-record files changed).

### Final clean S3 co-sim evidence (results/sim), 2026-10-24
Tree clean for rtl/ tb/ sw/ demo/ spec/ at HEAD f402cc7; splitsys-host rebuilt from it (embedded git_hash `f402cc7`, no `-dirty`).
- `scripts/wsl 'make -C tb cosim-check'` (SPLITSYS_CACHE_TAG=ve2) -> 8/8 host steps PASS, `sign-off eligible: YES`,
  provenance `head=f402cc7 rtl=f402cc7 tb=880b60c golden=382bda8 vec=8b48b12 cycle=c8b8125 spec_rs=d4fda53 spec=8a4691f run=20261024-112016-811 ns=ve2`.
- In parallel, on a second harness instance built from the same tree: `splitsys-host --transport tcp:127.0.0.1:<port> bench sweep --seed 1`
  (qkv, attn_out, ffn_up, ffn_down x M in {1,2,4,8,16,32,64} x FULL/STRIP = 56 runs) -> 56/56 verified.
- results/sim now holds **66 run records** (56 sweep + 10 cosim-check) plus bench_sweep/bench_smoke summaries. All 66 have
  source "sim", build_id 0f402cc7, git_hash f402cc7, model_git_hash 72da65248adf. Across them, 183,808 outputs were compared
  with **0 mismatches** and **0 counter diffs** (all 12 counters equal the model in every record), and every invariant held.
- All 49 older records (and their bench summaries and session.json), which carry a `-dirty` host hash, were moved with
  `git mv` to results/sim/archive_dirty/. The server (`read_dir`) and make_plots (`results/<source>/*.json`) do not read
  that directory, because neither search is recursive.
- results/sim/session.json is **not committed**: the sweep and cosim-check hosts updated it concurrently, so its totals lost
  updates (it reads 56 GEMMs instead of 66). The host has no command that rebuilds it from records.

### SIGN-OFF for KG2 candidate RTL dc1dea0 (H-1 display-orientation defaults), 2026-10-24 13:37-13:46 PDT
RTL: last commit touching rtl/ = dc1dea0 (`LEFT_IS_D1` 1->0, `AN0_IS_RIGHT` 1->0 in top.sv/board_io.sv). New in tb:
`display` suite (bee1cec), see the table above. No earlier system/cosim test checked pin-level display orientation.
- `scripts/wsl 'SPLITSYS_CACHE_TAG=ve make -C tb/system SIGNOFF=1 SEED=1 WORKERS=18'` -> **625/625 PASS**
  (the previous 617 = selftest 3, strip 100, full 200, directed 262, modeswitch 52, all pass; plus display 8/8),
  260 s, `sign-off eligible: YES`. Every one of the 12 counters equal to the model on every job (model_git_hash
  72da65248adf, latencies L_PE 3, L_RD 2, L_SETUP 16, L_RQ 7, L_WB 1, L_ACCEPT 1, L_DONE 1, provisional=false).
  Provenance: `head=bee1cec rtl=bee1cec tb=b020f8f golden=382bda8 vec=8b48b12 cycle=c8b8125 spec_rs=d4fda53 spec=8a4691f run=20261024-133733-19964 ns=ve`
- `scripts/wsl 'SPLITSYS_CACHE_TAG=ve make -C tb cosim-check RESULTS=$HOME/.cache/splitsys/ve/results'` -> **8/8 host
  steps PASS** (ping, selftest FULL 6c1d81af / STRIP 8b1c6e80, ffn_up M=1 strip, ffn_up M=1 full, qkv M=8 both,
  attn_out M=5 both int32, bench smoke, ping), 10 GEMMs, 2,400,256 MACs, 0 mismatches, `sign-off eligible: YES`.
  Records in `~/.cache/splitsys/ve/results/sim/` (not the repo). Provenance:
  `head=7b3af2c rtl=7b3af2c tb=b020f8f golden=382bda8 vec=8b48b12 cycle=c8b8125 spec_rs=d4fda53 spec=8a4691f run=20261024-134230-25213 ns=ve`
  (7b3af2c vs bee1cec: fpga/known_good and results/hw commits only; rtl/ and tb/ identical).

### SIGN-OFF for the KG3 candidate (D-020: BAUD 1,562,500, BAUD_DIV 4), 2026-10-24 21:48-21:57 PDT
RTL source is unchanged. Spec 21dd685 regenerated the package (BAUD_DIV 54 -> 4). HEAD is ac893cd (tb: real-divisor UART
tests and cosim/system `UART_DIV=0`). **Speed note:** both runs below use the sim-only divisor `UART_DIV_OVERRIDE=1`
(16 clk/bit). Real-divisor coverage comes from tb/uart (see tb/SIGNOFF.md) and from the UART_DIV=0 smoke below.
- `scripts/wsl 'SPLITSYS_CACHE_TAG=ve make -C tb/system SIGNOFF=1 SEED=1 WORKERS=18'` -> **625/625 PASS**
  (display 8, selftest 3, strip 100, modeswitch 52, full 200, directed 262). It took 220 s, `sign-off eligible: YES`, and
  all 12 counters equal the model on every job. Provenance:
  `head=ac893cd rtl=ac893cd tb=4d4b256 golden=382bda8 vec=8b48b12 cycle=c8b8125 spec_rs=8abf9ec spec=04f481c run=20261024-215046-65985 ns=ve`
- `scripts/wsl 'SPLITSYS_CACHE_TAG=ve make -C tb cosim-check RESULTS=$HOME/.cache/splitsys/ve/results'` -> **8/8 host
  steps PASS**: ping, selftest, ffn_up M=1 strip, ffn_up M=1 full, qkv M=8 both, attn_out M=5 both int32, bench smoke, ping.
  That is 10 GEMMs with 0 mismatches, PING build_id 0ac893cd, and `sign-off eligible: YES`. New since KG2: splitsys-host is
  built from the **committed** tree (`HOST_REV=HEAD` -> `git clone --shared` at ac893cd in `~/.cache/splitsys/host_rev/`).
  The run records therefore carry `git_hash "ac893cd"`, not `-dirty`, even though another agent had uncommitted sw/crates/host
  edits at the time. Records are in `~/.cache/splitsys/ve/results/sim/`, not the repo. Provenance:
  `head=ac893cd rtl=ac893cd tb=4d4b256 golden=382bda8 vec=8b48b12 cycle=c8b8125 spec_rs=8abf9ec spec=04f481c run=20261024-215500-72554 ns=ve host=HEAD:ac893cd`
- **Real-divisor system smoke** (no override: `top.UART_DIV_OVERRIDE=0`, RTL uses package BAUD_DIV 4, harness 64 clk/bit):
  `scripts/wsl 'SPLITSYS_CACHE_TAG=ve make -C tb/system UART_DIV=0 SUITES=selftest,full,strip N_FULL=2 N_STRIP=2 WORKERS=4 SEED=1'`
  -> **6/7**. These pass: full 2/2 and strip 2/2, including the 64x256x256 max shape (100,579 B in, 17,077 B out over the
  real-divisor UART, counters equal the model); selftest_opcode; and selftest_btn0. Each case starts with PING.
  **selftest_busy fails its own precondition**: `test window too long: 151079 sim cycles since RUN_GEMM >= job length 78594`.
  Its 8 probe frames during the busy window need about 151k cycles of UART time at 640 clk/byte, which is longer than the
  longest legal job. This is a test limitation, not an RTL bug: the BUSY/POLL checks never ran, and the case passes at
  UART_DIV=1 in the 625/625 run. Provenance (eligible YES, not a SIGNOFF=1 run because of this known case):
  `head=ac893cd rtl=ac893cd tb=4d4b256 golden=382bda8 vec=8b48b12 cycle=c8b8125 spec_rs=8abf9ec spec=04f481c run=20261024-214826-62441 ns=ve`
