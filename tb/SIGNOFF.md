# Unit sign-off record (owner: verification-engineer)

## How to re-run
`scripts/wsl 'make -C tb <module> RTL_REV=<hash> SEED=<seed>'`, or every suite at once with
`scripts/wsl 'make -C tb -j8 units RTL_REV=<hash>'`.

- **RTL under test:** by default `make -C tb` tests the **committed** `rtl/` at `HEAD`. It is exported
  with `git archive` to `~/.cache/splitsys/rtl_rev/<hash>/rtl`, never taken from the working tree.
  `RTL_REV=worktree` tests uncommitted edits; use it only as a pre-commit check, never for sign-off.
- **Fresh runs:** every suite run deletes its `results.xml` and re-runs the simulator. The top Makefile
  refuses to report a pass if no fresh `results.xml` appears. This closes a harness hole found at 09:55:
  under `make -j`, cocotb's `sim` target could skip the run and still exit 0.
- **Expected values:**
  - Numerics come only from Rust:
    - `splitsys_golden` through `tb/oracle`, a line-protocol CLI. Every answer it gives is the return
      value of a public golden function: `gemm_i32`, `requant`, `requant_is_tie`, `requant_unclamped`,
      `crc8`, `fletcher32`, `packet::request_frame` and `packet::response_frame`.
    - `sw/crates/vec` unit JSON (requant).
    - `splitsys-cycle predict --timeline` for counter values.
  - Identity checks need no vectors: skew index shift, UART bytes, the buffer memory image, and
    packet memory round trips.
  - Latencies are read from `rtl/gen/splitsys_pkg.sv` at test time. Each test asserts that the measured
    latency equals the package constant.

## Sign-off at RTL tree 64848bf (rtl/ last changed in 6171191), SEED=1 and SEED=2, SCALE=1

`make -C tb -j8 units` passed **8/8 suites, 50/50 tests**, for SEED=1 and SEED=2. The whole SEED=1 run took 83 s wall (after the 09:55 harness fix). Wall times below
are from runs in parallel under `-j8`. Standalone runs are all under 2 min; packet takes 83 s alone.

| Module | Command | Tests | Random volume (per seed) | Directed cases covered (SPEC §6) | Result |
| --- | --- | --- | --- | --- | --- |
| pe | `make -C tb pe` | 9 | 10,000 MAC streams (K 1..256, vld gaps carrying garbage), ≈283k cycles | (−128)(−128), (−128)(127), (127)(−128) and (127)(127), singly and at K=256. Zeros, and clear to zero. `first` mid-stream. Back-to-back clears (K=1 ×32 and K=2 ×16). vld gaps. **`first`=1 with vld=0 ignored** (golden flagged this gap). Reset mid-accumulation. 1-stage forward of a, w, vld and first checked every cycle. acc holds when there is no MAC. **Measured latency 3 == L_PE.** | PASS 9/9 (both seeds) |
| skew | `make -C tb skew` | 6 | 20,000 cycles × 16 lanes | Per-lane impulse delays for A (D=r), W top (D=c) and the W row-4 instance (D=4+c). The SPEC v1.1 §3.3 16-lane unit with W_SKEW_BASE_STRIP (lane 8+c → 4+c, D-008). FLAGS=0 ties vld and first to 0. No cross-lane leakage. Round trip is identity. Reset clears all stages. | PASS 6/6. Sensitivity: at RTL 925307f (old `BASE[j]+j`), `test_spec_w16_single_unit` fails as it should. |
| requant | `make -C tb requant` | 4 | vec `unit requant` (seed = SEED): 100,152 cases (14,724 ties, 61,668 saturating) plus 2,048 random back-to-back beats via the oracle | All 152 named vec cases: shift 1 and 31, ties at every shift (both signs), ± saturation edges, mult 1 and 32767, product bit 47, int32 extremes. Int32 bypass beats interleaved (identity). Parameter write takes effect the next cycle, captured per beat. **Measured latency 6 == L_RQ − 1 (D-016).** | PASS 4/4 |
| uart (rx+tx) | `make -C tb uart` | 5 | 10,000-byte tx→rx loopback (DIV_OVERRIDE=2), 300 bit-banged rx bytes | tx: 8N1, LSB first, each bit exactly DIV·16 clocks (checked at the first, middle and last cycle of every bit), 10-bit+2-clock spacing back to back. rx at baud error 0, ±1.5% and ±3%, with idle gaps including 0. Stop bit low → `rx_frame_err` with no byte. A quarter-bit glitch is rejected. Recovery afterwards. rx_valid and frame_err are never high together. **Spec divisor:** BAUD_DIV=54 == round(CLK_HZ/(BAUD·16)), measured bit = 864 clocks (115,741 baud, 0.47% error), loopback exact. | PASS 5/5 |
| buffers | `make -C tb buffers` | 5 | 12,000 random cycles over the 4 geometries, both ports, small address window | WBUF, ABUF, OBUF and RQPARAM geometries from the package. **Measured A and B read latency 2 == L_RD.** Power-up zero. First and last word written from either port. Byte enables, each lane alone and random masks (D-002). Same-cycle host write with array read of the same word returns OLD (read-first), both directions. a_rvalid pulse. a_rdata held across writes. b_rdata held while b_en=0. | PASS 5/5 |
| counters | `make -C tb counters` | 6 | 24 random legal descriptors (both modes, both flags, edge M/N/K) back to back without reset | Taps generated from the `splitsys-cycle` timeline. **All 12 counters equal the model**, in spec.toml order (CTR_* index checked). §3.7 invariants on the read-back values (mac_useful == M·N·K, Σq == mac_useful, total == Σparts + L_ACCEPT + L_DONE). Idle job (total=1). Taps in the start cycle and after done are not counted. Frozen after done. Cleared by the next start. A live snap returns an exact prefix (t+2). rd_data has 1-cycle latency; indices ≥ 12 read 0. Mutant (q0 reads q1's input) caught. | PASS 6/6 |
| packet | `make -C tb packet` (tb sets the sim-only override `RX_TIMEOUT_CYC=2000`) | 9 | 10,000 random frames back to back, payloads up to 1,024 B (23 frames ≥ 1,000 B), random tx backpressure, memory-image scoreboard | PING (SPLT, version, BUILD_ID LE). WRITE_MEM/READ_MEM in all 4 regions: first and last byte, unaligned, partial words, 1,019-byte writes and 1,024-byte reads. 40 bad CRCs → NACK_CRC with no side effect. Unknown opcode → NACK_OPCODE. Precedence CRC > OPCODE > BUSY > RANGE. BUSY set (PING, POLL, READ_COUNTERS and SET_DISPLAY still served). 71 NACK_RANGE length and field cases. 52 RUN_GEMM descriptors covering every §3.5 rule at exact fit and one over. POLL idle → busy → done. READ_COUNTERS returns 72 B. SELFTEST returns 10 B. Truncated frame → silent timeout → recovery. len > 1024 is dropped, then NACK_RANGE after the idle timeout. Bus rules asserted: read data exactly L_RD after mem_re, ≤ 1 write per 2 cycles, run_valid held. | PASS 9/9 |
| array (FULL+STRIP, ES=1 and ES=0) | `make -C tb array` | 6 | 500 random FULL tiles (ES=1 and ES=0 both checked) and 500 random STRIP tiles, K 1..256, random in-bounds masks | FULL: identity W, all-ones, all −128 and 127×−128 at K=256, **single nonzero element at each of the 64 output positions**. STRIP: **single nonzero at each of the 64 positions of the 4×16 tile** (every PE in all 4 quadrants, no leakage), garbage on A lanes 4–7 and on W lanes 8–15 in FULL is ignored, lower-half alignment on dense tiles. Mode switching FULL→STRIP→FULL… with no reset (32 tiles). ES=0 with mode_strip=1 still computes FULL. **Timing from the package:** results final in cycle t0+(K−1)+(ROWS−1)+(COLS−1)+L_PE. drain_data exactly 1 cycle after drain_row (the D-016 drain-read register). mac_cnt and mac_cnt_q exact every cycle (MAC entering PE(r,c) at t counts at t+2). Four mutants all caught: row-4 W skew base 3, STRIP lower-half col mask, STRIP lower-half drain, A-source mux removed. | PASS 6/6 |

## History
- 09:12: pe 8/8 and skew 6/6 at rtl 36771b6.
- 09:20: requant latency failed (6 vs L_RQ 4) → BUG-001. Closed by D-016 (d6e3bcf); requant re-signed off at 125ee10.
- 09:55: harness fix (forced fresh simulator runs). Every suite was re-run after it at 64848bf.

## Sign-off at commit b53783d (S2 level-2 + S1 units re-run), `make -C tb signoff`, SEED=1, SCALE=1

The run was clean and eligible under the provenance rules (red-team S1 SEV-2 fix):

```
== provenance: head=b53783d rtl=b53783d tb=ddc358f golden=382bda8 vec=8b48b12 cycle=c8b8125 spec_rs=d4fda53 spec=8a4691f run=20261024-102904-4395 ns=ve1
== sign-off eligible: YES
```

**Command:** `scripts/wsl 'SPLITSYS_CACHE_TAG=<tag> make -C tb -j12 signoff'` passed **11/11 suites, 74/74 tests**, in 139 s wall.
The packet suite takes 124 s under `-j12` load and about 85 s alone.

Logs are in `~/.cache/splitsys/tb/ve1/logs/20261024-102904-4395/`. Later commits that change only `tb/SIGNOFF.md` or
`tb/BUGS.md` do not affect any test input.

| Suite | Tests | What is new since the S1 entry |
| --- | --- | --- |
| core (rtl/core.sv + counters.sv, wrapper `tb/core/core_tb.sv`, drives the mem bus and run interface directly) | 5 | **All 262 jobs in vec `directed.json`.** This covers every SPEC §6 Sequencer/Array case: ±128/127 at K=256 in int8 and int32, zeros, identity, all-ones, single nonzero at every output position, quadrant-only, requant ties and saturation, and edge shapes 1×1×1 … 64×256×256 in both modes. The int32 bypass jobs exercise FULL at 2 writes/row and STRIP at 4.<br>**Mode switching:** the `sequences` FULL→STRIP→FULL plus a 12-job alternating chain, with no reset.<br>**PE(7,7) zero-slack boundary (D-019):** K=1 and K=256, int8 and int32, both modes, nonzero a/w/out/rq bases. Built by golden `Job::new` via the new `tb/oracle job` command.<br>**Random:** 40 vec `gemm` jobs (seed 1, random bases) and 24 golden `Job` jobs (random bases and requant params).<br>**Checks on every job:** OBUF is compared on `compare_ranges` only (D-006), after the in-bounds bytes are poisoned with ~expected so a missing write cannot pass. **All 12 counters equal `splitsys-cycle predict` exactly**, and the §3.7 invariants hold. This independently confirms the D-017 latencies on the real sequencer.<br>**Sensitivity:** FLUSH±1 mutants are caught with an exact per-counter diff. |
| core_ns (core with STRIP_EN=0) | 5 | Every FULL job above passes, with counters equal to the model (STRIP jobs skipped). |
| packet | 12 | RUN_GEMM accept/reject expectations now come from golden `Descriptor::from_bytes` + `validate` (`oracle descvalid`); SPEC-text intent is cross-checked. New: POLL priority busy > error > done > idle (`seq_error` level). New: exact RX timeout (D-019): g = RX_TIMEOUT continues, g = RX_TIMEOUT+1 resets, at several split points; both off-by-one mutants are caught. New: strict request/response, i.e. frames arriving during execution or during the response are discarded with no side effect. |
| packet_ns (packet with STRIP_EN=0) | 1 | All 52 RUN_GEMM descriptors with STRIP_EN=0 (golden + SPEC rule 1): STRIP gets NACK_RANGE. |
| pe, skew, uart, counters, buffers, requant, array | 9/6/5/6/5/4/6 | Unchanged tests, re-run clean at b53783d. |
