"""cocotb unit tests for rtl/buffers.sv (SPEC v1.1 §3.4, D-002, §6 row "Buffers").

Reference: a Python memory image (identity: what was written is what is read),
with the header's documented semantics: read-first on both ports, read data
exactly L_RD cycles after the read cycle (L_RD from the package), a_rdata held
until the next port-A read result, a_rvalid a 1-cycle pulse, b_rdata held while
b_en = 0, byte enables write only their lanes, memory powers up as zero.
All four region geometries from the package are tested.
"""

from __future__ import annotations

import cocotb

from splitsys_tb import (FallingEdge, RisingEdge, check, pkg_const, reset, scaled, seed_rng,
                         start_clock, uval)


def geoms():
    return {
        "w": (pkg_const("WBUF_WORDS"), pkg_const("WBUF_WORD_BYTES")),
        "a": (pkg_const("ABUF_WORDS"), pkg_const("ABUF_WORD_BYTES")),
        "o": (pkg_const("OBUF_WORDS"), pkg_const("OBUF_WORD_BYTES")),
        "r": (pkg_const("RQ_ENTRIES"), pkg_const("RQ_WORD_BYTES")),
    }


def sig(dut, p, n):
    return getattr(dut, f"{p}_{n}")


async def setup(dut):
    for p in geoms():
        for n in ("a_en", "a_we", "a_addr", "a_wdata", "b_en", "b_we", "b_addr", "b_wdata"):
            sig(dut, p, n).value = 0
    await start_clock(dut)
    await reset(dut)


_MODELS = {}


def model_for(p):
    """One memory image per geometry for the whole simulation: BRAM contents are not
    reset (header), so state carries across tests."""
    if p not in _MODELS:
        _MODELS[p] = Model(*geoms()[p])
    return _MODELS[p]


class Model:
    def __init__(self, words, wb):
        self.words, self.wb = words, wb
        self.mem = {}

    def read(self, addr):
        return self.mem.get(addr, 0)

    def write(self, addr, mask, data):
        old = self.read(addr)
        for i in range(self.wb):
            if (mask >> i) & 1:
                old = (old & ~(0xFF << (8 * i))) | (data & (0xFF << (8 * i)))
        self.mem[addr] = old


async def run(dut, p, model, ops, label):
    """ops: list per cycle of dict(a=(we_mask, addr, wdata)|None, b=...|None).
    Checks a_rdata/a_rvalid/b_rdata every cycle against the model (read-first)."""
    lrd = pkg_const("L_RD")
    words, wb = geoms()[p]
    a_res, b_res = {}, {}          # cycle issued -> (value)
    a_last, b_last = None, None    # latest result visible
    n = len(ops) + lrd + 2
    ops = ops + [{}] * (lrd + 2)
    for c in range(n):
        op = ops[c]
        await RisingEdge(dut.clk)
        for port in ("a", "b"):
            o = op.get(port)
            sig(dut, p, f"{port}_en").value = 1 if o else 0
            sig(dut, p, f"{port}_we").value = o[0] if o else 0
            sig(dut, p, f"{port}_addr").value = o[1] if o else 0
            sig(dut, p, f"{port}_wdata").value = o[2] if o else 0
        # model: reads see the state before this cycle's writes (read-first)
        if op.get("a"):
            if op["a"][0] == 0:
                a_res[c] = model.read(op["a"][1])
        if op.get("b"):
            b_res[c] = model.read(op["b"][1])          # b write cycles also return OLD word
        for port in ("a", "b"):
            o = op.get(port)
            if o and o[0]:
                model.write(o[1], o[0], o[2])
        await FallingEdge(dut.clk)
        src = c - lrd
        a_valid_now = src in a_res
        if a_valid_now:
            a_last = a_res[src]
        if src in b_res:
            b_last = b_res[src]
        got_rv = uval(sig(dut, p, "a_rvalid"))
        check(got_rv == (1 if a_valid_now else 0), f"[{p}:{label}] a_rvalid", cycle=c,
              expected=int(a_valid_now), got=got_rv, read_issued_cycle=src if a_valid_now else None)
        if a_last is not None:
            got = uval(sig(dut, p, "a_rdata"))
            check(got == a_last, f"[{p}:{label}] a_rdata", cycle=c, expected=hex(a_last), got=hex(got),
                  last_read_issued=max(k for k in a_res if k <= src),
                  ops_from_issue=ops[max(k for k in a_res if k <= src):c + 1])
        if b_last is not None:
            got = uval(sig(dut, p, "b_rdata"))
            check(got == b_last, f"[{p}:{label}] b_rdata", cycle=c, expected=hex(b_last), got=hex(got),
                  last_read_issued=max(k for k in b_res if k <= src),
                  ops_from_issue=ops[max(k for k in b_res if k <= src):c + 1])


def full(wb):
    return (1 << wb) - 1


@cocotb.test()
async def test_latency_equals_L_RD(dut):
    """Port A and port B read data appear exactly L_RD cycles after the read cycle."""
    await setup(dut)
    lrd = pkg_const("L_RD")
    for p, (words, wb) in geoms().items():
        m = model_for(p)
        val = int.from_bytes(bytes(range(1, wb + 1)), "little")
        await run(dut, p, m, [{"a": (full(wb), 5, val)}, {}, {}], "write")
        # issue a read on both ports in cycle 0 and watch for the value
        sig(dut, p, "a_en").value = 0
        seen = {}
        for c in range(8):
            await RisingEdge(dut.clk)
            on = 1 if c == 0 else 0
            for port in ("a", "b"):
                sig(dut, p, f"{port}_en").value = on
                sig(dut, p, f"{port}_we").value = 0
                sig(dut, p, f"{port}_addr").value = 5
            await FallingEdge(dut.clk)
            for port in ("a", "b"):
                if port not in seen and uval(sig(dut, p, f"{port}_rdata")) == val:
                    seen[port] = c
        dut._log.info(f"[{p}] measured read latency A={seen.get('a')} B={seen.get('b')}, L_RD={lrd}")
        check(seen.get("a") == lrd and seen.get("b") == lrd, f"[{p}] read latency != L_RD",
              measured=seen, L_RD=lrd)


@cocotb.test()
async def test_power_up_zero_first_last_address(dut):
    """Unwritten words read 0; first and last word on both ports, written from either port."""
    await setup(dut)
    rng = seed_rng(dut, "test_power_up_zero_first_last_address")
    for p, (words, wb) in geoms().items():
        m = model_for(p)
        ops = [{"a": (0, 0, 0)}, {"b": (0, words - 1, 0)}, {"a": (0, words // 2, 0)}]
        for addr in (0, words - 1):
            v1 = rng.getrandbits(8 * wb)
            v2 = rng.getrandbits(8 * wb)
            ops += [{"a": (full(wb), addr, v1)}, {"b": (0, addr, 0)}, {"a": (0, addr, 0)},
                    {"b": (full(wb), addr, v2)}, {"a": (0, addr, 0)}, {"b": (0, addr, 0)}]
        await run(dut, p, m, ops, "first/last")


@cocotb.test()
async def test_byte_enables(dut):
    """Partial-word writes on both ports: only enabled lanes change (D-002)."""
    await setup(dut)
    rng = seed_rng(dut, "test_byte_enables")
    for p, (words, wb) in geoms().items():
        m = model_for(p)
        ops = []
        addr = rng.randrange(words)
        ops.append({"a": (full(wb), addr, rng.getrandbits(8 * wb))})
        for i in range(wb):                       # each single lane, from each port
            port = "a" if i % 2 == 0 else "b"
            ops.append({port: (1 << i, addr, rng.getrandbits(8 * wb))})
            ops.append({"a": (0, addr, 0), "b": (0, addr, 0)})
        for _ in range(40):
            port = rng.choice("ab")
            ops.append({port: (rng.randrange(1, 1 << wb), addr, rng.getrandbits(8 * wb))})
            ops.append({"a": (0, addr, 0)})
        await run(dut, p, m, ops, "byte enables")


@cocotb.test()
async def test_simultaneous_host_write_array_read(dut):
    """Same cycle: A writes X while B reads X (B gets OLD, read-first) and vice versa;
    A write X with B read Y; both ports read the same word."""
    await setup(dut)
    rng = seed_rng(dut, "test_simultaneous_host_write_array_read")
    for p, (words, wb) in geoms().items():
        m = model_for(p)
        x, y = 3, words - 2
        ops = [{"a": (full(wb), x, rng.getrandbits(8 * wb))},
               {"b": (full(wb), y, rng.getrandbits(8 * wb))},
               {"a": (full(wb), x, rng.getrandbits(8 * wb)), "b": (0, x, 0)},
               {"b": (0, x, 0)},
               {"b": (full(wb), x, rng.getrandbits(8 * wb)), "a": (0, x, 0)},
               {"a": (0, x, 0)},
               {"a": (full(wb), x, rng.getrandbits(8 * wb)), "b": (0, y, 0)},
               {"a": (0, y, 0), "b": (0, y, 0)},
               {"a": (0, x, 0), "b": (0, x, 0)}]
        await run(dut, p, m, ops, "collision")


@cocotb.test()
async def test_random(dut):
    """Random reads/writes/partial writes on both ports every cycle, small address window
    (incl. first/last word) to force same-address read/write overlap; no dual writes
    to one word in one cycle (undefined per header)."""
    await setup(dut)
    rng = seed_rng(dut, "test_random")
    total = 0
    for p, (words, wb) in geoms().items():
        m = model_for(p)
        window = sorted({0, 1, 2, words - 1, words - 2} | {rng.randrange(words) for _ in range(6)})
        ops = []
        for _ in range(scaled(3000)):
            op = {}
            for port in ("a", "b"):
                r = rng.random()
                if r < 0.3:
                    continue
                addr = rng.choice(window)
                if r < 0.65:
                    op[port] = (0, addr, 0)
                else:
                    mask = full(wb) if rng.random() < 0.5 else rng.randrange(1, 1 << wb)
                    op[port] = (mask, addr, rng.getrandbits(8 * wb))
            if op.get("a") and op.get("b") and op["a"][0] and op["b"][0] and op["a"][1] == op["b"][1]:
                del op["b"]
            ops.append(op)
        await run(dut, p, m, ops, "random")
        total += len(ops)
    dut._log.info(f"{total} random cycles over 4 geometries")
