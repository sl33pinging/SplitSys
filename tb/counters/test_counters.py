"""cocotb unit tests for rtl/counters.sv (SPEC v1.1 §3.7, §4 D-011, §6 row "Counters").

Reference: the cycle model (sw/crates/cycle, `splitsys-cycle predict --timeline`).
The test turns the model's per-phase timeline into the sequencer taps a correct
sequencer would drive (one state tap per phase cycle, none in ACCEPT/DONE, a
tile_done pulse at the end of each WRITEBACK, the model's per-quadrant MAC totals
spread over the STREAM cycles), drives counters.sv with it, reads all 12 counters
through snap/rd_idx in spec.toml order, and requires exact equality with the
model's counter predictions, plus the §3.7 invariants on the values read back.
Contract details (counters.sv header): `start` clears, counting covers cycles
start+1 .. done inclusive, taps of cycle t land in the counter at t+2, snap is
atomic, rd_data is valid 1 cycle after rd_idx.
"""

from __future__ import annotations

import json
import os
import subprocess
import tempfile

import cocotb

from splitsys_tb import (FallingEdge, RisingEdge, check, pkg_const, reset, scaled, seed_rng,
                         start_clock, uval)

NAMES = ["cycles_total", "cycles_setup", "cycles_stream", "cycles_flush", "cycles_drain",
         "cycles_stall", "mac_useful", "mac_useful_q0", "mac_useful_q1", "mac_useful_q2",
         "mac_useful_q3", "tiles"]
PKG_IDX = ["CTR_CYCLES_TOTAL", "CTR_CYCLES_SETUP", "CTR_CYCLES_STREAM", "CTR_CYCLES_FLUSH",
           "CTR_CYCLES_DRAIN", "CTR_CYCLES_STALL", "CTR_MAC_USEFUL", "CTR_MAC_USEFUL_Q0",
           "CTR_MAC_USEFUL_Q1", "CTR_MAC_USEFUL_Q2", "CTR_MAC_USEFUL_Q3", "CTR_TILES"]
PHASE_TAP = {"TILE_SETUP": "setup", "STREAM": "stream", "FLUSH": "flush", "DRAIN": "drain",
             "WRITEBACK": "drain"}


def cycle_bin() -> str:
    b = os.environ.get("SPLITSYS_CYCLE_BIN")
    if not b or not os.path.exists(b):
        raise AssertionError(f"SPLITSYS_CYCLE_BIN not built: {b} (run through make -C tb counters)")
    return b


def model_predict(mode: str, m: int, k: int, n: int, flags: int) -> dict:
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump({"mode": mode, "M": m, "K": k, "N": n, "flags": flags}, f)
        path = f.name
    try:
        p = subprocess.run([cycle_bin(), "predict", path, "--timeline"], capture_output=True,
                           text=True, check=False)
    finally:
        os.unlink(path)
    if p.returncode != 0:
        raise AssertionError(f"splitsys-cycle predict failed: {p.stderr[-1000:]}")
    return json.loads(p.stdout)


def model_values(pred: dict) -> list[int]:
    md = pred["model"]
    q = md["mac_useful_q"]
    return [md["cycles_total"], md["cycles_setup"], md["cycles_stream"], md["cycles_flush"],
            md["cycles_drain"], md["cycles_stall"], md["mac_useful"], q[0], q[1], q[2], q[3],
            md["tiles"]]


def taps_from_timeline(pred: dict) -> list[dict]:
    """Per counted cycle (start+1 .. done): dict of taps. The last entry is the done cycle."""
    tl = pred["timeline"]
    total = pred["model"]["cycles_total"]
    cyc = [dict() for _ in range(total)]
    stream_cycles = []
    for ph in tl:
        tap = PHASE_TAP.get(ph["phase"])
        for i in range(ph["start"], ph["start"] + ph["len"]):
            check(i < total, "timeline beyond cycles_total", phase=ph)
            if tap:
                check(tap not in cyc[i] and not any(t in cyc[i] for t in PHASE_TAP.values()),
                      "two phases in one cycle", cycle=i)
                cyc[i][tap] = 1
            if ph["phase"] == "STREAM":
                stream_cycles.append(i)
        if ph["phase"] == "WRITEBACK":
            cyc[ph["start"] + ph["len"] - 1]["tile_done"] = 1
    # spread each quadrant's model total over the STREAM cycles (<= 16 per cycle)
    qtot = pred["model"]["mac_useful_q"]
    s = len(stream_cycles)
    for qi, t in enumerate(qtot):
        base, rem = divmod(t, s) if s else (0, 0)
        check(base + (1 if rem else 0) <= 16, "quadrant MACs exceed 16/cycle", q=qi, total=t, stream=s)
        for j, i in enumerate(stream_cycles):
            cyc[i][f"q{qi}"] = base + (1 if j < rem else 0)
    return cyc


async def setup(dut):
    for s in ("start", "done", "tap_setup", "tap_stream", "tap_flush", "tap_drain", "tap_stall",
              "mac_cnt", "mac_q_cnt", "tile_done", "snap", "rd_idx"):
        getattr(dut, s).value = 0
    await start_clock(dut)
    await reset(dut)


def apply(dut, t: dict):
    dut.tap_setup.value = t.get("setup", 0)
    dut.tap_stream.value = t.get("stream", 0)
    dut.tap_flush.value = t.get("flush", 0)
    dut.tap_drain.value = t.get("drain", 0)
    dut.tap_stall.value = t.get("stall", 0)
    q = [t.get(f"q{i}", 0) for i in range(4)]
    dut.mac_q_cnt.value = sum(v << (5 * i) for i, v in enumerate(q))
    dut.mac_cnt.value = t.get("mac", sum(q))
    dut.tile_done.value = t.get("tile_done", 0)


async def run_job(dut, taps: list[dict], start_taps: dict | None = None, check_running=True):
    """Cycle 0: start pulse (with optional taps that must NOT count); cycles 1..len: taps;
    done pulses in the last tap cycle. Returns after the done cycle."""
    await RisingEdge(dut.clk)
    dut.start.value = 1
    apply(dut, start_taps or {})
    for i, t in enumerate(taps):
        await RisingEdge(dut.clk)
        dut.start.value = 0
        apply(dut, t)
        dut.done.value = 1 if i == len(taps) - 1 else 0
        if check_running:
            await FallingEdge(dut.clk)
            check(uval(dut.running) == 1, "running low inside the job window", cycle=i + 1)
    await RisingEdge(dut.clk)
    dut.done.value = 0
    apply(dut, {})


async def read_all(dut, snap=True) -> list[int]:
    """snap (optional) then read indices 0..N_COUNTERS-1; rd_data valid 1 cycle after rd_idx."""
    if snap:
        await RisingEdge(dut.clk)
        dut.snap.value = 1
        await RisingEdge(dut.clk)
        dut.snap.value = 0
    vals = []
    for i in range(pkg_const("N_COUNTERS")):
        await RisingEdge(dut.clk)
        dut.rd_idx.value = i
        await RisingEdge(dut.clk)        # registered read: valid in the next cycle
        await FallingEdge(dut.clk)
        vals.append(uval(dut.rd_data))
    return vals


def check_vals(got, exp, label, **ctx):
    for i, (g, e) in enumerate(zip(got, exp)):
        check(g == e, f"[{label}] counter {NAMES[i]} (index {i})", expected=e, got=g, **ctx)


def check_invariants(v, m, k, n, label):
    la, ld = pkg_const("L_ACCEPT"), pkg_const("L_DONE")
    check(v[6] == m * n * k, f"[{label}] mac_useful == M*N*K", got=v[6], MNK=m * n * k)
    check(sum(v[7:11]) == v[6], f"[{label}] sum(mac_useful_q) == mac_useful", q=v[7:11], mac=v[6])
    check(v[0] == sum(v[1:6]) + la + ld, f"[{label}] cycles_total invariant",
          total=v[0], parts=v[1:6], L_ACCEPT=la, L_DONE=ld)


@cocotb.test()
async def test_index_order_matches_package(dut):
    """The test's name order is the spec.toml READ_COUNTERS order (CTR_* in the package)."""
    for i, nm in enumerate(PKG_IDX):
        check(pkg_const(nm) == i, "CTR index", name=nm, pkg=pkg_const(nm), expected=i)
    check(pkg_const("N_COUNTERS") == 12 and pkg_const("COUNTER_BITS") == 48, "counter geometry")


async def model_job(dut, mode, m, k, n, flags, label, start_garbage=True):
    pred = model_predict(mode, m, k, n, flags)
    taps = taps_from_timeline(pred)
    garbage = {"setup": 1, "stream": 1, "flush": 1, "drain": 1, "stall": 1, "q0": 16, "q1": 16,
               "q2": 16, "q3": 16, "tile_done": 1} if start_garbage else None
    await run_job(dut, taps, start_taps=garbage)
    await FallingEdge(dut.clk)
    check(uval(dut.running) == 0, f"[{label}] running after done")
    got = await read_all(dut)
    exp = model_values(pred)
    check_vals(got, exp, label, mode=mode, M=m, K=k, N=n, flags=flags)
    check_invariants(got, m, k, n, label)
    return got, pred


@cocotb.test()
async def test_single_tile_and_smallest_job(dut):
    """Model-driven: M=K=N=1 FULL; one full FULL tile (8x8, K=16); one STRIP tile (4x16)."""
    await setup(dut)
    await model_job(dut, "full", 1, 1, 1, 1, "1x1x1")
    await model_job(dut, "full", 8, 16, 8, 1, "FULL tile")
    await model_job(dut, "strip", 4, 16, 16, 1, "STRIP tile")
    await model_job(dut, "full", 8, 4, 8, 2, "FULL tile int32")


@cocotb.test()
async def test_idle_job_and_window(dut):
    """start then done in the next cycle with no taps: cycles_total = 1, rest 0.
    Taps in the start cycle and after done are not counted (window = start+1 .. done)."""
    await setup(dut)
    await run_job(dut, [{}], start_taps={"setup": 1, "q0": 3, "tile_done": 1})
    for _ in range(20):                              # garbage after done: frozen
        await RisingEdge(dut.clk)
        apply(dut, {"stream": 1, "q1": 5, "tile_done": 1})
    await RisingEdge(dut.clk)
    apply(dut, {})
    got = await read_all(dut)
    check_vals(got, [1] + [0] * 11, "idle job")


@cocotb.test()
async def test_freeze_clear_and_live_snapshot(dut):
    """Frozen after done (garbage taps ignored, re-snap identical); the next start clears
    (no accumulation across jobs); a snap while running returns the exact prefix of the
    taps registered so far (taps of cycle t counted at t+2)."""
    await setup(dut)
    rng = seed_rng(dut, "test_freeze_clear_and_live_snapshot")
    got1, _ = await model_job(dut, "full", 9, 3, 10, 1, "job A")
    for _ in range(30):
        await RisingEdge(dut.clk)
        apply(dut, {"stream": 1, "q2": 7, "tile_done": 1, "stall": 1})
    await RisingEdge(dut.clk)
    apply(dut, {})
    again = await read_all(dut)
    check_vals(again, got1, "frozen after done")
    await model_job(dut, "strip", 5, 7, 20, 1, "job B (cleared)")
    # live snapshot: random taps, snap at cycle s of the window
    taps = []
    for _ in range(200):
        q = [rng.randint(0, 16) for _ in range(4)]
        st = rng.choice(["setup", "stream", "flush", "drain", "stall", None])
        t = {f"q{i}": q[i] for i in range(4)}
        if st:
            t[st] = 1
        if rng.random() < 0.05:
            t["tile_done"] = 1
        taps.append(t)
    s_at = 120                                    # snap in window cycle s_at (1-based)
    await RisingEdge(dut.clk)
    dut.start.value = 1
    apply(dut, {})
    for i, t in enumerate(taps, start=1):
        await RisingEdge(dut.clk)
        dut.start.value = 0
        apply(dut, t)
        dut.snap.value = 1 if i == s_at else 0
        dut.done.value = 0
    await RisingEdge(dut.clk)
    dut.snap.value = 0
    apply(dut, {})
    live = await read_all(dut, snap=False)
    pre = taps[: s_at - 2]                         # window cycles 1 .. s_at-2
    exp = [s_at - 2,
           sum(t.get("setup", 0) for t in pre), sum(t.get("stream", 0) for t in pre),
           sum(t.get("flush", 0) for t in pre), sum(t.get("drain", 0) for t in pre),
           sum(t.get("stall", 0) for t in pre),
           sum(sum(t.get(f"q{i}", 0) for i in range(4)) for t in pre)] + \
          [sum(t.get(f"q{i}", 0) for t in pre) for i in range(4)] + \
          [sum(t.get("tile_done", 0) for t in pre)]
    check_vals(live, exp, "live snapshot prefix", snap_window_cycle=s_at)
    await RisingEdge(dut.clk)
    dut.done.value = 1
    await RisingEdge(dut.clk)
    dut.done.value = 0


@cocotb.test()
async def test_read_port(dut):
    """rd_data registered (valid 1 cycle after rd_idx); indices >= N_COUNTERS read 0."""
    await setup(dut)
    await model_job(dut, "full", 3, 2, 3, 1, "small")
    for idx in range(pkg_const("N_COUNTERS"), 1 << (pkg_const("N_COUNTERS") - 1).bit_length()):
        await RisingEdge(dut.clk)
        dut.rd_idx.value = idx
        await RisingEdge(dut.clk)
        await FallingEdge(dut.clk)
        check(uval(dut.rd_data) == 0, "out-of-range rd_idx", idx=idx, got=uval(dut.rd_data))
    # latency: switch index, the old value is still visible in that same cycle
    await RisingEdge(dut.clk)
    dut.rd_idx.value = 0
    await RisingEdge(dut.clk)
    await FallingEdge(dut.clk)
    v0 = uval(dut.rd_data)
    await RisingEdge(dut.clk)
    dut.rd_idx.value = 11
    await FallingEdge(dut.clk)
    check(uval(dut.rd_data) == v0, "rd_data changed in the same cycle as rd_idx (expected 1-cycle latency)")
    await RisingEdge(dut.clk)
    await FallingEdge(dut.clk)
    check(uval(dut.rd_data) != v0, "rd_data did not update 1 cycle after rd_idx", v0=v0)


@cocotb.test()
async def test_random_descriptors(dut):
    """Random legal descriptors (both modes, both flags, edge M/N/K) back to back without
    reset: every counter == splitsys-cycle prediction, §3.7 invariants hold."""
    await setup(dut)
    rng = seed_rng(dut, "test_random_descriptors")
    n_jobs = scaled(24)
    for j in range(n_jobs):
        mode = rng.choice(["full", "strip"])
        m = rng.choice([1, 2, 3, 4, 5, 7, 8, 9, rng.randint(1, 24)])
        k = rng.choice([1, 2, rng.randint(1, 40)])
        n = rng.choice([1, 8, 15, 16, 17, rng.randint(1, 40)])
        flags = rng.choice([1, 1, 2])
        await model_job(dut, mode, m, k, n, flags, f"job {j}", start_garbage=rng.random() < 0.5)
    dut._log.info(f"{n_jobs} random descriptors: counters == splitsys-cycle model")
