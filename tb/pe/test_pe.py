"""cocotb unit tests for rtl/pe.sv (SPEC §3.1, §6 row "PE").

Expected accumulator values: splitsys_golden::gemm_i32 (1xK . Kx1) via tb/oracle.
Timing contract (pe.sv header): MAC presented in cycle t is visible on acc_out
from cycle t + L_PE; a_out/w_out/a_vld_out/a_first_out are the inputs delayed 1.
"""

from __future__ import annotations

import cocotb

from splitsys_tb import (FallingEdge, RisingEdge, check, gemm_req, oracle, pkg_const,
                         rand_i8, reset, scaled, seed_rng, start_clock, sval, uval)

PREFIX_ALL = 16      # check every prefix sum of a segment up to this length
CHECKPOINT = 16      # beyond that, check every 16th prefix and the final value


async def setup(dut):
    dut.a_in.value = 0
    dut.w_in.value = 0
    dut.a_vld_in.value = 0
    dut.a_first_in.value = 0
    await start_clock(dut)
    await reset(dut)


async def drive(dut, stim):
    """stim: list of (a, w, vld, first) per cycle. Returns per-cycle samples."""
    samples = []
    for (a, w, v, f) in stim:
        await RisingEdge(dut.clk)
        dut.a_in.value = a & 0xFF
        dut.w_in.value = w & 0xFF
        dut.a_vld_in.value = v
        dut.a_first_in.value = f
        await FallingEdge(dut.clk)
        samples.append((sval(dut.acc_out), sval(dut.a_out), sval(dut.w_out),
                        uval(dut.a_vld_out), uval(dut.a_first_out)))
    return samples


def expected_timeline(stim, lat):
    """Per cycle: (expected acc or None if unchecked, landed?) from golden prefix sums.

    Segments start at reset or at a valid MAC with first=1. Expected values are
    golden::gemm_i32 of the segment prefix; this function only does bookkeeping.
    """
    segs = []           # list of (list of (a,w)), list of input cycles
    for t, (a, w, v, f) in enumerate(stim):
        if not v:
            continue
        if f or not segs:
            segs.append(([], []))
        segs[-1][0].append((a, w))
        segs[-1][1].append(t)
    reqs, keys = [], []
    for si, (ops, _cyc) in enumerate(segs):
        n = len(ops)
        for j in range(n):
            if j < PREFIX_ALL or (j + 1) % CHECKPOINT == 0 or j == n - 1:
                aa = [o[0] for o in ops[: j + 1]]
                ww = [o[1] for o in ops[: j + 1]]
                reqs.append(gemm_req(1, j + 1, 1, aa, ww))
                keys.append((si, j))
    vals = {k: int(v) for k, v in zip(keys, oracle(reqs))}
    land = {}           # cycle -> (seg, idx, input cycle)
    for si, (_ops, cyc) in enumerate(segs):
        for j, t in enumerate(cyc):
            land[t + lat] = (si, j, t)
    return vals, land


def check_run(dut, stim, samples, lat, label, acc0):
    vals, land = expected_timeline(stim, lat)
    acc_exp = acc0       # value held from the previous (already checked) run
    known = True
    prev_in = (0, 0, 0, 0)
    for c, (acc, a_o, w_o, v_o, f_o) in enumerate(samples):
        # 1-stage forwarding (identity)
        pa, pw, pv, pf = prev_in
        check(a_o == pa and w_o == pw and v_o == pv and (f_o == pf),
              f"[{label}] forward mismatch", cycle=c,
              expected=(pa, pw, pv, pf), got=(a_o, w_o, v_o, f_o))
        prev_in = (stim[c][0], stim[c][1], stim[c][2], stim[c][3])
        if c in land:
            si, j, t = land[c]
            if (si, j) in vals:
                acc_exp, known = vals[(si, j)], True
            else:
                known = False
        if known:
            if acc != acc_exp:
                si, j, t = land.get(c, (None, None, None))
                ctx_t = t
                # find the latest landed MAC for the hint
                lc = max((x for x in land if x <= c), default=None)
                if lc is not None:
                    si, j, ctx_t = land[lc]
                seg_inputs = [stim[x] for x in
                              range(max(0, (ctx_t or 0) - 3), (ctx_t or 0) + 1)]
                check(False, f"[{label}] acc_out mismatch", cycle=c, expected=acc_exp, got=acc,
                      last_mac_input_cycle=ctx_t, segment=si, prefix_len=(j or 0) + 1,
                      recent_inputs_a_w_vld_first=seg_inputs)
    # hold rule: acc changes only in landing cycles
    accs = [acc0] + [x[0] for x in samples]
    for c in range(len(samples)):
        if c not in land:
            check(accs[c + 1] == accs[c], f"[{label}] acc changed without a MAC",
                  cycle=c, before=accs[c], after=accs[c + 1])


def idle(n):
    return [(0, 0, 0, 0)] * n


async def run_and_check(dut, stim, label):
    lat = pkg_const("L_PE")
    stim = stim + idle(lat + 2)
    acc0 = sval(dut.acc_out)
    samples = await drive(dut, stim)
    check_run(dut, stim, samples, lat, label, acc0)


@cocotb.test()
async def test_latency_equals_L_PE(dut):
    """A single isolated MAC appears on acc_out exactly L_PE cycles after entry."""
    await setup(dut)
    lat = pkg_const("L_PE")
    stim = idle(4) + [(3, -5, 1, 1)] + idle(10)
    samples = await drive(dut, stim)
    t_in = 4
    changed = [c for c in range(1, len(samples)) if samples[c][0] != samples[c - 1][0]]
    exp = int(oracle([gemm_req(1, 1, 1, [3], [-5])])[0])
    check(len(changed) == 1, "expected exactly one acc change", changes=changed)
    measured = changed[0] - t_in
    dut._log.info(f"measured PE latency = {measured}, splitsys_pkg::L_PE = {lat}")
    check(measured == lat, "PE latency != splitsys_pkg::L_PE", measured=measured, L_PE=lat)
    check(samples[changed[0]][0] == exp, "acc value", expected=exp, got=samples[changed[0]][0])
    # back-to-back: a MAC every cycle lands every cycle (fully pipelined)
    stim2 = [(1, 1, 1, 1)] + [(1, 1, 1, 0)] * 9
    await run_and_check(dut, stim2, "pipelined")


@cocotb.test()
async def test_directed_extremes(dut):
    """(-128)(-128), (-128)(127), (127)(-128), (127)(127), K=256 worst case both signs."""
    await setup(dut)
    for (a, w) in [(-128, -128), (-128, 127), (127, -128), (127, 127), (-1, -1), (-128, 1)]:
        await run_and_check(dut, [(a, w, 1, 1)], f"single {a}*{w}")
    await run_and_check(dut, [(-128, -128, 1, 1)] + [(-128, -128, 1, 0)] * 255, "K256 (-128)(-128)")
    await run_and_check(dut, [(-128, 127, 1, 1)] + [(-128, 127, 1, 0)] * 255, "K256 (-128)(127)")
    await run_and_check(dut, [(127, -128, 1, 1)] + [(127, -128, 1, 0)] * 255, "K256 (127)(-128)")


@cocotb.test()
async def test_directed_zeros(dut):
    await setup(dut)
    await run_and_check(dut, [(0, 0, 1, 1)] + [(0, 0, 1, 0)] * 7, "all zeros")
    await run_and_check(dut, [(0, 99, 1, 1), (-128, 0, 1, 0), (0, -128, 1, 0)], "zero operand")
    # a nonzero accumulator cleared by a zero product with first
    await run_and_check(dut, [(100, 100, 1, 1), (0, 0, 1, 1), (0, 0, 1, 0)], "clear to zero")


@cocotb.test()
async def test_directed_first_mid_stream(dut):
    """`first` in the middle of an unbroken valid stream starts a new sum that cycle."""
    await setup(dut)
    s = [(5, 7, 1, 1), (-3, 2, 1, 0), (11, -13, 1, 0), (4, 4, 1, 1), (-128, -128, 1, 0), (1, 2, 1, 0)]
    await run_and_check(dut, s, "first mid-stream")


@cocotb.test()
async def test_directed_back_to_back_clears(dut):
    """`first` on every cycle: each acc equals just that product."""
    await setup(dut)
    rng = seed_rng(dut, "test_directed_back_to_back_clears")
    s = [(rand_i8(rng), rand_i8(rng), 1, 1) for _ in range(32)]
    await run_and_check(dut, s, "back-to-back clears")
    # alternating 2-long tiles back to back (stale-sum guard, SPEC §8)
    s = []
    for _ in range(16):
        s += [(rand_i8(rng), rand_i8(rng), 1, 1), (rand_i8(rng), rand_i8(rng), 1, 0)]
    await run_and_check(dut, s, "back-to-back 2-tiles")


@cocotb.test()
async def test_directed_vld_gaps(dut):
    """Bubbles (vld=0) carry garbage a/w/first and must not touch acc."""
    await setup(dut)
    s = [(5, 5, 1, 1), (127, 127, 0, 1), (-128, -128, 0, 0), (3, 3, 1, 0),
         (99, -99, 0, 1), (2, -2, 1, 0), (0, 0, 0, 0), (0, 0, 0, 0), (7, 7, 1, 0)]
    await run_and_check(dut, s, "vld gaps with garbage")


@cocotb.test()
async def test_reset_clears(dut):
    """Synchronous reset mid-accumulation clears acc and the forward registers."""
    await setup(dut)
    await drive(dut, [(50, 50, 1, 1), (50, 50, 1, 0), (50, 50, 1, 0)])
    dut.a_in.value = 0
    dut.w_in.value = 0
    dut.a_vld_in.value = 0
    dut.a_first_in.value = 0
    await reset(dut)
    await FallingEdge(dut.clk)
    check(sval(dut.acc_out) == 0, "acc after reset", got=sval(dut.acc_out))
    check(uval(dut.a_vld_out) == 0 and sval(dut.a_out) == 0 and sval(dut.w_out) == 0,
          "forward regs after reset", a=sval(dut.a_out), w=sval(dut.w_out))
    # no `first` after reset: accumulates onto 0
    await run_and_check(dut, [(3, 4, 1, 0), (5, 6, 1, 0)], "accumulate from reset")


@cocotb.test()
async def test_random_streams(dut):
    """10k random MAC streams (SCALE-scaled): random K, gaps, garbage on bubbles."""
    await setup(dut)
    rng = seed_rng(dut, "test_random_streams")
    n_streams = scaled(10000)
    stim = []
    for _ in range(n_streams):
        r = rng.random()
        k = rng.randint(1, 16) if r < 0.7 else rng.randint(17, 64) if r < 0.95 else rng.randint(65, 256)
        gap_p = rng.choice((0.0, 0.0, 0.1, 0.4))
        first_done = False
        while k:
            if rng.random() < gap_p:
                stim.append((rand_i8(rng), rand_i8(rng), 0, rng.randint(0, 1)))
                continue
            stim.append((rand_i8(rng), rand_i8(rng), 1, 0 if first_done else 1))
            first_done = True
            k -= 1
    dut._log.info(f"{n_streams} streams, {len(stim)} cycles")
    await run_and_check(dut, stim, "random")


@cocotb.test()
async def test_directed_first_without_vld(dut):
    """first=1 with vld=0 is a bubble (pe.sv: first matters only with a valid MAC):
    the next valid MAC without first must ACCUMULATE onto the held value."""
    await setup(dut)
    s = [(9, 9, 1, 1), (4, 4, 1, 0), (-128, -128, 0, 1), (100, -7, 0, 1), (2, 3, 1, 0),
         (0, 0, 0, 1), (5, -5, 1, 0)]
    await run_and_check(dut, s, "first with vld=0")
