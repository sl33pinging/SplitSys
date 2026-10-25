# SplitSys Technical Specification (v1.1)

This document is the contract between RTL, the Rust models, the host, and the dashboard. `spec-steward` owns it. Values marked **[spec.toml]** must be defined in `spec/spec.toml` and consumed through codegen, never hard-coded twice.

**Codegen.** `cd sw && cargo run -p splitsys-spec --bin gen` validates `spec/spec.toml` and writes `rtl/gen/splitsys_pkg.sv` (SystemVerilog `package splitsys_pkg`) and `sw/crates/spec/src/generated.rs` (re-exported as `splitsys_spec::*`). `--check` verifies both are current without writing. Names are identical on both sides (`ROWS`, `L_PE`, `WBUF_AW`, `PING_MAGIC_LE`, `OP_RUN_GEMM`, `ST_NACK_RANGE`, `CTR_TILES`, ...; a test enforces this, and the only exceptions are the Rust-only tables listed in D-018); SV opcodes, status codes, poll states, modes, and regions are `typedef enum logic [7:0]` (`opcode_e`, `status_e`, `poll_e`, `mode_e`, `region_e`). v1.1 resolves the ambiguities listed in `docs/DECISIONS.md` (D-001 to D-015; later entries follow).

---

## 0. Summary

| Item | Value |
| --- | --- |
| Board | RealDigital Urbana, AMD Spartan-7 XC7S50-CSGA324 (-2 speed grade, confirm) |
| Resources | 120 DSP48E1, 75 BRAM36 (~337 KB), 32,600 LUT, 65,200 FF, 5 MMCM |
| Core clock | 100 MHz target; fall back to 75 MHz rather than fight timing late at night |
| Array | 8×8 int8 processing elements, output-stationary, one DSP48E1 per PE |
| Modes | `FULL` (one 8×8), `STRIP` (behaves as 4×16), `INDEP` (four 4×4 jobs; stretch, reserved) |
| Peak | 64 MAC/cycle = 6.4 GMAC/s at 100 MHz |
| Host link | USB-UART, framed packets with CRC-8 |
| Claim | Measured utilization per mode, on LLM-shaped GEMMs, matches the cycle model. Bit-exact against the Rust golden model. |

---

## 1. Workload and numerics contract

### 1.1 Benchmark GEMMs

C[M×N] = A[M×K] · W[K×N]. One transformer block with d_model = 64, at M = 1 (decode) and M = 64 (prefill):

| Name | K | N | MACs at M = 1 | MACs at M = 64 |
| --- | --- | --- | --- | --- |
| `qkv` | 64 | 192 | 12,288 | 786,432 |
| `attn_out` | 64 | 64 | 4,096 | 262,144 |
| `ffn_up` | 64 | 256 | 16,384 | 1,048,576 |
| `ffn_down` | 256 | 64 | 16,384 | 1,048,576 |

Also run M ∈ {1, 2, 4, 8, 16, 32, 64} for the utilization-vs-M sweep (the headline chart).

The shapes and the M sweep live in **[spec.toml]** `[bench]` (`BENCH_SHAPES`, `BENCH_M_SWEEP`, `BENCH_M_DECODE`, `BENCH_M_PREFILL`); golden, cycle, and host read that one list (D-009).

### 1.2 Quantization (identical in RTL and in `golden`)

- **Weights:** int8, symmetric, per-output-column scale. **Activations:** int8, symmetric, per-tensor scale. No zero points.
- **Accumulation:** int32. Worst case at K = 256: 256 × 128 × 128 = 4,194,304, well inside int32. The PE accumulator is wider internally (DSP P register, 48 bits). The int32 value presented to drain is the low 32 bits, and saturation never occurs for K ≤ 256.
- **Requantization, per output column n:**
  ```
  t = (int64(acc) * int64(mult[n])) + (1 << (shift[n] - 1))
  y = clamp(t >>> shift[n], -128, 127)        // >>> = arithmetic shift (floor)
  ```
  - `mult[n]`: unsigned 15-bit, 1..32767 (positive scales only). `shift[n]`: 1..31.
  - The product fits in 48 bits signed (|acc| < 2^31, mult < 2^15).
  - Rounding is round-half-up toward +∞, which follows from floor(x + 0.5).
  - The activation scale is folded into `mult[n]` by the host.
- **Bypass:** with `flags.requant_en = 0, flags.out_int32 = 1` (flags = `FLAGS_INT32` = 0x02), the raw int32 accumulators are written (bring-up only; slower output path, see §3.6). The normal path is flags = `FLAGS_INT8` = 0x01 (`requant_en = 1, out_int32 = 0`). These are the **only two legal flags values**; any other value (0x00, 0x03, or any other bit set) is rejected with NACK_RANGE (D-003).

### 1.3 Data layouts (host packs, RTL streams; `golden` provides pack/unpack)

All multi-byte values are little-endian. Out-of-range rows and columns are **zero-padded by the host**, so the RTL never reads garbage.

- **A (activations)** in 8-row blocks: word64 index `mb*K + k` holds `A[8*mb + r, k]` in byte lane r, for r = 0..7. STRIP mode uses lanes 0–3 or 4–7 of the word (a 4-row half-block, selected by the sequencer).
- **W (weights)** in 16-column blocks: word128 index `nb*K + k` holds `W[k, 16*nb + c]` in byte lane c, for c = 0..15. FULL mode uses lanes 0–7 or 8–15 (an 8-column half-block); STRIP uses all 16.
- **C int8 output** in 16-column blocks: word128 index `nb*M_pad + m` holds `C[m, 16*nb + c]` in byte lane c. FULL writes 8 lanes with byte enables. `M_pad = ceil(M/8)*8`.
- **C int32 output (bypass)**: word128 index `(nb*M_pad + m)*4 + q` holds int32 lanes 4q..4q+3.
- **RQPARAM:** word32 index n = `{8'h0, shift[7:0], mult[15:0]}`. Byte image of entry n: bytes `4n, 4n+1` = mult (LE), `4n+2` = shift, `4n+3` = 0. Hardware does not range-check RQPARAM contents; the host must write mult in 1..32767 and shift in 1..31 for every column `rq_base .. rq_base+N-1`.
- **Word indices are relative to the descriptor base:** "word index i" above means buffer word `a_base + i`, `w_base + i`, `out_base + i`, and RQPARAM entry `rq_base + n`.
- **Out-of-bounds outputs are unspecified (D-006).** OBUF lanes or rows that do not hold a logical `C[m, n]` with m < M and n < N (padding rows `M..M_pad-1`, padding columns `N..16*ceil(N/16)-1`) may be written with any value or left untouched. Host, golden comparisons, and testbenches compare **in-bounds elements only**. Padding columns may read RQPARAM entries past `rq_base+N-1` (the RQPARAM address wraps modulo `RQ_ENTRIES`); this is harmless because those outputs are unspecified.

---

## 2. Architecture overview

```
 host (laptop) ──USB-UART──► [uart_rx]─►[packet: framing+CRC]─►[regfile/mem mux]──┐
       ▲                                                                            │
       └──────────────────── [uart_tx]◄─[packet]◄── responses, READ_MEM, counters ◄─┤
                                                                                    ▼
   ┌────────────── sequencer FSM (descriptor → tiles → stream → drain → writeback) ─┐
   │                    │ addr/mode                      │                         │
   ▼                    ▼                                ▼                         ▼
 ABUF (64b, 8 lanes) ─►[A skew]─► ┌──── 8×8 PE array ────┐ ─►[drain mux]─►[requant ×16]─► OBUF
 WBUF (128b, 16 lanes)─►[W skew]─►│ Q0 │ Q1             │                                  │
                                  │────┼────  row-4 mux ◄── W lanes 8..15 (STRIP)           │
                                  │ Q2 │ Q3             │                                  │
                                  └──────────────────────┘                                  │
 perf counters ◄── valid/busy taps from array + FSM                 board I/O: LEDs, 7-seg, buttons, self-test ROM
```

One clock domain. No DDR3 (out of scope).

---

## 3. Module specifications

### 3.1 PE (`rtl/pe.sv`)

- **Dataflow:** output-stationary. `a` moves right, `w` moves down, partial sums stay in place.
- **Ports:** `clk, rst, a_in[7:0] signed, a_vld_in, a_first_in, w_in[7:0] signed, a_out, a_vld_out, a_first_out, w_out, acc_out[31:0] signed`.
- **Behavior:** when `a_vld_in` is set, `acc <= a_first_in ? a*w : acc + a*w`. Operands are forwarded one register stage per PE in each direction.
- `first` and `vld` travel **with the activation**. Because A and W meet in lockstep, this is sufficient.
- **Implementation:** infer one DSP48E1 (`(* use_dsp = "yes" *)`) with input, multiply, and P registers. The PE pipeline latency `L_PE` **[spec.toml]** is the cycles from operand entry to `acc` updated = 3, confirmed by RTL (D-016).
- **Acceptance:** 64 DSPs in the synthesized array; no LUT multipliers.

### 3.2 Array and modes (`rtl/array.sv`)

Quadrants (4×4 each): Q0 = rows 0–3 × cols 0–3, Q1 = rows 0–3 × cols 4–7, Q2 = rows 4–7 × cols 0–3, Q3 = rows 4–7 × cols 4–7. In STRIP mode the upper half (Q0+Q1) and lower half (Q2+Q3) each act as a 4×8 slab, side by side logically to form 4×16.

| Mode | Output tile | A lanes used | W lanes used | Boundary muxes |
| --- | --- | --- | --- | --- |
| FULL (0) | 8 × 8 | 8 (rows 0–7) | 8 (top edge, cols 0–7) | none |
| STRIP (1) | 4 × 16 | 4, duplicated into rows 4–7 | 16: lanes 0–7 enter the top edge, lanes 8–15 enter at the row-4 boundary | W mux at row 4 (8 × 8 bits); A-source mux on rows 4–7 |
| INDEP (2) | four 4 × 4 | reserved (stretch) | reserved | both boundaries |

**STRIP wiring:**
- Rows 4–7 take A from the activation lanes of rows 0–3 (the same data and flags). The A-source mux sits **before** the A skew unit, so lane 4+i still receives its normal `4 + i` delay.
- At the row-4 boundary, the W input of rows 4–7 comes from W lanes 8–15, not from rows 0–3 above.
- **Skew alignment (important):** activation skew is identical in both modes (row r is delayed r cycles). To keep the lower half aligned, **W lanes 8–15 injected at row 4 must be delayed `4 + c` cycles** (c = column 0–7), not `c`. With that delay, PE(4+i, c) receives A[i,k] and W[k, 8+c] together at cycle k + 4 + i + c. The last PE finishes at the same cycle in both modes, so `T_stream` does not depend on mode.
- **Result mapping in STRIP:** PE(i, c) holds C[i, c] and PE(4+i, c) holds C[i, 8+c], for i = 0..3.

### 3.3 Skew units (`rtl/skew.sv`)

One parameterized module: `LANES`, `WIDTH`, and per-lane extra delay as a parameter array. Lane j is delayed `BASE[j] + (j mod COLS)` cycles, i.e. base plus its **column index within its 8-wide slab** (for A, `j mod COLS = j` = the row index), carrying data, vld, and first. The same module is used for A skew (base 0) and W skew (base 0 for lanes 0–7; base 4 for lanes 8–15, active only in STRIP). So W lane 8+c is delayed `4 + c`, matching §3.2, **not** `4 + 8 + c` (D-008; v1.0 wrote `BASE[j] + j`, which contradicted §3.2). Generated constants: `STRIP_W_LANE_FIRST = 8`, `STRIP_W_ENTRY_ROW = 4`, `STRIP_W_SKEW_BASE = 4`, `W_SKEW_BASE_STRIP` (per-lane, SV packed `[8*j +: 8]`). In FULL mode, a lane-select mux before the W skew routes WBUF lanes `8*(nb8 mod 2) .. +7` (the tile's half-block) into skew lanes 0–7; skew lanes 8–15 are unused.

### 3.4 Buffers (`rtl/buffers.sv`)

True dual-port BRAM (UG901 inference templates, `(* ram_style = "block" *)`): port A for host write and read, port B for the array. Read latency `L_RD` = 2 with output registers **[spec.toml]**.

| Buffer | Width | Default size **[spec.toml]** | BRAM36 (approx.) |
| --- | --- | --- | --- |
| WBUF | 128 b | 64 KiB (4096 words) | 16 |
| ABUF | 64 b | 16 KiB (2048 words) | 4 |
| OBUF | 128 b | 16 KiB (1024 words) | 4 |
| RQPARAM | 32 b | 256 entries | LUTRAM or 1 |
| Self-test ROM | per contents | small | ≤ 4 |

Host-side addressing: byte addresses through the packet layer. The buffer converts them to words and handles partial-word writes with byte enables.

**Byte addressing rule (D-002):** within a region, `byte_addr = word_index * WORD_BYTES + lane`, where lane 0 is the least significant byte of the word (little-endian). So `word_index = byte_addr / WORD_BYTES` and `lane = byte_addr mod WORD_BYTES`. Region sizes in bytes **[spec.toml]**: WBUF 65,536 (16 B/word), ABUF 16,384 (8 B/word), OBUF 16,384 (16 B/word), RQPARAM 1,024 (4 B/entry). Accesses need not be word-aligned; partial words use byte enables, and untouched bytes of a word keep their value.

### 3.5 Sequencer (`rtl/sequencer.sv`)

**Descriptor (24 bytes, little-endian) [spec.toml field map]:**

| Offset | Field | Type | Notes |
| --- | --- | --- | --- |
| 0 | mode | u8 | 0 FULL, 1 STRIP, 2 INDEP (reserved → NACK_RANGE) |
| 1 | flags | u8 | bit0 `requant_en`, bit1 `out_int32` |
| 2 | M | u16 | 1..64 |
| 4 | K | u16 | 1..256 |
| 6 | N | u16 | 1..256 |
| 8 | a_base | u32 | word index into ABUF |
| 12 | w_base | u32 | word index into WBUF |
| 16 | out_base | u32 | word index into OBUF |
| 20 | rq_base | u16 | index into RQPARAM for column 0 |
| 22 | reserved | u16 | 0 |

`a_base`, `w_base`, `out_base` are **word indices** in their buffer's own word size (8 B, 16 B, 16 B); `rq_base` is an RQPARAM **entry** index. They are not byte addresses (D-005).

**Validation (RUN_GEMM; RTL and host implement exactly these, D-005).** With `MB = ceil(M/8)`, `M_pad = 8*MB`, `NB = ceil(N/16)`, the descriptor is accepted only if all of the following hold; otherwise the response is **NACK_RANGE** and nothing runs. A RUN_GEMM while a job or self-test is running gets **BUSY** (checked first). A payload length other than 24 gets NACK_RANGE.
1. `mode ∈ {FULL}`, or `mode = STRIP` and the bitstream has `ENABLE_STRIP = 1`. INDEP (2) and values ≥ 3 → NACK_RANGE.
2. `flags ∈ {0x01 (int8), 0x02 (int32 bypass)}`.
3. `1 ≤ M ≤ 64`, `1 ≤ K ≤ 256`, `1 ≤ N ≤ 256`.
4. `a_base + MB*K ≤ ABUF_WORDS` (A extent).
5. `w_base + NB*K ≤ WBUF_WORDS` (W extent).
6. `out_base + NB*M_pad*X ≤ OBUF_WORDS`, with `X = 1` for int8 and `X = INT32_WORDS_PER_ROW = 4` for int32 bypass (C extent). At `out_base = 0` the int8 path fits every legal shape (16 × 64 = 1024 words); int32 bypass fits only `NB*M_pad ≤ 256` (e.g. M ≤ 64 with N ≤ 64).
7. `rq_base + N ≤ RQ_ENTRIES` (checked for both flags values; the host sends `rq_base = 0` in bypass).
8. `reserved = 0`.

All sums are evaluated without wraparound (the bases are u32; RTL may first require the upper bits of each base to be zero, which is equivalent since every buffer is smaller than 2^16 words).

**FSM:** `IDLE → (ACCEPT) → TILE_SETUP → STREAM → FLUSH → DRAIN → WRITEBACK → (next tile | DONE)`. State lengths per tile are exact and are what the counters measure (§3.7, §4):

| State | Cycles per tile | Counter |
| --- | --- | --- |
| ACCEPT (once per job) | `L_ACCEPT` | `cycles_total` only |
| TILE_SETUP | `L_SETUP` | `cycles_setup` |
| STREAM | `L_RD + K` | `cycles_stream` |
| FLUSH | `(R-1) + (C-1) + L_PE` = `14 + L_PE` | `cycles_flush` |
| DRAIN | `T_DRAIN(mode, flags)` | `cycles_drain` |
| WRITEBACK | `L_RQ + L_WB` | `cycles_drain` |
| DONE entry (once per job) | `L_DONE` | `cycles_total` only |

- **Tile order:** column-block outer, row-block inner. FULL tiles are 8×8; STRIP tiles are 4×16.
- **TILE_SETUP** loads the 16 (or 8) requant parameters for the tile's columns. It takes `L_SETUP` cycles **[spec.toml]**, in both modes and for both flags values (the length does not depend on mode, so the model has one constant).
- **STREAM** issues K reads per buffer, read k in cycle `s + k`, where s is the first STREAM cycle. The state lasts `L_RD + K` cycles. The address generator selects the half-word lanes per mode and tile parity.
- **Operand arrival (normative, D-019).** Read k's BRAM data appears in cycle `s + k + L_RD` and passes exactly **one** register (the lane-select register) before the array. So word k is at the array input port in cycle `s + OPERAND_PORT_OFFSET + k`, with `OPERAND_PORT_OFFSET = L_RD + 1` = 3 **[spec.toml, derived]**. It is at the port of PE(r, c) in `s + L_RD + 1 + k + r + c`, and PE(4+i, c) in STRIP also at `+ 4 + i + c` (§3.2). The **last word (k = K-1) reaches the array port in the first FLUSH cycle** `f0 = s + L_RD + K`. PE(r, c)'s accumulator is final from `f0 + r + c + L_PE`, so PE(7,7) is final exactly at the first DRAIN cycle `f0 + 14 + L_PE`: FLUSH has zero slack for PE(7,7) by construction. Drain reads row slots in order, so PE(7,·) is read last: FULL has ≥ 7 cycles of margin and STRIP ≥ 3 (rtl/sequencer.sv 3786ad1, `operand_port_offset` in splitsys-cycle). Changing the register count between BRAM and array changes `OPERAND_PORT_OFFSET`, and then the FLUSH length must change in spec.toml with it.
- **FLUSH** waits `(R-1)+(C-1)+L_PE` cycles for the last products to land.
- **DRAIN** reads results row by row, one OBUF write per cycle: FULL int8 reads 8 rows × 8 lanes (8 cycles), STRIP int8 reads 4 rows × 16 lanes (4 cycles). int32 bypass: FULL 8 rows × 2 writes = 16 cycles, STRIP 4 rows × 4 writes = 16 cycles (D-004).
- **WRITEBACK** waits `L_RQ + L_WB` cycles for the requant pipeline and the last OBUF write to complete.
- **MVP:** stages do not overlap; the array idles during drain. A later target: shadow accumulators that overlap drain with the next tile.
- **Bounds:** the sequencer computes in-bounds masks from M and N for the counters (padding is zero, so the math is unaffected).

### 3.6 Drain and requantization (`rtl/requant.sv`)

- 16 lanes, each implementing §1.2 bit-exactly. A 32×15 multiply needs **two DSP48E1 per lane**, or use LUT multipliers if the DSP budget is tight. The DSP budget is 64 (PE) + up to 32 (requant) = 96 ≤ 120.
- Pipeline latency `L_RQ` **[spec.toml]** is measured at sequencer level: from DRAIN presenting `drain_row` to requant output valid, including the 1-cycle registered drain-read mux in the array (D-016). The standalone `requant.sv` latency is therefore `L_RQ - 1`.
- **int8 path:** one 128-bit write per drained row (FULL uses byte enables for 8 lanes).
- **int32 bypass:** per the §1.3 int32 layout one 16-column block row is 4 OBUF words (`INT32_WORDS_PER_ROW`). A STRIP row covers all 16 columns → **4** full 128-bit writes; a FULL row covers 8 columns (half-block `p = nb8 mod 2`) → **2** full 128-bit writes, words `q = 2p, 2p+1` (D-004). This is slower, used only for bring-up, and its extra cycles are counted as `cycles_drain` (`T_DRAIN_FULL_INT32 = T_DRAIN_STRIP_INT32 = 16`).

### 3.7 Performance counters (`rtl/counters.sv`)

48-bit counters (`COUNTER_BITS`), cleared on an accepted `RUN_GEMM` (or at the start of each self-test GEMM), frozen at DONE. `READ_COUNTERS` returns all 12, 6 bytes each little-endian (72 bytes), **in exactly this order** (the index is `CTR_<NAME>` in both the SV package and Rust; this order also matches the DEMO.md §4 results schema, where `mac_useful_q` is the array `[q0, q1, q2, q3]`). The v1.0 table listed them in a different order; this table is normative (D-007).

| Index | Counter | Increments when |
| --- | --- | --- |
| 0 | `cycles_total` | every cycle from descriptor accept to DONE: `L_ACCEPT + Σ(per-tile states) + cycles_stall + L_DONE` |
| 1 | `cycles_setup` | FSM in TILE_SETUP |
| 2 | `cycles_stream` | FSM in STREAM |
| 3 | `cycles_flush` | FSM in FLUSH |
| 4 | `cycles_drain` | FSM in DRAIN or WRITEBACK |
| 5 | `cycles_stall` | the FSM is held waiting on buffer or output backpressure (0 in MVP unless a hazard exists). A stalled cycle increments **only** `cycles_stall` (and `cycles_total`), never a state counter, so the invariant below stays exact |
| 6 | `mac_useful` | += number of PEs performing an in-bounds MAC this cycle (popcount of 64 bits) |
| 7–10 | `mac_useful_q0` .. `mac_useful_q3` | the same, per quadrant (Q0..Q3 as in §3.2) |
| 11 | `tiles` | one per completed tile (at the end of its WRITEBACK) |

While a job runs, READ_COUNTERS returns live (not frozen) values.

**Invariants checked on every run:**
- `mac_useful == M*N*K` exactly.
- `sum(mac_useful_q) == mac_useful`.
- `cycles_total == cycles_setup + cycles_stream + cycles_flush + cycles_drain + cycles_stall + L_ACCEPT + L_DONE` (constants in spec.toml).

**Utilization** = `mac_useful / (64 * cycles_total)`.

### 3.8 Host link (`rtl/uart_rx.sv`, `uart_tx.sv`, `packet.sv`)

- **UART:** 8N1, 16× oversampling, `BAUD` **[spec.toml]** (**1,562,500**, `BAUD_DIV = 4`, exact at 100 MHz; D-020). One bit is 64 clock cycles and one 10-bit character is 640 cycles (6.4 µs). The previous value, 115200 (`BAUD_DIV = 54`, +0.47%), is what known-good bitstream KG2 was built with, and it remains the fallback: a KG2 board must be driven at 115200, and a board built from this spec at 1,562,500. The FPGA side is exact. The FT2232H bridge cannot hit 1,562,500 exactly (its nearest divisors are about +0.7% or −0.9% off), which is well inside the 2% budget. A bitstream at the new baud is not trusted until the loopback test passes 10,000 packets with zero errors **at that baud on hardware**. Only a baud that the integer 16× divider hits within 2% at 100 MHz is allowed. **921600 is not allowed:** the divider rounds to 7, a 3.1% error, and codegen rejects it (D-014). The only other exact option is 3,125,000 (divider 2), and it is not used: the FT2232H error is the same, it has half the timing margin per bit, and wall time is bounded by USB request/response latency anyway (D-020). Any other rate needs a fractional (accumulator) baud generator first.
- **Request frame:** `0xA5 | opcode u8 | len u16 | payload[len] | crc8`.
- **Response frame:** `0x5A | status u8 | len u16 | payload[len] | crc8`.
- **CRC-8:** polynomial 0x07, init 0x00, computed over every byte after the sync byte, up to and including the payload.
- **Maximum payload:** 1024 bytes. The host chunks larger transfers.
- **Status codes:** 0 ACK, 1 NACK_CRC, 2 NACK_OPCODE, 3 NACK_RANGE, 4 BUSY.

| Opcode | Name | Request payload | Response payload |
| --- | --- | --- | --- |
| 0x01 | PING | — | `"SPLT"`, version u8, build_id u32 |
| 0x10 | WRITE_MEM | region u8, byte_addr u32, data | — |
| 0x11 | READ_MEM | region u8, byte_addr u32, len u16 | data |
| 0x20 | RUN_GEMM | descriptor (24 B) | — (ACK on accept) |
| 0x21 | POLL | — | state u8 (0 idle, 1 busy, 2 done, 3 error) |
| 0x30 | READ_COUNTERS | — | counters, 6 B each, in spec.toml order |
| 0x40 | SELFTEST | — | pass_full u8, pass_strip u8, fletcher32 u32 ×2 |
| 0x50 | SET_DISPLAY | util_permille u16, mode u8 | — (drives the 7-segment display) |

**Regions:** 0 WBUF, 1 ABUF, 2 OBUF, 3 RQPARAM. An inter-byte gap longer than `RX_TIMEOUT` cycles resets the frame parser; the exact rule is in the protocol details below.

`build_id` is a 32-bit generic set at synthesis from the short git hash. The host and dashboard display it.

**Protocol details (v1.1, D-010).** All multi-byte fields little-endian, including `len`.
- **CRC-8** is CRC-8/SMBUS: MSB-first, no reflection, no final XOR; check value of ASCII `"123456789"` = 0xF4 (`CRC8_CHECK`). Request CRC covers `opcode, len(2), payload`; response CRC covers `status, len(2), payload`.
- **Strictly request/response (D-019).** The device parses one request, executes it, and sends its complete response before it parses the next request. **Bytes that arrive while a command executes or while its response is being sent are discarded** (they are not queued). The host must wait for each response, or for its own response timeout, before it sends the next request. A byte other than `0xA5` while the parser is hunting for a frame is ignored.
- **`RX_TIMEOUT` = 1,000,000 cycles (10 ms at 100 MHz), exact boundary (D-019).** Gaps are measured in core clock cycles between the `rx_valid` strobes of consecutive received bytes of one frame (`uart_rx` raises one single-cycle strobe per byte, at mid-stop-bit), starting from the sync byte. If a frame byte's strobe is in cycle t, the frame continues when the next strobe is in cycle t + g with **g ≤ RX_TIMEOUT**. If no strobe occurs in cycles t+1 .. t+RX_TIMEOUT, then at the end of cycle t + RX_TIMEOUT the parser drops the frame silently and hunts for `0xA5` again from cycle t + RX_TIMEOUT + 1. An oversize frame being dropped answers NACK_RANGE at that point. Note that g includes the next byte's own character time (10 bit times: 640 cycles at 1,562,500 baud; 8,681 at the KG2 fallback of 115200), so the true line-idle tolerance is `RX_TIMEOUT` minus one character time. This is what `rtl/packet.sv` implements: `to_cnt` clears on each strobe and the frame is reset when `to_cnt == RX_TIMEOUT-1` with no strobe in that cycle. Rationale for the value: the gaps it must tolerate are host and USB-UART scheduling gaps between USB packets. Those are wall-clock effects that do not depend on the baud, so the value is set in time (10 ms), not in characters, and is still invisible to a human. It is about 1,562 character times at 1,562,500 baud (about 115 at 115200), and it was not changed when the baud was raised (D-020). Codegen enforces ≥ 2 byte times (1,280 cycles at 1,562,500). Testbenches may override the `RX_TIMEOUT_CYC` module parameter (default = the package value) to keep truncated-frame tests short.
- **Error precedence:** bad CRC → NACK_CRC (payload ignored); unknown opcode → NACK_OPCODE; then BUSY; then payload-length or range errors → NACK_RANGE. Error responses have `len = 0`. A frame with `len > MAX_PAYLOAD` is not buffered: the parser drops it, waits for `RX_TIMEOUT` of idle, and sends NACK_RANGE.
- **BUSY:** while a RUN_GEMM job or self-test runs, `RUN_GEMM`, `SELFTEST`, `WRITE_MEM`, and `READ_MEM` get BUSY. `PING`, `POLL`, `READ_COUNTERS`, and `SET_DISPLAY` are always served.
- **PING** (request len 0): response `"SPLT"` (bytes 0x53 0x50 0x4C 0x54, `PING_MAGIC`), `PROTO_VERSION` u8 (= 1), `build_id` u32; len 9.
- **WRITE_MEM:** payload `region u8, byte_addr u32, data[len-5]`, with `1 ≤ len-5`, so at most 1019 data bytes per frame. NACK_RANGE if `region > 3` or `byte_addr + (len-5) > region bytes`.
- **READ_MEM:** payload `region u8, byte_addr u32, rlen u16` (len 7). NACK_RANGE if `region > 3`, `rlen = 0`, `rlen > MAX_PAYLOAD`, or `byte_addr + rlen > region bytes`. Response payload is `rlen` bytes.
- **RUN_GEMM:** see §3.5 validation. ACK (len 0) means accepted and started.
- **POLL** (len 0): response 1 byte: 0 IDLE (after reset), 1 BUSY, 2 DONE (last job finished; stays until the next RUN_GEMM/SELFTEST), 3 ERROR (reserved; never produced by the MVP).
- **READ_COUNTERS** (len 0): response 72 bytes, order of §3.7.
- **SELFTEST** (len 0): the response is sent when both self-test GEMMs finish (well under 1 ms): `pass_full u8, pass_strip u8, fletcher_full u32, fletcher_strip u32` (len 10; pass = 1 if the computed checksum equals the ROM's stored value, else 0; the checksums are the computed ones). Afterwards the counters hold the STRIP self-test run.
- **SET_DISPLAY:** payload `util_permille u16, mode u8` (len 3). NACK_RANGE if `util_permille > 1000` or `mode > 1`.
- Any opcode whose payload length differs from the one stated here gets NACK_RANGE.

### 3.9 Board I/O and self-test (`rtl/board_io.sv`, `rtl/selftest.sv`, `rtl/top.sv`)

- **Self-test:** BTN0 (or the SELFTEST opcode) runs one FULL and one STRIP GEMM from a ROM image generated by `sw/crates/vec` (`$readmemh` at synthesis). It compares the Fletcher-32 checksum of the output buffer against the stored value.
- **Fletcher-32 definition (D-001):** input bytes are grouped into 16-bit **little-endian** words, `word[i] = byte[2i] | byte[2i+1] << 8`; an odd trailing byte is zero-padded. `sum1 = sum2 = 0` initially; per word, `sum1 = (sum1 + word) mod 65535`, then `sum2 = (sum2 + sum1) mod 65535`. Both sums stay in 0..65534 (a word 0xFFFF counts as 0; end-around-carry hardware must map 0xFFFF to 0). Result = `(sum2 << 16) | sum1`. Check values: `"abcde"` → 0xF04FC729, `"abcdef"` → 0x56502D2A, `"abcdefgh"` → 0xEBE19591 (`FLETCHER32_CHECK_*`).
- **Bytes summed:** the C extent of that run, i.e. OBUF words `out_base .. out_base + NB*M_pad - 1` (`NB = ceil(N/16)`, `M_pad = 8*ceil(M/8)`) in increasing word order, each word as its 16-byte little-endian image (lane 0 first). Because out-of-bounds outputs are unspecified (§1.3), **self-test descriptors must use `M mod 8 = 0`, `N mod 16 = 0`, and flags = int8**, so every summed byte is a defined output. The sum length is a multiple of 16 bytes, so the odd-length rule never applies in hardware.
- **LEDs:** LED0 FULL pass, LED1 STRIP pass, LED2 busy, LED3 UART activity, LED15 heartbeat (1 Hz).
- **7-segment display:** shows the mode letter and utilization to one decimal place, for example `F  9.3` or `S 21.4`, from the last run or SET_DISPLAY.
- **BTN1:** synchronous reset.
- All pin assignments come from the official Urbana master XDC.

---

## 4. Timing model (the `cycle` crate implements this; RTL must agree exactly)

All `L_*` constants live in spec.toml and are set to the RTL's real values by `rtl-designer`, reviewed by `perf-modeler`.

```
T_tile(mode, K) = L_SETUP + (L_RD + K) + ((R-1) + (C-1) + L_PE) + T_DRAIN(mode, flags) + L_RQ + L_WB
  R = 8, C = 8 in both modes (physical array; the STRIP lower-half delay is absorbed, see §3.2)
  T_DRAIN = 8 (FULL int8), 4 (STRIP int8), 16 (FULL int32 = 8 rows x 2), 16 (STRIP int32 = 4 rows x 4)
tiles(FULL)  = ceil(M/8)  * ceil(N/8)
tiles(STRIP) = ceil(M/4)  * ceil(N/16)
cycles_total = L_ACCEPT + tiles * T_tile + cycles_stall + L_DONE      (cycles_stall = 0 in MVP)
```

**Each term lands in exactly one counter (D-011, normative; RTL and `cycle` implement the same split):**

| Counter | Model value |
| --- | --- |
| `cycles_setup` | `tiles * L_SETUP` |
| `cycles_stream` | `tiles * (L_RD + K)` |
| `cycles_flush` | `tiles * ((R-1) + (C-1) + L_PE)` |
| `cycles_drain` | `tiles * (T_DRAIN(mode, flags) + L_RQ + L_WB)` (DRAIN + WRITEBACK states) |
| `cycles_stall` | 0 (MVP) |
| `cycles_total` | `L_ACCEPT + setup + stream + flush + drain + stall + L_DONE` |
| `tiles` | `tiles(mode)` |
| `mac_useful` | `M*N*K` |
| `mac_useful_q0..q3` | in-bounds MACs per quadrant from the tile mapping (§3.2) |

`L_ACCEPT` and `L_DONE` appear only in `cycles_total`.

**Utilization ceilings (from the mapping, before overheads):**

| Shape | FULL | STRIP | Operand bytes/cycle (FULL / STRIP) |
| --- | --- | --- | --- |
| M = 1 | ≤ 12.5% | ≤ 25% | 16 / 20 |
| M = 4 | ≤ 50% | ≤ 100% | 16 / 20 |
| M ≥ 8 | → 100% minus fill, flush, drain | same | 16 / 20 |

The model must predict **every counter**, not just the ratio. Calibrate any unknown constant once against hardware, then **freeze the model** (commit hash recorded) before taking benchmark numbers.

---

## 5. Resource budget (check after the first synthesis)

| Resource | Budget | Notes |
| --- | --- | --- |
| DSP48E1 | ≤ 96 of 120 | 64 PE + ≤ 32 requant |
| BRAM36 | ≤ 40 of 75 | WBUF 16, ABUF 4, OBUF 4, ROM ≤ 4, margin |
| LUT | < 50% | skew, muxes, popcount, UART |
| FF | < 30% | |
| Fmax | ≥ 100 MHz (WNS ≥ 0) | otherwise 75 MHz through an MMCME2_BASE primitive, no IP generator |

A second build with the STRIP muxes compiled out (`ENABLE_STRIP = 0`) gives the **cost of splitting**: ΔLUT, ΔFF, ΔFmax. This number goes on the dashboard.

---

## 6. Verification plan and sign-off

**Levels:** (1) unit, cocotb + Verilator; (2) array integration, no UART; (3) system sim, with real host packets through a simulated serial pipe; (4) hardware, the same vectors over UART plus the self-test.

| Module | Reference | Directed tests | Random tests | Sign-off |
| --- | --- | --- | --- | --- |
| PE | golden MAC | (−128)(−128), (−128)(127), zeros, `first` mid-stream, back-to-back clears, vld gaps | 10k streams | bit-exact; latency = L_PE |
| Skew | index shift | lane delays incl. base 4; round trip = identity | random | exact |
| Array FULL | golden | identity W, all-ones, all −128, **single nonzero element per position** | 500 tiles, K 1..256 | bit-exact |
| Array STRIP | golden | single nonzero in each quadrant (no cross-quadrant leakage); lower-half alignment | 500 tiles | bit-exact |
| Mode switching | golden | FULL→STRIP→FULL with no reset | random sequences | bit-exact |
| Buffers | memory image | simultaneous host write and array read; first and last address; byte enables | random | exact; latency = L_RD |
| Sequencer | golden + cycle | edge tiles (M, N not multiples), K = 1, max sizes, both modes | random descriptors | outputs bit-exact; counters == model |
| Requant | golden (i64) | shift 1 and 31; tie at exactly 0.5; ± saturation; mult 1 and 32767; bypass | 100k | bit-exact |
| UART + packet | host lib | loopback, bad CRC → NACK, truncated frame → timeout recovery, back-to-back frames | random payloads ≤ 1024 | 0 errors in 10k frames |
| Counters | cycle model | idle job, single tile, invariants of §3.7 | random descriptors | exact equality |
| Self-test | stored checksum | BTN0 on hardware | — | LED0 and LED1 on |

**Manual coverage checklist:** both modes, edge tiles in M/N/K, saturation both ways, a rounding tie, a mode switch, int32 bypass, the NACK paths.

---

## 7. Measurement and honesty rules

- Record every run as JSON in `results/` (schema in `docs/DEMO.md` §4) with `source`, `build_id`, `git_hash`, `clock_mhz`, and the model prediction next to the measurement.
- **Separate on-chip cycles from end-to-end time.** UART dominates wall-clock time; never present wall time as accelerator speed.
- **Do not claim speedups over CPUs or GPUs.** The claim is about utilization, bandwidth per MAC, and model fidelity.
- Report the model error per counter. Anything over 5% gets an explanation, not a tweak.

---

## 8. Known pitfalls → required guards

| Pitfall | Guard |
| --- | --- |
| Signed/unsigned mixing | `signed` at ports; directed (−128)(−128) test |
| Off-by-one skew | single parameterized skew module; single-nonzero tests |
| `first` misaligned → stale sums | flags travel in the same registers as data; back-to-back tile test |
| DSP or BRAM latency not counted | `L_PE` and `L_RD` from spec.toml used by RTL and model alike |
| Failed DSP or BRAM inference | check the utilization report after every build (64+ DSP, expected BRAM) |
| Rounding mismatch | requant contract written first; tie-case test |
| STRIP lower-half misalignment | W lanes 8–15 delayed `4 + c`; quadrant-isolation test |
| Long Vivado builds | simulation first; batch changes; build during meals |
| Timing failure | register DSP stages and mode muxes; drop to 75 MHz rather than debug at 4 a.m. |
| USB contention (WSL vs Windows) | one strategy recorded in ENV.md |
| Scope creep | no stretch work before G4 (see PLAN) |
| Overclaiming | §7 rules; reviewer audits every public number |

---

## 9. Out of scope

Training, floating point, DDR3 and MIG, softmax or norms in hardware, INDEP mode before G4, any speed claim against a CPU or GPU.
