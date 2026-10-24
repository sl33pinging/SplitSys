# SplitSys Technical Specification (v1.0)

This document is the contract between RTL, the Rust models, the host, and the dashboard. `spec-steward` owns it. Values marked **[spec.toml]** must be defined in `spec/spec.toml` and consumed through codegen, never hard-coded twice.

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
- **Bypass:** with `flags.requant_en = 0`, the raw int32 accumulators are written (bring-up only; slower output path, see §3.6).

### 1.3 Data layouts (host packs, RTL streams; `golden` provides pack/unpack)

All multi-byte values are little-endian. Out-of-range rows and columns are **zero-padded by the host**, so the RTL never reads garbage.

- **A (activations)** in 8-row blocks: word64 index `mb*K + k` holds `A[8*mb + r, k]` in byte lane r, for r = 0..7. STRIP mode uses lanes 0–3 or 4–7 of the word (a 4-row half-block, selected by the sequencer).
- **W (weights)** in 16-column blocks: word128 index `nb*K + k` holds `W[k, 16*nb + c]` in byte lane c, for c = 0..15. FULL mode uses lanes 0–7 or 8–15 (an 8-column half-block); STRIP uses all 16.
- **C int8 output** in 16-column blocks: word128 index `nb*M_pad + m` holds `C[m, 16*nb + c]` in byte lane c. FULL writes 8 lanes with byte enables. `M_pad = ceil(M/8)*8`.
- **C int32 output (bypass)**: word128 index `(nb*M_pad + m)*4 + q` holds int32 lanes 4q..4q+3.
- **RQPARAM:** word32 index n = `{8'h0, shift[7:0], mult[15:0]}`.

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
- **Implementation:** infer one DSP48E1 (`(* use_dsp = "yes" *)`) with input, multiply, and P registers. The PE pipeline latency `L_PE` **[spec.toml]** is the cycles from operand entry to `acc` updated (expected 3).
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

One parameterized module: `LANES`, `WIDTH`, and per-lane extra delay as a parameter array. Lane j is delayed `BASE[j] + j` cycles, carrying data, vld, and first. The same module is used for A skew (base 0) and W skew (base 0 for lanes 0–7; base 4 for lanes 8–15, active only in STRIP). The FULL-mode lanes 8–15 are unused.

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

**FSM:** `IDLE → TILE_SETUP → STREAM → FLUSH → DRAIN → WRITEBACK → (next tile | DONE)`.
- **Tile order:** column-block outer, row-block inner. FULL tiles are 8×8; STRIP tiles are 4×16.
- **TILE_SETUP** loads the 16 (or 8) requant parameters for the tile's columns. It takes `L_SETUP` cycles **[spec.toml]**.
- **STREAM** issues K reads per buffer. The address generator selects the half-word lanes per mode and tile parity.
- **FLUSH** waits `(R-1)+(C-1)+L_PE` cycles for the last products to land.
- **DRAIN** reads results row by row: FULL reads 8 rows × 8 lanes (8 cycles), STRIP reads 4 rows × 16 lanes (4 cycles).
- **MVP:** stages do not overlap; the array idles during drain. A later target: shadow accumulators that overlap drain with the next tile.
- **Bounds:** the sequencer computes in-bounds masks from M and N for the counters (padding is zero, so the math is unaffected).

### 3.6 Drain and requantization (`rtl/requant.sv`)

- 16 lanes, each implementing §1.2 bit-exactly. A 32×15 multiply needs **two DSP48E1 per lane**, or use LUT multipliers if the DSP budget is tight. The DSP budget is 64 (PE) + up to 32 (requant) = 96 ≤ 120.
- Pipeline latency `L_RQ` **[spec.toml]**.
- **int8 path:** one 128-bit write per drained row (FULL uses byte enables for 8 lanes).
- **int32 bypass:** four 128-bit writes per drained row. This is slower, used only for bring-up, and its extra cycles are counted as `cycles_drain`.

### 3.7 Performance counters (`rtl/counters.sv`)

48-bit counters, cleared on `RUN_GEMM`, frozen at DONE, read with `READ_COUNTERS` in the order given in **[spec.toml]**:

| Counter | Increments when |
| --- | --- |
| `cycles_total` | every cycle from descriptor accept to DONE |
| `cycles_stream` | FSM in STREAM |
| `cycles_flush` | FSM in FLUSH |
| `cycles_drain` | FSM in DRAIN or WRITEBACK |
| `cycles_setup` | FSM in TILE_SETUP |
| `cycles_stall` | the array is waiting on buffer or output backpressure (0 in MVP unless a hazard exists) |
| `mac_useful` | += number of PEs performing an in-bounds MAC this cycle (popcount of 64 bits) |
| `mac_useful_q[0..3]` | the same, per quadrant |
| `tiles` | one per completed tile |

**Invariants checked on every run:**
- `mac_useful == M*N*K` exactly.
- `sum(mac_useful_q) == mac_useful`.
- `cycles_total == cycles_setup + cycles_stream + cycles_flush + cycles_drain + cycles_stall + L_ACCEPT + L_DONE` (constants in spec.toml).

**Utilization** = `mac_useful / (64 * cycles_total)`.

### 3.8 Host link (`rtl/uart_rx.sv`, `uart_tx.sv`, `packet.sv`)

- **UART:** 8N1, 16× oversampling, `BAUD` **[spec.toml]**. Start at 115200; raise only after the loopback test passes 10,000 packets with zero errors (try 921600, then higher if the bridge allows).
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

**Regions:** 0 WBUF, 1 ABUF, 2 OBUF, 3 RQPARAM. A timeout of `RX_TIMEOUT` cycles between bytes resets the frame parser.

`build_id` is a 32-bit generic set at synthesis from the short git hash. The host and dashboard display it.

### 3.9 Board I/O and self-test (`rtl/board_io.sv`, `rtl/selftest.sv`, `rtl/top.sv`)

- **Self-test:** BTN0 (or the SELFTEST opcode) runs one FULL and one STRIP GEMM from a ROM image generated by `sw/crates/vec` (`$readmemh` at synthesis). It compares the Fletcher-32 checksum of the output buffer against the stored value.
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
  T_DRAIN = 8 (FULL int8), 4 (STRIP int8), ×4 for int32 bypass
tiles(FULL)  = ceil(M/8)  * ceil(N/8)
tiles(STRIP) = ceil(M/4)  * ceil(N/16)
cycles_total = L_ACCEPT + tiles * T_tile + L_DONE
```

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
