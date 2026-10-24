"""Level-2 verification (SPEC §6 "array integration, no UART"): rtl/core.sv (buffers +
sequencer + array + requant) with rtl/counters.sv wired as in top.sv (tb/core/core_tb.sv).

Drives the core's memory bus and run interface directly (no UART), so it is fast.
Expected values come only from Rust:
  * jobs: sw/crates/vec directed.json (committed), `vec gemm` random jobs, and custom
    jobs built by golden Job::new through tb/oracle `job` (loads, OBUF image, compare
    ranges); OBUF is compared on compare_ranges ONLY (D-006);
  * counters: `splitsys-cycle predict` for the job's (mode, M, K, N, flags); all 12
    counters must match exactly, plus the SPEC §3.7 invariants.
Before every job the in-bounds OBUF bytes are poisoned with the bitwise NOT of the
expected bytes, so a missing write can never pass on stale data.
"""

from __future__ import annotations

import json
import os

import cocotb
from cocotb.triggers import ClockCycles, First, RisingEdge as RE, Timer

from splitsys_tb import (COUNTER_NAMES, FallingEdge, RisingEdge, check, cycle_predict_cached,
                         cycle_predict_many,
                         model_counter_values, oracle, pkg_const, rand_i8, reset, scaled,
                         seed_rng, start_clock, uval)

REPO = os.environ.get("SPLITSYS_REPO_ROOT", "")
WB = {0: 16, 1: 8, 2: 16, 3: 4}           # region -> word bytes (checked against the package)
MODE = {0: "full", 1: "strip"}
STRIP_EN = int(os.environ.get("CORE_STRIP_EN", "1"))   # tb/core_ns builds core with STRIP_EN=0


def check_geometry():
    exp = {0: pkg_const("WBUF_WORD_BYTES"), 1: pkg_const("ABUF_WORD_BYTES"),
           2: pkg_const("OBUF_WORD_BYTES"), 3: pkg_const("RQ_WORD_BYTES")}
    check(exp == WB, "region word bytes", pkg=exp, tb=WB)


class Core:
    def __init__(self, dut):
        self.dut = dut
        self.lrd = pkg_const("L_RD")
        self.fails = []          # collected job failures (reported at the end of a test)
        self.jobs_run = 0
        self.skipped = 0

    async def init(self):
        d = self.dut
        for s in ("mem_region", "mem_addr", "mem_wdata", "mem_be", "mem_we", "mem_re",
                  "run_desc", "run_valid", "ctr_snap", "ctr_idx"):
            getattr(d, s).value = 0
        await start_clock(d)
        await reset(d)
        await ClockCycles(d.clk, 2)

    async def write(self, region: int, byte_addr: int, data: bytes):
        """Word writes with byte enables, one per cycle."""
        d, wb = self.dut, WB[region]
        words = {}
        for i, b in enumerate(data):
            a = byte_addr + i
            w, lane = divmod(a, wb)
            v, be = words.get(w, (0, 0))
            words[w] = (v | (b << (8 * lane)), be | (1 << lane))
        for w in sorted(words):
            v, be = words[w]
            await RisingEdge(d.clk)
            d.mem_region.value = region
            d.mem_addr.value = w
            d.mem_wdata.value = v
            d.mem_be.value = be
            d.mem_we.value = 1
        await RisingEdge(d.clk)
        d.mem_we.value = 0
        d.mem_be.value = 0

    async def read(self, region: int, byte_addr: int, n: int) -> bytes:
        """Pipelined word reads, one per cycle; data sampled exactly L_RD cycles later."""
        d, wb = self.dut, WB[region]
        w0, w1 = byte_addr // wb, (byte_addr + n - 1) // wb
        addrs = list(range(w0, w1 + 1))
        got = {}
        issue = {}
        c = 0
        total = len(addrs) + self.lrd + 1
        for c in range(total):
            await RisingEdge(d.clk)
            if c < len(addrs):
                d.mem_region.value = region
                d.mem_addr.value = addrs[c]
                d.mem_re.value = 1
                issue[c] = addrs[c]
            else:
                d.mem_re.value = 0
            await FallingEdge(d.clk)
            src = c - self.lrd
            if src in issue:
                got[issue[src]] = uval(d.mem_rdata) & ((1 << (8 * wb)) - 1)
        buf = b"".join(got[a].to_bytes(wb, "little") for a in addrs)
        off = byte_addr - w0 * wb
        return buf[off:off + n]

    async def run(self, desc: bytes, max_cycles: int = 400000) -> int:
        """run_valid held until run_accept; wait for DONE (seq_done rises); return cycles."""
        d = self.dut
        await RisingEdge(d.clk)
        d.run_desc.value = int.from_bytes(desc, "little")
        d.run_valid.value = 1
        n = 0
        while True:
            await FallingEdge(d.clk)
            n += 1
            check(n < 1000, "run_accept never asserted", dbg_state=uval(d.dbg_state))
            if uval(d.run_accept):
                break
        await RisingEdge(d.clk)          # taken at this edge
        d.run_valid.value = 0
        await FallingEdge(d.clk)
        check(uval(d.seq_busy) == 1 or uval(d.seq_done) == 1, "seq_busy not set after accept")
        if not uval(d.seq_done):
            # event-driven wait (Timer, not ClockCycles: no per-cycle Python while running)
            await First(RE(d.seq_done), Timer(max_cycles * 10, unit="ns"))
            check(uval(d.seq_done) == 1, "job did not finish", max_cycles=max_cycles,
                  dbg_state=uval(d.dbg_state))
        await ClockCycles(d.clk, 3)       # counters final 2 cycles after done
        check(uval(d.seq_busy) == 0, "seq_busy still high after DONE")
        check(uval(d.seq_error) == 0, "seq_error set")
        return n

    async def counters(self) -> list[int]:
        d = self.dut
        await RisingEdge(d.clk)
        d.ctr_snap.value = 1
        await RisingEdge(d.clk)
        d.ctr_snap.value = 0
        vals = []
        for i in range(len(COUNTER_NAMES)):
            await RisingEdge(d.clk)
            d.ctr_idx.value = i
            await RisingEdge(d.clk)
            await FallingEdge(d.clk)
            vals.append(uval(d.ctr_data))
        return vals

    async def job(self, j: dict, label: str | None = None):
        """j: vec job JSON dict (load, desc_hex, desc, M, K, N, expected)."""
        label = label or j["name"]
        if not STRIP_EN and j["desc"]["mode"] != 0:
            self.skipped += 1          # STRIP_EN=0 build: STRIP jobs are rejected upstream (packet)
            return
        exp = j["expected"]
        ob_addr = exp["obuf_byte_addr"]
        ob = bytes.fromhex(exp["obuf_hex"])
        ranges = exp["compare_ranges"]
        # poison in-bounds bytes with ~expected
        for off, ln in ranges:
            await self.write(2, ob_addr + off, bytes((~b) & 0xFF for b in ob[off:off + ln]))
        for ld in j["load"]:
            await self.write(ld["region"], ld["byte_addr"], bytes.fromhex(ld["hex"]))
        desc = bytes.fromhex(j["desc_hex"])
        await self.run(desc)
        got_c = await self.counters()
        lo = min(o for o, _ in ranges)
        hi = max(o + ln for o, ln in ranges)
        rb = await self.read(2, ob_addr + lo, hi - lo)
        self.jobs_run += 1
        # OBUF, in-bounds only
        bad = []
        for off, ln in ranges:
            for i in range(off, off + ln):
                if rb[i - lo] != ob[i]:
                    bad.append(i)
        dsc = j["desc"]
        mode, flags = MODE.get(dsc["mode"], str(dsc["mode"])), dsc["flags"]
        m, k, n = dsc["M"], dsc["K"], dsc["N"]
        if bad:
            i = bad[0]
            word = 16 if flags == 1 else 64
            self.fails.append(
                f"[{label}] OBUF mismatch: {len(bad)} bytes, first at obuf offset {i} "
                f"(byte_addr {ob_addr + i}, word {(ob_addr + i) // 16}, lane {(ob_addr + i) % 16}): "
                f"expected {ob[i]:#04x} got {rb[i - lo]:#04x}; mode={mode} flags={flags} M={m} K={k} N={n}; "
                f"first offsets {bad[:8]}")
        # counters vs cycle model, exact
        pred = cycle_predict_cached(mode, m, k, n, flags)
        want = model_counter_values(pred)
        diff = [(COUNTER_NAMES[i], want[i], got_c[i]) for i in range(len(want)) if want[i] != got_c[i]]
        if diff:
            self.fails.append(f"[{label}] counters != splitsys-cycle (mode={mode} flags={flags} "
                              f"M={m} K={k} N={n}): " +
                              ", ".join(f"{nm}: model {w} rtl {g} (diff {g - w:+d})" for nm, w, g in diff))
        la, ld_ = pkg_const("L_ACCEPT"), pkg_const("L_DONE")
        inv = []
        if got_c[6] != m * n * k:
            inv.append(f"mac_useful {got_c[6]} != M*N*K {m * n * k}")
        if sum(got_c[7:11]) != got_c[6]:
            inv.append(f"sum(q) {sum(got_c[7:11])} != mac_useful {got_c[6]}")
        if got_c[0] != sum(got_c[1:6]) + la + ld_:
            inv.append(f"cycles_total {got_c[0]} != parts {sum(got_c[1:6])} + L_ACCEPT + L_DONE")
        if inv:
            self.fails.append(f"[{label}] §3.7 invariants: " + "; ".join(inv))

    def prefetch(self, jobs):
        """Model predictions for all jobs up front, in parallel (CLI is ~1 s per call)."""
        cycle_predict_many([(MODE.get(j["desc"]["mode"], "?"), j["desc"]["M"], j["desc"]["K"],
                             j["desc"]["N"], j["desc"]["flags"]) for j in jobs
                            if STRIP_EN or j["desc"]["mode"] == 0])

    def report(self, dut, what):
        dut._log.info(f"{what}: {self.jobs_run} jobs, {len(self.fails)} failures"
                      f"{f', {self.skipped} STRIP jobs skipped (STRIP_EN=0 build)' if self.skipped else ''}")
        for f in self.fails[:20]:
            dut._log.error(f)
        check(not self.fails, f"{what}: {len(self.fails)} job failures", first=self.fails[:5])


def load_directed():
    with open(os.path.join(REPO, "sw", "crates", "vec", "out", "directed.json")) as f:
        return json.load(f)


def oracle_job(mode, a, w, rq=None, bases=(0, 0, 0, 0), name="custom"):
    """Build a job with golden Job::new (tb/oracle `job`); returns a vec-style dict."""
    m, k, n = len(a), len(a[0]), len(w[0])
    req = (f"job {mode} {m} {k} {n} {1 if rq else 0} {bases[0]} {bases[1]} {bases[2]} {bases[3]} "
           + " ".join(str(x) for r in a for x in r) + " " + " ".join(str(x) for r in w for x in r))
    if rq:
        req += " " + " ".join(f"{mu} {sh}" for mu, sh in rq)
    f = oracle([req])[0].split()
    desc_hex, a_addr, a_hex, w_addr, w_hex, rq_addr, rq_hex, o_addr, o_hex, rng_s = f
    loads = [{"region": 1, "byte_addr": int(a_addr), "hex": a_hex},
             {"region": 0, "byte_addr": int(w_addr), "hex": w_hex}]
    if rq_hex != "-":
        loads.append({"region": 3, "byte_addr": int(rq_addr), "hex": rq_hex})
    ranges = [[int(x.split(":")[0]), int(x.split(":")[1])] for x in rng_s.split(",")]
    d = bytes.fromhex(desc_hex)
    return {"name": name, "desc_hex": desc_hex, "load": loads,
            "desc": {"mode": d[0], "flags": d[1], "M": m, "K": k, "N": n},
            "expected": {"obuf_byte_addr": int(o_addr), "obuf_hex": o_hex, "compare_ranges": ranges}}


async def setup(dut):
    check_geometry()
    c = Core(dut)
    await c.init()
    return c


@cocotb.test()
async def test_directed_jobs(dut):
    """All sw/crates/vec directed.json jobs (SPEC §6 Array/Sequencer rows): ±128/127 at
    K=256 int8+int32, zeros, identity, all-ones, single nonzero at every output position
    (FULL 8x16, STRIP 4x16), quadrant-only, requant ties/saturation, edge shapes 1x1x1 ..
    64x256x256 in both modes and both output paths. Back to back, no reset."""
    c = await setup(dut)
    d = load_directed()
    dut._log.info(f"directed.json generator {d['meta'].get('git_hash')} dirty={d['meta'].get('git_dirty')}")
    jobs = d["jobs"] if STRIP_EN else [j for j in d["jobs"] if j["desc"]["mode"] == 0]
    if float(os.environ.get("SCALE", "1")) < 1:
        jobs = [j for j in jobs if j["M"] * j["N"] * j["K"] <= 64 * 64 * 64]
    c.prefetch(jobs)
    for j in jobs:
        await c.job(j)
    c.report(dut, "directed")


@cocotb.test()
async def test_mode_switch_sequences(dut):
    """directed.json `sequences` (FULL -> STRIP -> FULL with no reset) plus a longer
    alternating sequence; outputs and counters checked for every job."""
    c = await setup(dut)
    d = load_directed()
    by = {j["name"]: j for j in d["jobs"]}
    c.prefetch(list(by.values()))
    for s in d["sequences"]:
        for nm in s["jobs"]:
            await c.job(by[nm], f"{s['name']}:{nm}")
    alt = ["full_edge_7x17x15_rq", "strip_edge_7x17x15_i32", "full_edge_5x2x9_i32",
           "strip_edge_5x2x9_rq", "full_edge_9x33x17_i32", "strip_edge_9x33x17_rq"]
    for nm in alt * 2:
        await c.job(by[nm], f"alt:{nm}")
    c.report(dut, "sequences")


@cocotb.test()
async def test_drain_boundary_pe77(dut):
    """PE(7,7) zero-slack boundary (D-019): a single nonzero product landing only in PE(7,7)
    (FULL) / PE(7,7) = C[3,15] (STRIP), at K=1 and K=256, k0 = last k, int32 and int8.
    Also nonzero descriptor bases (a/w/out/rq) to exercise base offsets."""
    c = await setup(dut)
    rng = seed_rng(dut, "test_drain_boundary_pe77")
    jobs = []
    for mode, rows, cols, (r, cc) in ((0, 8, 8, (7, 7)), (1, 4, 16, (3, 15))):
        for k in (1, 256):
            for rq in (None, [(1, 1)] * cols):
                a = [[0] * k for _ in range(rows)]
                w = [[0] * cols for _ in range(k)]
                a[r][k - 1] = -128
                w[k - 1][cc] = 127 if rq is None else -1
                bases = (rng.randint(0, 64), rng.randint(0, 64), rng.randint(0, 64), rng.randint(0, 64))
                jobs.append(oracle_job(mode, a, w, rq, bases,
                                       f"pe77 {MODE[mode]} K={k} {'int8' if rq else 'int32'} bases={bases}"))
    # dense K=256 single tile in both modes with every PE active
    for mode, rows, cols in ((0, 8, 8), (1, 4, 16)):
        a = [[rand_i8(rng) for _ in range(256)] for _ in range(rows)]
        w = [[rand_i8(rng) for _ in range(cols)] for _ in range(256)]
        jobs.append(oracle_job(mode, a, w, None, (0, 0, 0, 0), f"dense {MODE[mode]} K=256 int32"))
    c.prefetch(jobs)
    for j in jobs:
        await c.job(j)
    c.report(dut, "PE(7,7) boundary")


@cocotb.test()
async def test_random_gemm(dut):
    """Random jobs from `splitsys-vec gemm` (seed = SEED) in both modes, back to back."""
    c = await setup(dut)
    vd = os.environ.get("VEC_DIR")
    files = []
    for mode in ("full", "strip"):
        p = os.path.join(vd, "gemm", f"core_{mode}.json")
        check(os.path.exists(p), "vec gemm file missing (make -C tb core generates it)", path=p)
        files.append(p)
    n = 0
    for p in files:
        with open(p) as f:
            d = json.load(f)
        jobs = d["jobs"] if isinstance(d, dict) else d
        jobs = jobs[: scaled(len(jobs))]
        c.prefetch(jobs)
        dut._log.info(f"{p}: {len(jobs)} jobs, meta={d.get('meta') if isinstance(d, dict) else None}")
        for j in jobs:
            await c.job(j)
            n += 1
    c.report(dut, f"random gemm ({n} jobs)")


@cocotb.test()
async def test_random_custom(dut):
    """Random shapes with random bases and per-column requant params, built by golden
    Job::new (tb/oracle `job`), alternating modes and output paths."""
    c = await setup(dut)
    rng = seed_rng(dut, "test_random_custom")
    n = scaled(24)
    jobs = []
    for i in range(n):
        mode = rng.randint(0, 1) if STRIP_EN else 0
        m, k, nn = rng.randint(1, 20), rng.choice([1, 2, rng.randint(1, 48)]), rng.randint(1, 40)
        a = [[rand_i8(rng) for _ in range(k)] for _ in range(m)]
        w = [[rand_i8(rng) for _ in range(nn)] for _ in range(k)]
        rq = [(rng.randint(1, 32767), rng.randint(1, 31)) for _ in range(nn)] if rng.random() < 0.6 else None
        bases = (rng.randint(0, 100), rng.randint(0, 100), rng.randint(0, 64), rng.randint(0, 100))
        jobs.append(oracle_job(mode, a, w, rq, bases, f"custom {i}"))
    c.prefetch(jobs)
    for j in jobs:
        await c.job(j)
    c.report(dut, f"random custom ({n} jobs)")
