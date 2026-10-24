# Unit sign-off record (owner: verification-engineer)

How to re-run any entry: `scripts/wsl 'make -C tb <module> RTL_REV=<rtl hash> SEED=<seed>'`.
By default `make -C tb ...` tests the **committed** `rtl/` at `HEAD` (exported with
`git archive` to `~/.cache/splitsys/rtl_rev/<hash>/rtl`), never the working tree.
`RTL_REV=worktree` tests uncommitted edits (pre-commit check only; not a sign-off).

Expected numerics come only from `splitsys_golden` through `tb/oracle` (a thin
line-protocol CLI; every answer is the return value of a public golden function:
`gemm_i32`, `requant`, `requant_is_tie`, `requant_unclamped`, `crc8`, `fletcher32`,
`packet::request_frame/response_frame`). Tests whose expectation is the input itself
(skew index shift, UART loopback bytes, buffer memory image) need no vectors.
Latencies are read from `rtl/gen/splitsys_pkg.sv` at test time and asserted equal
to the measured value.

| Module | RTL hash | tb hash | Command | Tests / seeds | Directed cases (SPEC §6) | Result |
| --- | --- | --- | --- | --- | --- | --- |
| pe | 36771b6 | see git log -- tb/pe | `make -C tb pe` (SEED=1, SCALE=1) | 8 tests; random: 10,000 streams / 283,038 cycles, SEED=1 | (−128)(−128), (−128)(127), (127)(−128), (127)(127) single and K=256; zeros; clear to zero; `first` mid-stream; back-to-back clears (K=1 ×32, K=2 ×16); vld gaps carrying garbage a/w/first; reset mid-accumulation; 1-stage a/w/vld/first forwarding every cycle; acc holds without MAC; **latency measured 3 == L_PE** | PASS 8/8, 25 s |
| skew | 36771b6 | see git log -- tb/skew | `make -C tb skew` (SEED=1) | 6 tests; random round trip 20,000 cycles × 16 lanes, SEED=1 | per-lane impulse delays: A (D=r), W top (D=c), W row-4 instance (D=4+c, as array.sv uses it), SPEC v1.1 §3.3 single 16-lane unit with W_SKEW_BASE_STRIP (lane 8+c → 4+c); FLAGS=0 ties vld/first to 0; no cross-lane leakage; reset clears all stages | PASS 6/6, 6 s. Sensitivity: at RTL 925307f (old `BASE[j]+j`) `test_spec_w16_single_unit` FAILS as expected |
