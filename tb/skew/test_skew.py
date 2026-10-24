"""cocotb unit tests for rtl/skew.sv (SPEC v1.1 §3.2, §3.3, §6 row "Skew").

Expected values are the inputs themselves (index shift), so no golden vectors
are needed. Expected per-lane delays come from the generated package:
  A skew            D(r) = r                         (BASE 0)
  W skew, top edge  D(c) = c                         (BASE 0)
  W skew, row 4     D(c) = STRIP_W_SKEW_BASE + c     (lane 8+c, SPEC §3.2: 4 + c)
  SPEC §3.3 16-lane unit: D(j) = W_SKEW_BASE_STRIP[j] + (j mod COLS)
"""

from __future__ import annotations

import cocotb

from splitsys_tb import (FallingEdge, RisingEdge, check, pkg_const, reset, scaled, seed_rng,
                         start_clock, uval)


def configs():
    cols = pkg_const("COLS")
    base = pkg_const("STRIP_W_SKEW_BASE")
    wbs = pkg_const("W_SKEW_BASE_STRIP")
    return {
        # name: (lanes, input lane offset, flags carried, delays)
        "a": (8, 0, True, [r for r in range(8)]),
        "wt": (8, 0, False, [c for c in range(8)]),
        "wm": (8, 8, False, [base + c for c in range(8)]),
        "w16": (16, 0, True, [((wbs >> (8 * j)) & 0xFF) + (j % cols) for j in range(16)]),
    }


async def setup(dut):
    dut.in_data.value = 0
    dut.in_vld.value = 0
    dut.in_first.value = 0
    await start_clock(dut)
    await reset(dut)


def lanes_of(v: int, n: int, width: int) -> list[int]:
    return [(v >> (width * j)) & ((1 << width) - 1) for j in range(n)]


async def drive(dut, stim, names):
    """stim: list of (data[16], vld[16], first[16]). Samples every instance each cycle."""
    cfg = configs()
    samples = {n: [] for n in names}
    for (d, v, f) in stim:
        await RisingEdge(dut.clk)
        dut.in_data.value = sum((x & 0xFF) << (8 * j) for j, x in enumerate(d))
        dut.in_vld.value = sum(b << j for j, b in enumerate(v))
        dut.in_first.value = sum(b << j for j, b in enumerate(f))
        await FallingEdge(dut.clk)
        for n in names:
            lanes = cfg[n][0]
            samples[n].append((lanes_of(uval(getattr(dut, f"{n}_data")), lanes, 8),
                               lanes_of(uval(getattr(dut, f"{n}_vld")), lanes, 1),
                               lanes_of(uval(getattr(dut, f"{n}_first")), lanes, 1)))
    return samples


def check_shift(stim, samples, name):
    lanes, off, flags, delays = configs()[name]
    for c, (od, ov, of_) in enumerate(samples):
        for j in range(lanes):
            src = c - delays[j]
            ed, ev, ef = (stim[src][0][off + j], stim[src][1][off + j], stim[src][2][off + j]) \
                if src >= 0 else (0, 0, 0)
            if not flags:
                ev, ef = 0, 0
            check((od[j], ov[j], of_[j]) == (ed, ev, ef), f"[{name}] lane {j} mismatch",
                  cycle=c, lane=j, expected_delay=delays[j], expected_data_vld_first=(ed, ev, ef),
                  got=(od[j], ov[j], of_[j]), source_cycle=src)


async def lane_delays(dut, name):
    """Impulse on every lane of the instance at cycle 4 (value j+1, vld=first=1)."""
    lanes, off, flags, delays = configs()[name]
    t0 = 4
    stim = []
    for c in range(t0 + max(delays) + 6):
        d, v, f = [0] * 16, [0] * 16, [0] * 16
        if c == t0:
            for j in range(lanes):
                d[off + j] = j + 1
                v[off + j] = 1
                f[off + j] = 1
        stim.append((d, v, f))
    s = (await drive(dut, stim, [name]))[name]
    measured = []
    for j in range(lanes):
        hits = [c for c, (od, _, _) in enumerate(s) if od[j] != 0]
        check(len(hits) == 1 and s[hits[0]][0][j] == j + 1, f"[{name}] lane {j} impulse",
              cycles_nonzero=hits, values=[s[c][0][j] for c in hits])
        measured.append(hits[0] - t0)
    dut._log.info(f"[{name}] measured lane delays {measured}; expected {delays}")
    check(measured == delays, f"[{name}] lane delays", expected=delays, got=measured)
    check_shift(stim, s, name)


@cocotb.test()
async def test_lane_delays_a(dut):
    """A skew: lane r delayed exactly r (BASE 0), flags travel with data."""
    await setup(dut)
    await lane_delays(dut, "a")


@cocotb.test()
async def test_lane_delays_w_top(dut):
    """W skew top edge: lane c delayed c; FLAGS=0 ties out_vld/out_first to 0."""
    await setup(dut)
    await lane_delays(dut, "wt")


@cocotb.test()
async def test_lane_delays_w_row4_base4(dut):
    """W lanes 8..15 (row-4 entry, as array.sv instantiates it): lane 8+c delayed 4+c."""
    await setup(dut)
    await lane_delays(dut, "wm")


@cocotb.test()
async def test_spec_w16_single_unit(dut):
    """SPEC v1.1 §3.3 contract on one 16-lane instance with W_SKEW_BASE_STRIP:
    lane j delayed BASE[j] + (j mod COLS), so lane 8+c -> 4+c (D-008)."""
    await setup(dut)
    await lane_delays(dut, "w16")


@cocotb.test()
async def test_random_round_trip(dut):
    """Random data/vld/first on all lanes every cycle; every used instance's output,
    de-skewed by its lane delay, reproduces the input stream exactly (identity)."""
    await setup(dut)
    rng = seed_rng(dut, "test_random_round_trip")
    n = scaled(20000)
    stim = [([rng.randint(0, 255) for _ in range(16)], [rng.randint(0, 1) for _ in range(16)],
             [rng.randint(0, 1) for _ in range(16)]) for _ in range(n)]
    names = ["a", "wt", "wm"]
    samples = await drive(dut, stim, names)
    for name in names:
        check_shift(stim, samples[name], name)
    dut._log.info(f"{n} random cycles, instances {names}: identity after de-skew")


@cocotb.test()
async def test_reset_mid_stream(dut):
    """Synchronous reset clears every stage: outputs 0 (incl. vld) right after reset."""
    await setup(dut)
    rng = seed_rng(dut, "test_reset_mid_stream")
    stim = [([rng.randint(1, 255) for _ in range(16)], [1] * 16, [1] * 16) for _ in range(20)]
    await drive(dut, stim, ["a"])
    dut.in_data.value = 0
    dut.in_vld.value = 0
    dut.in_first.value = 0
    await reset(dut)
    await FallingEdge(dut.clk)
    for name in ("a", "wt", "wm"):
        for sig in ("data", "vld", "first"):
            check(uval(getattr(dut, f"{name}_{sig}")) == 0, f"[{name}] {sig} not cleared by reset",
                  got=hex(uval(getattr(dut, f"{name}_{sig}"))))
