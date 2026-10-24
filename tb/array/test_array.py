"""cocotb tests for rtl/array.sv, FULL + STRIP, ENABLE_STRIP = 1 and 0
(SPEC v1.1 §3.2, §3.3, §6 rows "Array FULL", "Array STRIP", "Mode switching").

Expected tile results: splitsys_golden::gemm_i32 via tb/oracle
  FULL : C[8x8]  = A[8xK]  . W[Kx8]   drain_row r -> lane c = C[r,c], lanes 8..15 = 0
  STRIP: C[4x16] = A[4xK]  . W[Kx16]  drain_row i -> lane j = C[i,j] (j = 0..15)
Timing contract (array.sv header, SPEC §3.5 FLUSH, D-016), all from the package:
  word k at the port in cycle t0+k reaches PE(r,c) in t0+k+r+c; every acc is final
  in cycle t_d = t0+(K-1)+(ROWS-1)+(COLS-1)+L_PE; drain_data shows the row presented
  in cycle x exactly in cycle x+1 (the +1 drain-read register of L_RQ, D-016);
  mac_cnt / mac_cnt_q count the MAC entering PE(r,c) in cycle t in cycle t+2.
In-bounds masks only affect the counters; every drained element is compared.
"""

from __future__ import annotations

import cocotb

from splitsys_tb import (FallingEdge, RisingEdge, check, gemm_req, oracle, pkg_const, rand_i8,
                         reset, scaled, seed_rng, start_clock, uval)


def geom():
    return pkg_const("ROWS"), pkg_const("COLS"), pkg_const("QUAD"), pkg_const("L_PE")


def s32(v):
    v &= 0xFFFFFFFF
    return v - (1 << 32) if v & 0x80000000 else v


def tile(mode, k, a, w, row_inb=None, col_inb=None):
    """mode 'full': a 8xK, w Kx8 ; mode 'strip': a 4xK, w Kx16."""
    rows = 8 if mode == "full" else 4
    cols = 8 if mode == "full" else 16
    assert len(a) == rows and all(len(x) == k for x in a)
    assert len(w) == k and all(len(x) == cols for x in w)
    return {"mode": mode, "K": k, "A": a, "W": w,
            "row_inb": row_inb if row_inb is not None else (1 << rows) - 1,
            "col_inb": col_inb if col_inb is not None else (1 << cols) - 1}


def golden(tiles):
    reqs = []
    for t in tiles:
        rows, cols = len(t["A"]), len(t["W"][0])
        reqs.append(gemm_req(rows, t["K"], cols, [x for r in t["A"] for x in r],
                             [x for r in t["W"] for x in r]))
    out = []
    for t, line in zip(tiles, oracle(reqs)):
        vals = [int(x) for x in line.split()]
        cols = len(t["W"][0])
        out.append([vals[i * cols:(i + 1) * cols] for i in range(len(t["A"]))])
    return out


async def setup(dut):
    for s in ("mode_strip", "a_in", "a_vld", "a_first", "w_in", "row_inb", "col_inb", "drain_row"):
        getattr(dut, s).value = 0
    await start_clock(dut)
    await reset(dut)


def schedule(tiles, rng, es0_mode="full"):
    """Build per-cycle inputs and the expectations. Returns (cycles, drains, macs)."""
    R, C, H, lpe = geom()
    cyc = []
    drains = []          # (cycle the row is presented, tile idx, row)
    mac_exp = {}         # cycle -> [q0..q3] for the ES=1 instance
    mac_exp0 = {}        # cycle -> [q0..q3] for the ES=0 instance (always FULL mapping)
    cur = {"mode": 0, "row_inb": 0, "col_inb": 0}

    def idle():
        return {"mode": cur["mode"], "row_inb": cur["row_inb"], "col_inb": cur["col_inb"],
                "a": [rng.randint(0, 255) for _ in range(8)], "vld": 0,
                "first": rng.randint(0, 1), "w": [rng.randint(0, 255) for _ in range(16)],
                "drain": rng.randint(0, 7)}

    for _ in range(3):
        cyc.append(idle())
    for ti, t in enumerate(tiles):
        strip = t["mode"] == "strip"
        cur = {"mode": 1 if strip else 0, "row_inb": t["row_inb"], "col_inb": t["col_inb"]}
        for _ in range(2 + rng.randint(0, 2)):            # mode/masks >= 2 cycles before a_vld
            cyc.append(idle())
        t0 = len(cyc)
        k = t["K"]
        for kk in range(k):
            c = idle()
            c["vld"], c["first"] = 1, 1 if kk == 0 else 0
            if strip:
                c["a"] = [t["A"][i][kk] & 0xFF for i in range(4)] + [rng.randint(0, 255) for _ in range(4)]
                c["w"] = [t["W"][kk][j] & 0xFF for j in range(16)]
            else:
                c["a"] = [t["A"][r][kk] & 0xFF for r in range(8)]
                c["w"] = [t["W"][kk][j] & 0xFF for j in range(8)] + [rng.randint(0, 255) for _ in range(8)]
            cyc.append(c)
        # mac expectations: MAC entering PE(r,c) at t0+kk+r+c is counted at +2
        rin, cin = t["row_inb"], t["col_inb"]
        for r in range(R):
            for c_ in range(C):
                q = (r // H) * 2 + (c_ // H)
                if strip:
                    inb = ((rin >> (r % H)) & 1) & ((cin >> (c_ + (C if r >= H else 0))) & 1)
                else:
                    inb = ((rin >> r) & 1) & ((cin >> c_) & 1)
                inb0 = ((rin >> r) & 1) & ((cin >> c_) & 1)      # ES=0: FULL mapping
                for kk in range(k):
                    x = t0 + kk + r + c_ + 2
                    if inb:
                        mac_exp.setdefault(x, [0, 0, 0, 0])[q] += 1
                    if inb0:
                        mac_exp0.setdefault(x, [0, 0, 0, 0])[q] += 1
        t_d = t0 + (k - 1) + (R - 1) + (C - 1) + lpe
        while len(cyc) < t_d:
            cyc.append(idle())
        nrows = 4 if strip else 8
        for row in range(nrows):
            c = idle()
            c["drain"] = row
            drains.append((len(cyc), ti, row))
            cyc.append(c)
        # hold mode/masks through the cycle in which the last drain_data is registered
    for _ in range(4):
        cyc.append(idle())
    return cyc, drains, mac_exp, mac_exp0


async def run_tiles(dut, tiles, rng, label, check_es1=True, check_es0=None, es0_full_ref=None):
    """Drive the tiles back to back; compare drains and per-cycle MAC counts.
    check_es0: compare the ES=0 instance too (FULL tiles, or tiles driven with
    mode_strip=1 where ES=0 must still behave FULL: pass es0_full_ref tiles)."""
    if check_es0 is None:
        check_es0 = all(t["mode"] == "full" for t in tiles)
    gold = golden(tiles)
    gold0 = golden(es0_full_ref) if es0_full_ref is not None else gold
    cyc, drains, mac_exp, mac_exp0 = schedule(tiles, rng)
    samples = []
    for c in cyc:
        await RisingEdge(dut.clk)
        dut.mode_strip.value = c["mode"]
        dut.row_inb.value = c["row_inb"]
        dut.col_inb.value = c["col_inb"]
        dut.a_in.value = sum(v << (8 * i) for i, v in enumerate(c["a"]))
        dut.a_vld.value = c["vld"]
        dut.a_first.value = c["first"]
        dut.w_in.value = sum(v << (8 * i) for i, v in enumerate(c["w"]))
        dut.drain_row.value = c["drain"]
        await FallingEdge(dut.clk)
        samples.append((uval(dut.es1_drain_data), uval(dut.es1_mac_cnt_q), uval(dut.es1_mac_cnt),
                        uval(dut.es0_drain_data), uval(dut.es0_mac_cnt_q), uval(dut.es0_mac_cnt)))
    R, C, H, lpe = geom()
    for (x, ti, row) in drains:
        t = tiles[ti]
        strip = t["mode"] == "strip"
        insts = []
        if check_es1:
            insts.append(("ES=1", 0, gold[ti], strip))
        if check_es0:
            insts.append(("ES=0", 3, gold0[ti], False))
        for name, si, g, st in insts:
            raw = samples[x + 1][si]                      # registered: valid exactly 1 cycle later
            lanes = [s32(raw >> (32 * j)) for j in range(16)]
            exp = list(g[row]) if st else list(g[row]) + [0] * 8
            if lanes != exp:
                bad = [j for j in range(16) if lanes[j] != exp[j]]
                j = bad[0]
                if st:
                    pe = (row, j) if j < 8 else (H + row, j - 8)
                else:
                    pe = (row, j)
                quad = (pe[0] // H) * 2 + (pe[1] // H) if j < (16 if st else 8) else None
                check(False, f"[{label} {name}] drain mismatch", tile=ti, mode=t["mode"], K=t["K"],
                      drain_row=row, presented_cycle=x, bad_lanes=bad, lane=j, physical_PE=pe,
                      quadrant=quad, expected=exp[j], got=lanes[j], expected_row=exp, got_row=lanes)
    for x in range(len(samples)):
        for name, on, exp_map, qi, ti_ in (("ES=1", check_es1, mac_exp, 1, 2),
                                           ("ES=0", check_es0, mac_exp0, 4, 5)):
            if not on:
                continue
            e = exp_map.get(x, [0, 0, 0, 0])
            gq = [(samples[x][qi] >> (5 * q)) & 0x1F for q in range(4)]
            check(gq == e and samples[x][ti_] == sum(e), f"[{label} {name}] mac_cnt",
                  cycle=x, expected_q=e, got_q=gq, expected_total=sum(e), got_total=samples[x][ti_])


def rand_tile(rng, mode, k=None, masks=False):
    k = k if k is not None else rng.randint(1, 256)
    rows, cols = (8, 8) if mode == "full" else (4, 16)
    a = [[rand_i8(rng) for _ in range(k)] for _ in range(rows)]
    w = [[rand_i8(rng) for _ in range(cols)] for _ in range(k)]
    ri = rng.getrandbits(rows) if masks and rng.random() < 0.5 else None
    ci = rng.getrandbits(cols) if masks and rng.random() < 0.5 else None
    return tile(mode, k, a, w, ri, ci)


def zeros(r, c):
    return [[0] * c for _ in range(r)]


@cocotb.test()
async def test_full_directed(dut):
    """FULL: identity W, all-ones, all -128 at K=256, single nonzero element per output
    position (64 tiles: only C[r,c] may be nonzero), single-row / single-column feeds."""
    await setup(dut)
    rng = seed_rng(dut, "test_full_directed")
    tiles = []
    eye = [[1 if kk == c else 0 for c in range(8)] for kk in range(8)]
    tiles.append(tile("full", 8, [[rand_i8(rng) for _ in range(8)] for _ in range(8)], eye))
    tiles.append(tile("full", 37, [[1] * 37 for _ in range(8)], [[1] * 8 for _ in range(37)]))
    tiles.append(tile("full", 256, [[-128] * 256 for _ in range(8)], [[-128] * 8 for _ in range(256)]))
    tiles.append(tile("full", 256, [[127] * 256 for _ in range(8)], [[-128] * 8 for _ in range(256)]))
    for r in range(8):
        for c in range(8):
            k = rng.randint(1, 5)
            k0 = rng.randrange(k)
            a, w = zeros(8, k), zeros(k, 8)
            a[r][k0] = rng.choice((-128, 127, -1, 3))
            w[k0][c] = rng.choice((-128, 127, 1, -7))
            tiles.append(tile("full", k, a, w))
    await run_tiles(dut, tiles, rng, "FULL directed")


@cocotb.test()
async def test_strip_quadrant_isolation(dut):
    """STRIP: single nonzero output per position of the 4x16 tile (64 tiles; covers every
    PE of all four quadrants incl. the lower half fed by W lanes 8..15), no cross-quadrant
    leakage, A lanes 4..7 carry garbage that must be ignored, lower-half alignment."""
    await setup(dut)
    rng = seed_rng(dut, "test_strip_quadrant_isolation")
    tiles = []
    for i in range(4):
        for j in range(16):
            k = rng.randint(1, 5)
            k0 = rng.randrange(k)
            a, w = zeros(4, k), zeros(k, 16)
            a[i][k0] = rng.choice((-128, 127, -1, 5))
            w[k0][j] = rng.choice((-128, 127, 1, -9))
            tiles.append(tile("strip", k, a, w))
    # dense tiles: misalignment of the lower half would mix k indices
    for _ in range(6):
        tiles.append(rand_tile(rng, "strip", rng.randint(1, 40)))
    tiles.append(tile("strip", 256, [[-128] * 256 for _ in range(4)], [[-128] * 16 for _ in range(256)]))
    await run_tiles(dut, tiles, rng, "STRIP directed")


@cocotb.test()
async def test_mode_switching(dut):
    """FULL -> STRIP -> FULL ... with no reset, random tiles, random in-bounds masks."""
    await setup(dut)
    rng = seed_rng(dut, "test_mode_switching")
    seq = ["full", "strip", "full", "strip", "strip", "full", "full", "strip"]
    seq += [rng.choice(["full", "strip"]) for _ in range(scaled(24))]
    tiles = [rand_tile(rng, m, rng.randint(1, 64), masks=True) for m in seq]
    await run_tiles(dut, tiles, rng, "mode switch")


@cocotb.test()
async def test_random_full(dut):
    """scaled(500) random FULL tiles, K 1..256, random masks; ES=1 and ES=0 both checked."""
    await setup(dut)
    rng = seed_rng(dut, "test_random_full")
    n = scaled(500)
    for b in range(0, n, 50):
        tiles = [rand_tile(rng, "full", masks=True) for _ in range(min(50, n - b))]
        await run_tiles(dut, tiles, rng, f"random FULL batch {b // 50}")
    dut._log.info(f"{n} random FULL tiles bit-exact (ES=1 and ES=0)")


@cocotb.test()
async def test_random_strip(dut):
    """scaled(500) random STRIP tiles, K 1..256, random masks (ES=1)."""
    await setup(dut)
    rng = seed_rng(dut, "test_random_strip")
    n = scaled(500)
    for b in range(0, n, 50):
        tiles = [rand_tile(rng, "strip", masks=True) for _ in range(min(50, n - b))]
        await run_tiles(dut, tiles, rng, f"random STRIP batch {b // 50}")
    dut._log.info(f"{n} random STRIP tiles bit-exact")


@cocotb.test()
async def test_enable_strip0_ignores_mode(dut):
    """ENABLE_STRIP = 0: with mode_strip = 1 the array still computes the FULL tile
    (8x8, A lanes 0..7, W lanes 0..7) and counts FULL-mapping MACs."""
    await setup(dut)
    rng = seed_rng(dut, "test_enable_strip0_ignores_mode")
    full_tiles = [rand_tile(rng, "full", rng.randint(1, 40), masks=True) for _ in range(8)]
    # Drive them with mode_strip = 1: reuse the FULL data in a STRIP-shaped schedule would change
    # the drain count, so drive FULL tiles but force the mode bit high.
    forced = []
    for t in full_tiles:
        f = dict(t)
        forced.append(f)
    orig = schedule

    def schedule_forced(tiles, rng_):
        cyc, drains, mac_exp, mac_exp0 = orig(tiles, rng_)
        for c in cyc:
            c["mode"] = 1
        return cyc, drains, mac_exp, mac_exp0

    globals()["schedule"] = schedule_forced
    try:
        await run_tiles(dut, forced, rng, "ES=0 mode forced", check_es1=False, check_es0=True)
    finally:
        globals()["schedule"] = orig
