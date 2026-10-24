# Open RTL / spec issues found by verification (owner: verification-engineer)

Each entry: failing command, minimal reproduction, expected vs got, classification, suspected location.
Status: OPEN | FIXED-pending-reverify | CLOSED (re-verified, hash).

## BUG-001 — requant latency 6 != splitsys_pkg::L_RQ 4  (OPEN, routes to rtl-designer + spec-steward/perf-modeler)
- Command: `scripts/wsl 'make -C tb requant COCOTB_TEST_FILTER=test_latency_equals_L_RQ'` (RTL d8d5eb1)
- Minimal repro: load params (1,1) on all lanes, present one beat (in_vld=1) in cycle t; `out_vld` rises in cycle t+6 (same for bypass).
- Expected: t + L_RQ = t + 4 (spec.toml `[latency.L_RQ] value = 4`, provisional). Got: t + 6.
- Classification: **spec-constant mismatch** (not a numerics bug). The RTL header documents LATENCY = 6 (E1..E6), the
  package still carries the provisional 4. Numerics are bit-exact: all 100,152 vec cases + 2,048 random back-to-back beats pass.
- Impact: SPEC §4 `WRITEBACK = L_RQ + L_WB` and every `cycles_drain`/`cycles_total` prediction of the cycle model are 2 cycles/tile off
  unless spec.toml L_RQ is set to the real pipeline (and note the array drain mux adds 1 more cycle, `L_DRAIN_MUX = 1`, between
  `drain_row` and requant input that §4 does not name).
- Fix options: (a) spec-steward sets `L_RQ = 6` (+ decide where the drain-mux cycle is counted), perf-modeler regenerates;
  or (b) rtl-designer shortens the pipeline to 4. The test asserts `measured == L_RQ`, whichever way it is resolved.
