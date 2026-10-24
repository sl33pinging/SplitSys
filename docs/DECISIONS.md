# SplitSys Decisions Log

Append-only. Owner: `spec-steward`. Each entry: date, decision, reason, affected files and consumers. Machine values live in `spec/spec.toml`; prose in `docs/SPEC.md` (v1.1 introduced D-001 to D-015).

---

### D-001 — Fletcher-32 definition and the bytes it covers (2026-10-24)
**Decision.** Standard Fletcher-32 over 16-bit little-endian words (`byte[2i] | byte[2i+1] << 8`), odd trailing byte zero-padded, both sums initialized to 0 and kept in 0..65534 (mod 65535), result `(sum2 << 16) | sum1`. The self-test sums the run's C extent: OBUF words `out_base .. out_base + ceil(N/16)*M_pad - 1` in word order, each as its 16-byte LE image. Self-test descriptors must have `M % 8 == 0`, `N % 16 == 0`, flags = int8.
**Reason.** SPEC v1.0 named Fletcher-32 but did not define byte order, init, modulus, or extent. LE words match the rest of the contract and the common reference implementation (check values "abcde" → 0xF04FC729, "abcdef" → 0x56502D2A, pinned in spec.toml and validated by codegen). Summing raw OBUF words in address order is the simplest hardware walk; restricting self-test shapes avoids summing unspecified padding (D-006). golden-modeler independently proposed the same definition (`sw/crates/golden/src/checksum.rs`), so no change is needed there.
**Affects.** SPEC §3.9; spec.toml `[selftest]`; golden (checksum, ROM image), vec (self-test ROM shapes), rtl `selftest.sv`.

### D-002 — Byte addressing per region (2026-10-24)
**Decision.** `byte_addr = word_index * WORD_BYTES + lane`, lane 0 = least-significant byte. WORD_BYTES: WBUF 16, ABUF 8, OBUF 16, RQPARAM 4. Unaligned access allowed with byte enables.
**Reason.** Confirms the lead's proposal; it is the only rule consistent with "all multi-byte values are little-endian" and with lane numbering in §1.3.
**Affects.** SPEC §1.3, §3.4; host (WRITE_MEM/READ_MEM), rtl `buffers.sv`, packet layer.

### D-003 — Only two legal `flags` values (2026-10-24)
**Decision.** `flags = 0x01` (requant_en=1, out_int32=0: int8 output) or `flags = 0x02` (requant_en=0, out_int32=1: int32 bypass). Every other value, including 0x00 and 0x03, is NACK_RANGE. Bit constants stay `FLAG_REQUANT_EN = 0x01`, `FLAG_OUT_INT32 = 0x02`; the legal combined values are additionally exported as `FLAGS_INT8 = 0x01` and `FLAGS_INT32 = 0x02` (plural = whole byte, singular = one bit).
**Reason.** v1.0 §1.2 described bypass as `requant_en = 0` while §3.5 also defined `out_int32`, leaving two combinations meaningless. Lead's proposed rule; adopted unchanged.
**Affects.** SPEC §1.2, §3.5; golden descriptor validation, host, rtl sequencer, cycle (`t_drain` should key on the flags value).

### D-004 — int32 bypass drain length is per mode (2026-10-24)
**Decision.** `T_DRAIN`: FULL int8 8, STRIP int8 4, **FULL int32 16 (8 rows × 2 writes)**, **STRIP int32 16 (4 rows × 4 writes)**. New consts `T_DRAIN_FULL_INT32`, `T_DRAIN_STRIP_INT32`, `INT32_WRITES_PER_ROW_FULL = 2`, `INT32_WRITES_PER_ROW_STRIP = 4`, `INT32_WORDS_PER_ROW = 4` (layout). `spec.toml` `drain.int32_factor` removed; codegen checks both int32 lengths against ROWS, COLS and the OBUF width.
**Reason.** v1.0 §4 said "×4 for int32 bypass" in both modes, but the §1.3 int32 layout puts a 16-column block row in 4 words; a FULL tile owns only 8 of those columns (32 bytes = 2 words). Writing 4 words would clobber the other half-block's columns. Raised by perf-modeler via the lead.
**Transition.** Rust keeps a LEGACY `INT32_DRAIN_FACTOR = 4` (STRIP-only, wrong for FULL) so `splitsys-cycle` still compiles; it is removed once perf-modeler migrates. It is not in the SV package.
**Affects.** SPEC §3.5, §3.6, §4; cycle (`t_drain`), rtl sequencer/drain.

### D-005 — Descriptor units and RUN_GEMM validation (2026-10-24)
**Decision.** `a_base`, `w_base`, `out_base` are word indices in their own buffer's word size; `rq_base` is an RQPARAM entry index. RUN_GEMM accepts only if (`MB = ceil(M/8)`, `M_pad = 8*MB`, `NB = ceil(N/16)`): mode is FULL, or STRIP when ENABLE_STRIP; flags legal (D-003); M, K, N in range; `a_base + MB*K ≤ ABUF_WORDS`; `w_base + NB*K ≤ WBUF_WORDS`; `out_base + NB*M_pad*X ≤ OBUF_WORDS` (X = 1 int8, 4 int32); `rq_base + N ≤ RQ_ENTRIES` (for both flags values); `reserved == 0`; payload length 24. Otherwise NACK_RANGE. BUSY is checked first. Arithmetic is exact (no wrap).
**Reason.** v1.0 said "word index" but gave no range checks, so RTL and host could disagree on NACK_RANGE. Buffer sizes fit every legal int8 job at base 0 (checked by codegen), but **int32 bypass at the maximum shape needs 4096 OBUF words > 1024**: bypass is restricted by check 6, which limits it to `NB*M_pad ≤ 256` (enough for bring-up).
**Affects.** SPEC §3.5; golden descriptor validation, host, rtl sequencer/packet, verification (NACK tests).

### D-006 — Out-of-bounds outputs are unspecified (2026-10-24)
**Decision.** OBUF bytes that are not a logical in-bounds `C[m, n]` (padding rows or columns) may hold any value after a run. All comparisons use in-bounds elements only. RQPARAM reads for padding columns may go past `rq_base+N-1` and wrap modulo RQ_ENTRIES.
**Reason.** STRIP never writes rows `ceil(M/4)*4 .. M_pad-1`, and FULL with an odd number of 8-column tiles never writes lanes 8–15 of the last block, so the raw OBUF extent is not fully defined in general. Requiring zero-fill would add RTL masking for no demo value.
**Affects.** SPEC §1.3, §3.9; golden/vec comparisons, verification scoreboards, host verify.

### D-007 — Counter readback order (2026-10-24)
**Decision.** READ_COUNTERS order: `cycles_total, cycles_setup, cycles_stream, cycles_flush, cycles_drain, cycles_stall, mac_useful, mac_useful_q0..q3, tiles` (indices 0..11, `CTR_*`), 6 bytes LE each, 72 bytes.
**Reason.** Matches the DEMO.md §4 results schema. v1.0 §3.7's table listed setup after drain; the table is now in readback order and marked normative.
**Affects.** SPEC §3.7; rtl counters, host decode, dashboard, cycle `as_array`.

### D-008 — Skew delay formula (2026-10-24)
**Decision.** Skew lane j is delayed `BASE[j] + (j mod COLS)`. W lanes 8–15 in STRIP: base 4, so lane 8+c is delayed `4 + c`.
**Reason.** v1.0 §3.3 wrote `BASE[j] + j`, which gives `12 + c` for lane 8+c and contradicts §3.2 (`4 + c`), the alignment proof, and the role-file contract. §3.2 is the one that is right.
**Affects.** SPEC §3.3; rtl `skew.sv`/`array.sv`, verification skew tests (lane delays incl. base 4).

### D-009 — Benchmark shapes in spec.toml (2026-10-24)
**Decision.** `[bench]` holds the §1.1 shapes (qkv 64×192, attn_out 64×64, ffn_up 64×256, ffn_down 256×64), `m_sweep = [1,2,4,8,16,32,64]`, `m_decode = 1`, `m_prefill = 64`. Rust: `BENCH_SHAPES: [BenchShape; 4]`, `BENCH_M_SWEEP`, `BENCH_M_DECODE`, `BENCH_M_PREFILL`. Not emitted in SV (RTL does not need it).
**Reason.** cycle hard-coded the list in `report.rs`; golden `vec bench` and host need the same list. Lead request (optional, cheap).
**Affects.** cycle, vec, host.

### D-010 — Protocol details and RX_TIMEOUT (2026-10-24)
**Decision.** `RX_TIMEOUT = 1,000,000` cycles (10 ms). Error precedence: NACK_CRC, NACK_OPCODE, BUSY, NACK_RANGE; error responses have len 0. Oversize frames are dropped, then NACK_RANGE after RX_TIMEOUT idle. BUSY applies to RUN_GEMM, SELFTEST, WRITE_MEM, READ_MEM; PING, POLL, READ_COUNTERS, SET_DISPLAY are always served. Exact payload lengths per opcode (PING 9 bytes in response, READ_MEM request 7, SELFTEST response 10, SET_DISPLAY 3, ...); a wrong length gets NACK_RANGE. POLL ERROR is reserved. SELFTEST responds after both runs. `PROTO_VERSION = 1`.
**Reason.** v1.0 gave no number for RX_TIMEOUT and no precedence/BUSY rules, which host and RTL tests both need. 10 ms is about 115 byte times at 115200 baud, long enough to cover USB-UART scheduling gaps between host USB packets and short enough to be invisible. Codegen rejects a timeout below 2 byte times. Testbenches may override the module parameter.
**Affects.** SPEC §3.8; rtl uart_rx/packet, host driver, cosim bridge, verification.

### D-011 — Mapping T_tile terms to counters (2026-10-24)
**Decision.** Per tile: `cycles_setup += L_SETUP` (TILE_SETUP), `cycles_stream += L_RD + K` (STREAM), `cycles_flush += (R-1)+(C-1)+L_PE` (FLUSH), `cycles_drain += T_DRAIN + L_RQ + L_WB` (DRAIN, then WRITEBACK lasting `L_RQ + L_WB`). `L_ACCEPT` and `L_DONE` appear only in `cycles_total`. A stall cycle counts only in `cycles_stall`, so `cycles_total = L_ACCEPT + Σ + cycles_stall + L_DONE` stays exact.
**Reason.** Confirms the lead's proposal; it is the only split consistent with both the §3.7 state→counter table and the §4 T_tile formula. perf-modeler already implemented it (8aeedea).
**Affects.** SPEC §3.5 (state table), §3.7, §4; rtl sequencer/counters, cycle.

### D-012 — Provisional latency values (2026-10-24)
**Decision.** `L_PE = 3`, `L_RD = 2` (from SPEC), `L_SETUP = 16` (one RQ param per cycle, 16 lanes), `L_RQ = 4`, `L_WB = 1`, `L_ACCEPT = 1`, `L_DONE = 1`. All `provisional = true` (`*_PROVISIONAL` consts, `LATENCIES` table, `// PROVISIONAL` in SV). `L_SETUP` is the same in both modes.
**Reason.** Placeholders so models and RTL can start; rtl-designer sets the real values in spec.toml (through spec-steward) and flips `provisional`. Results produced with provisional values must say so.
**Affects.** spec.toml `[latency.*]`; cycle, results/model, rtl.

### D-013 — Verilator UNUSEDPARAM waiver in the generated package (2026-10-24)
**Decision.** `splitsys_pkg.sv` wraps its body in `/* verilator lint_off UNUSEDPARAM */ ... lint_on`.
**Reason.** Verilator 5.052 `-Wall` reports every package localparam that a module importing `splitsys_pkg::*` does not use (89 warnings in a smoke top). A constants package is never fully used by one module. The waiver is limited to this file and this one warning; Vivado ignores the comment.
**Affects.** rtl/gen only; RTL lint commands remain `-Wall` clean.

### D-014 — UART divider guard (2026-10-24)
**Decision.** Codegen derives `BAUD_DIV = round(CLK_HZ / (BAUD * OVERSAMPLE))` (54 at 115200) and rejects any configuration whose integer-divider baud error exceeds `max_baud_error_permille` (20 = 2%).
**Reason.** SPEC §3.8 plans to try 921600 baud later. At 100 MHz with 16× oversampling, the divider becomes 7, a 3.1% error, which is unreliable. Raising the baud needs either a fractional (accumulator) baud generator or a baud such as 1,562,500 (divider 4, exact). Codegen makes that failure loud instead of silent.
**Affects.** spec.toml `[uart]`; rtl uart_rx/uart_tx, host.

### D-015 — Rust/SV naming and types of generated constants (2026-10-24)
**Decision.** Names are identical in SV and Rust. Rust types: dimensions, sizes, offsets, and indices are `usize`; latencies and drain lengths are `u64`; byte-valued codes (modes, opcodes, status, poll, region ids, flags, sync, CRC) are `u8`; requant limits are `u32`; clock/baud/timeout are `u64`. SV: integers are `localparam int unsigned`, byte codes `logic [7:0]`, and the enums `mode_e`, `region_e`, `opcode_e`, `status_e`, `poll_e`, whose members are the same names (`MODE_FULL`, `REGION_WBUF`, `OP_PING`, `ST_ACK`, `POLL_IDLE`). The RQPARAM buffer uses prefix `RQ_` (`RQ_ENTRIES`, `RQ_WORD_BYTES`, `RQ_AW`); its region id is `REGION_RQPARAM`. No structs, `let`, or classes in SV, for Vivado compatibility.
**Reason.** Follows the lead's S0 coordination names exactly; no deviation.
**Affects.** All consumers.

### D-016 — First RTL-measured latencies; legacy INT32_DRAIN_FACTOR removed (2026-10-24)
**Decision.** `L_PE = 3` **confirmed** (operands at the PE port to `acc_out`), `provisional = false`. `L_RD = 2` **confirmed** on both BRAM ports, `provisional = false`. `L_RQ = 7` (was 4): measured from DRAIN presenting `drain_row` to requant output valid, made of 1 registered drain-read mux in the array plus 6 requant stages; it stays `provisional = true` until the sequencer lands. `L_WB = 1` unchanged and provisional, because the sequencer will register the OBUF write. `L_SETUP`, `L_ACCEPT`, `L_DONE` stay provisional, because the sequencer defines them. FLUSH = `(R-1)+(C-1)+L_PE = 17` was confirmed, with one register between the BRAM output and the array input. The Rust-only LEGACY `INT32_DRAIN_FACTOR` (D-004 transition) is deleted: no consumer uses it any more (perf-modeler migrated in 37d0b2a), and a test asserts it is not generated.
**Reason.** rtl-designer A's report (Vivado OOC + smoke tests), relayed by the lead.
**Note.** `L_RQ` is a **sequencer-level** latency (drain mux + requant). The standalone `requant.sv` module latency is `L_RQ - 1 = 6`. A unit test of `requant.sv` alone must expect 6, not `L_RQ`. See tb BUG-001, which compared 6 against the old L_RQ = 4.
**Affects.** spec.toml `[latency.*]`; rtl/gen; generated.rs; cycle (results must be regenerated); tb requant latency test; rtl sequencer.

### D-017 — All seven latencies confirmed by RTL (2026-10-24)
**Decision.** `L_SETUP = 16`, `L_RQ = 7`, `L_WB = 1`, `L_ACCEPT = 1`, `L_DONE = 1` are now `provisional = false`, joining `L_PE = 3` and `L_RD = 2` (D-016). Every `*_PROVISIONAL` const is false. They are now also emitted in SV as `localparam bit L_*_PROVISIONAL`.
**Reason.** rtl-designer's sequencer.sv and core.sv (3786ad1) implement exactly these values. Its Verilator smoke `tb_core` ran 283 golden jobs (FULL and STRIP, int8 and int32, edge shapes, K = 1 and 256, drain boundary) and found the OBUF bit-exact and **every counter equal to `splitsys-cycle` predict**. It also ran 175 FULL jobs with STRIP_EN = 0. verification-engineer's independent tb/core will re-check; if it disagrees, the value goes back to provisional through spec-steward.
**Affects.** spec.toml `[latency.*]`; generated files; perf-modeler must regenerate results with `latencies_provisional = false`.

### D-018 — SV/Rust name parity enforced by test (2026-10-24)
**Decision.** D-015 claimed identical names but the outputs had drifted. They are now unified, and the test `sv_and_rust_names_match` fails on any future drift:
- Rust `*_ADDR_BITS` was renamed to **`*_AW`** (`WBUF_AW`, `ABUF_AW`, `OBUF_AW`, `RQ_AW`; type u32), the name the RTL already uses. No Rust consumer used `*_ADDR_BITS` (grep of sw/, tb/), so no alias was kept.
- Rust gained `*_WORD_BITS` and **`PING_MAGIC_LE: u32`** (SV already had them).
- SV gained `DESC_SIZE_*` and `L_*_PROVISIONAL`.
- Both gained `OPERAND_PORT_OFFSET` (D-019) and `FLETCHER32_CHECK_ABCDEFGH = 0xEBE19591`. SPEC §3.9 listed three check values but only two existed; this is the value golden already tests.
**Documented Rust-only names** (tables and conveniences that have no SV equivalent): `SPEC_VERSION`, `PING_MAGIC` (`[u8; 4]`, the same bytes as `PING_MAGIC_LE`), `LATENCIES`, `REGIONS`, `OPCODES`, `STATUSES`, `COUNTER_NAMES`, `BENCH_SHAPES`, `BENCH_M_SWEEP`, `BENCH_M_DECODE`, `BENCH_M_PREFILL`. There are no SV-only names. Types still differ by language (D-015), for example `W_SKEW_BASE_STRIP` is a packed `logic [127:0]` in SV and `[usize; 16]` in Rust.
**Reason.** Lead review. No RTL rename was needed: Rust moved to the SV names, which rtl/ already uses (`*_AW` in core, packet, selftest, sequencer; `PING_MAGIC_LE` in packet).
**Affects.** Rust consumers only gain names. Nothing in rtl/ or tb/ changes.

### D-019 — Operand arrival offset, strict request/response, exact RX timeout, baud plan (2026-10-24)
**Decision.**
1. Word k is at the array input port in `s + OPERAND_PORT_OFFSET + k`, where `OPERAND_PORT_OFFSET = L_RD + 1` (one lane-select register between BRAM and array). The last word reaches the array in the first FLUSH cycle, and PE(7,7) is final exactly at the first DRAIN cycle (zero slack for PE(7,7); drain order gives FULL ≥ 7 and STRIP ≥ 3 cycles of margin). `OPERAND_PORT_OFFSET` is a derived constant in both languages.
2. The protocol is strictly request/response. Bytes that arrive while a command executes or while its response is sent are discarded, and the host must wait for each response.
3. RX timeout: with consecutive frame-byte `rx_valid` strobes at t and t+g, the frame continues iff `g ≤ RX_TIMEOUT`. Otherwise the parser resets at the end of cycle `t + RX_TIMEOUT`. The gap g includes one character time.
4. SPEC §3.8 no longer says "try 921600" (contradicted D-014). Allowed upgrades are 1,562,500 or 3,125,000 baud (exact dividers 4 and 2), or a fractional generator.
**Reason.** Lead review. Items 1–3 pin behavior that RTL already implements (rtl/sequencer.sv header, rtl/packet.sv `to_cnt == TIMEOUT-1` with no strobe) and that splitsys-cycle (`operand_port_offset`) and host must match, so that the zero-slack boundary and the timeout edge cannot drift.
**Affects.** SPEC §3.5, §3.8; host driver (wait for each response; do not pipeline requests), verification (timeout edge tests at g = RX_TIMEOUT and RX_TIMEOUT+1).
