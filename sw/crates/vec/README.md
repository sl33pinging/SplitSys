# splitsys-vec

Test-vector generator for SplitSys. All expected values come from `splitsys_golden`
(bit-exact reference). Testbenches never recompute expected values; they read these files.

```
cd sw
cargo run -p splitsys-vec -- selftest                       # out/selftest_rom.mem, out/selftest_expected.json
cargo run -p splitsys-vec -- directed                       # out/directed.json
cargo run -p splitsys-vec -- gemm --seed 1 --n 20 --mode full [--max-m 64 --max-k 256 --max-n 256]
cargo run -p splitsys-vec -- bench                          # out/bench/bench_suite.json
cargo run -p splitsys-vec -- unit pe|requant|skew|packet [--seed S --n N]
cargo run -p splitsys-vec -- all                            # everything, default seeds
# common option: --out DIR (default sw/crates/vec/out)
```

Same seed + same generator commit gives byte-identical files. Every file records the
seed, the generator's git hash, `git_dirty` (uncommitted changes in golden, vec, or spec),
the command line, and the spec version (`meta` in JSON, a `//` header in `.mem`).

**Committed** (canonical, small): `out/selftest_rom.mem`, `out/selftest_expected.json`,
`out/directed.json`. **Regenerated** (gitignored): `out/unit/`, `out/gemm/`, `out/bench/`.
Use `cargo run -p splitsys-vec -- all` to regenerate everything.

## Common conventions

- Hex strings are bytes in address order, lowercase (`bytes.fromhex(...)` in Python).
- Matrices are row-major nested arrays: `c[m][n]`, `acc[m][n]`.
- Memory layouts are SPEC §1.3; descriptor bytes are SPEC §3.5 (`desc_hex`, 24 bytes).

## Job JSON (gemm, directed, bench, selftest)

```
{ "name", "mode": "full"|"strip", "M","K","N", "requant": bool,
  "desc": {...fields...}, "desc_hex": "<24 bytes>",
  "rq": [[mult, shift], ...],                       // empty for int32 bypass
  "load": [ {"region", "region_name", "byte_addr", "len", "hex"}, ... ],   // WRITE_MEM list
  "expected": { "obuf_byte_addr", "obuf_len", "obuf_hex",
                "written_ranges": [[offset, len], ...],                  // relative to obuf_byte_addr
                "c": [[i8]] | null, "acc": [[i32]] } }
```

To check a job: perform every `load` (region codes per SPEC §3.8: 0 WBUF, 1 ABUF, 2 OBUF,
3 RQPARAM), run `desc_hex`, read OBUF from `obuf_byte_addr` for `obuf_len` bytes, and
compare **only the bytes in `written_ranges`** with `obuf_hex`. Bytes outside these ranges
are not written by the job in that mode (e.g. FULL with N = 8 leaves lanes 8..15 of each
row untouched; STRIP with M = 4 leaves rows 4..7 of the 8-row block untouched) and keep
their previous contents. Every written padding byte is 0.

The memory images and `obuf_hex` do not depend on the mode; only `desc.mode` and
`written_ranges` do (tested in `golden`: `mode_changes_schedule_not_layout_or_expected_c`).

**RQPARAM padding.** RQPARAM is written for `ceil(N/16)*16` columns. Padding columns
use `(mult, shift) = (1, 1)`, not zero: zero is out of range per SPEC §1.2. With
`acc = 0`, a padding column requantizes to exactly 0.

**Flags.** Jobs use `flags = 0x01` (requant, int8 output) or `flags = 0x02` (int32
bypass). The other combinations are not generated.

## directed.json

`jobs` covers SPEC §6. Each case runs in both modes unless noted:
- (−128)(−128), (−128)(127), and (127)(127) at K = 256, in int32 and requant
- zeros (K = 1 and 256)
- identity W, all ones (K = 37)
- `single_nz_r{i}_c{j}`: exactly one nonzero output at every (i, j). FULL covers 8×16
  (both half-blocks); STRIP covers 4×16.
- `quadrant_q{q}_only`: only one physical quadrant gets nonzero products. In STRIP,
  quadrant q holds C columns 4q..4q+3.
- `requant_ties`, `requant_saturation`
- `edge_MxKxN` (1×1×1 up to 64×256×256), in both modes and both output paths when the
  int32 output fits OBUF

`sequences` lists jobs to run back to back with no reset (FULL→STRIP→FULL).
`requant_cases` holds the named unit-level requant cases.

## Unit vectors (`out/unit/`)

| File | Contents |
| --- | --- |
| `pe.json` | Per-cycle `a, w, vld, first` at the PE inputs. `acc[t]` is the accumulator after cycle t (zero-latency model), so compare `acc_out` at t + `l_pe`. `groups[]` gives one accumulation each (`start`, `end`, `acc`, `name` for directed groups: ±128/127 at K = 256, zeros, back-to-back length-1 clears, `first` mid-stream, vld gaps with garbage operands). |
| `pe_stim.mem` / `pe_acc.mem` | The same data for `$readmemh`: `{first, vld, a[7:0], w[7:0]}` (5 hex digits) and `acc` (8 hex digits), one line per cycle. |
| `requant.json` / `requant.mem` | Named directed cases first (`n_directed`), then random ones. Fields: `acc, mult, shift, y, tie, sat`. The `.mem` line is `{acc[31:0], mult[15:0], shift[7:0], y[7:0]}`. |
| `skew_a.json`, `skew_w_full.json`, `skew_w_strip.json` | Per-lane `delays`. `out_*[lane][t] = in_*[lane][t − delay]`, and 0 before that. Arrays are indexed [lane][cycle]. The W STRIP delays are `W_SKEW_BASE_STRIP[j] + (j % COLS)`, so lanes 8–15 are delayed 4..11. |
| `packet.json` | CRC-8 known answers, good request and response frames, `bad_crc`, `truncated` (expect a timeout reset), a corrupted length byte (`nack_crc_or_timeout`), an unknown opcode, and a `back_to_back` byte stream. |

## Self-test ROM (`out/selftest_rom.mem`)

`$readmemh` file: `//` comment header, then one 32-bit word per line (8 hex digits).
ROM byte address `4i + b` maps to bits `[8b+7:8b]` of word i (little-endian).

| Word | Field |
| --- | --- |
| 0 | `MAGIC` = `0x53535431` |
| 1 | `VERSION` = 1 |
| 2 | `N_JOBS` = 2 (job 0 FULL, job 1 STRIP) |
| 3 | `N_LOADS` |
| 4 | `TOTAL_WORDS` (ROM length) |
| 5 | `JOB_TBL` = 8 (word offset of the job table) |
| 6 | `LOAD_TBL` (word offset of the load table) |
| 7 | reserved, 0 |
| `JOB_TBL + 8j + 0..5` | descriptor j (24 bytes, little-endian; descriptor byte `4w + b` = word w bits `[8b+7:8b]`, so `{w5,…,w0}` is the 192-bit descriptor) |
| `JOB_TBL + 8j + 6` | expected Fletcher-32 for job j |
| `JOB_TBL + 8j + 7` | `CHECK_WORDS`: the number of 128-bit OBUF words checked, starting at `desc.out_base` |
| `LOAD_TBL + 4l + 0` | region code (0 WBUF, 1 ABUF, 2 OBUF, 3 RQPARAM) |
| `LOAD_TBL + 4l + 1` | destination byte address in that region (a multiple of 4) |
| `LOAD_TBL + 4l + 2` | byte length (a multiple of 4) |
| `LOAD_TBL + 4l + 3` | source ROM word offset |
| after the tables | data words |

**Procedure.**
1. Do every load: ROM word `src + k` goes to region bytes `dst + 4k .. dst + 4k + 3`,
   little-endian. A loader can reuse the host WRITE_MEM path with 4-byte writes.
2. For each job in order: run its descriptor, then compute Fletcher-32 over OBUF bytes
   `[out_base*16, (out_base + CHECK_WORDS)*16)` in address order. The job passes if the
   result equals the stored value. The SELFTEST response returns `pass_full`,
   `pass_strip`, and the two computed checksums.

The current image has 464 words (1 BRAM36). It contains:
- **FULL job:** M = 8, K = 32, N = 16, all bases 0.
- **STRIP job:** M = 6, K = 32, N = 16, with a_base = 32, w_base = 32, out_base = 8,
  rq_base = 16. M = 6 means two 4-row STRIP tiles, one of them partial, with zero
  padding rows. Those tiles cover the whole M_pad = 8 block, so the job writes every
  byte it checks.

Each job is guaranteed positive saturation, negative saturation, at least one exact
rounding tie, and at least 25% mid-range outputs. `selftest_expected.json` lists
per-job statistics, the checksums, the full job JSON, and the expected OBUF bytes.

**Fletcher-32** (identical to `splitsys_spec::fletcher32` and `splitsys_golden::fletcher32`):
- Bytes are taken in address order as 16-bit little-endian words,
  `word = byte[2i] | byte[2i+1] << 8`. An odd final byte is zero-padded (never the case
  here, since lengths are multiples of 16).
- `sum1 = sum2 = 0`. For each word: `sum1 = (sum1 + word) mod 65535`, then
  `sum2 = (sum2 + sum1) mod 65535`.
- The result is `(sum2 << 16) | sum1`. Both sums stay in 0..65534; an end-around-carry
  implementation must map 0xFFFF to 0.

## Bench suite (`out/bench/bench_suite.json`)

Covers qkv, attn_out, ffn_up, and ffn_down at M ∈ {1, 2, 4, 8, 16, 32, 64}, from the fixed
seed `BENCH_SEED`. The data are synthetic but realistically scaled:
- weights: symmetric int8 per output column
- activations: symmetric int8 per tensor, with about 1% outliers
- requant parameters: calibrated per column on the M = 64 activations

Each entry has the FULL job plus `desc_strip_hex` and `written_ranges_strip`. Both modes
use the same images and the same expected bytes. Smaller M use the first M rows of the
M = 64 activation matrix.
