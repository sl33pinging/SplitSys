"""cocotb unit tests for rtl/packet.sv (SPEC v1.1 §3.5 validation, §3.8 + D-010, §6 "UART + packet").

The DUT is driven at its byte interface (rx_data/rx_valid in, tx_data/tx_valid/tx_ready out);
UART serialization is tested in tb/uart. Every request frame and every expected response
frame (sync, status, len, payload, CRC-8) is built by tb/oracle from
splitsys_golden::packet::{request_frame, response_frame} -- nothing is CRC'd in Python.
Expected payload *contents* are identities: memory bytes written earlier (scoreboard image),
stub-provided counter/self-test values, the PING constants of SPEC §3.8.
Accept/reject of RUN_GEMM descriptors uses directed descriptors whose outcome is stated
by SPEC §3.5 rules 1-8 (one rule violated per case, boundary just-in / just-out).

Stubs (one per-cycle coroutine, `TB.loop`): memory regions with an exact-L_RD read port
(garbage on the neighbouring cycles), sequencer (run_valid/run_accept handshake, then
seq_busy, then seq_done held), counters snapshot (read latency 1), self-test engine.
"""

from __future__ import annotations

import os
from collections import deque

import cocotb
from cocotb.triggers import ClockCycles, Event, First

from splitsys_tb import (FallingEdge, RisingEdge, check, oracle, pkg_const, reset, scaled,
                         seed_rng, start_clock, uval)

# SPEC §3.8 opcode / status tables
PING, WRITE_MEM, READ_MEM, RUN_GEMM, POLL, READ_COUNTERS, SELFTEST, SET_DISPLAY = \
    0x01, 0x10, 0x11, 0x20, 0x21, 0x30, 0x40, 0x50
KNOWN_OPS = {PING, WRITE_MEM, READ_MEM, RUN_GEMM, POLL, READ_COUNTERS, SELFTEST, SET_DISPLAY}
ACK, NACK_CRC, NACK_OPCODE, NACK_RANGE, BUSY = 0, 1, 2, 3, 4
POLL_IDLE, POLL_BUSY, POLL_DONE = 0, 1, 2

TIMEOUT = int(os.environ.get("PKT_RX_TIMEOUT", "2000"))
BUILD_ID = int(os.environ.get("PKT_BUILD_ID", "1592594996"))


def regions():
    return {
        0: (pkg_const("WBUF_BYTES"), pkg_const("WBUF_WORD_BYTES")),
        1: (pkg_const("ABUF_BYTES"), pkg_const("ABUF_WORD_BYTES")),
        2: (pkg_const("OBUF_BYTES"), pkg_const("OBUF_WORD_BYTES")),
        3: (pkg_const("RQ_BYTES"), pkg_const("RQ_WORD_BYTES")),
    }


def le(v: int, n: int) -> bytes:
    return int(v).to_bytes(n, "little")


def reqframes(items):
    """items: list of (opcode, payload bytes) -> list of frame bytes (golden)."""
    out = oracle([f"reqframe {op} {p.hex() or '-'}" for op, p in items])
    return [bytes.fromhex(x) for x in out]


def rspframes(items):
    out = oracle([f"rspframe {st} {p.hex() or '-'}" for st, p in items])
    return [bytes.fromhex(x) for x in out]


def corrupt(frame: bytes, rng) -> bytes:
    """Flip bits in one byte after the sync byte, never the len field (so the frame keeps
    its length); CRC-8 detects every single-byte error."""
    i = rng.choice([1] + list(range(4, len(frame))))
    b = bytearray(frame)
    b[i] ^= rng.randint(1, 255)
    return bytes(b)


class TB:
    def __init__(self, dut, rng):
        self.dut = dut
        self.rng = rng
        self.l_rd = pkg_const("L_RD")
        self.regions = regions()
        self.mem = {r: bytearray(n) for r, (n, _) in self.regions.items()}
        self.exp_mem = {r: bytearray(n) for r, (n, _) in self.regions.items()}
        self.cycle = 0
        self.rxq: deque = deque()
        self.last_rx_cycle = -1
        self.bp_prob = 0.05
        self.bp_left = 0
        self.ready = 1
        self.cache = {}
        # tx parse
        self.cur = bytearray()
        self.cur_start = None
        self.responses = deque()   # (start_cycle, bytes)
        self.resp_event = Event()
        # memory stub
        self.rd_sched = {}
        self.last_we = -100
        self.last_re = -100
        self.cmd_region = None
        self.n_writes = 0
        self.n_reads = 0
        # sequencer stub
        self.force_busy = False
        self.run_pending_since = None
        self.accept_at = None
        self.busy_left = 0
        self.seq_done = 0
        self.accepted = []
        self.accept_delay = (0, 6)
        self.busy_len = (10, 40)
        self.rv_prev = 0
        # counters stub
        self.snap = [0] * pkg_const("N_COUNTERS")
        self.snaps = []
        self.idx_prev = 0
        # self-test stub
        self.st_vals = (1, 0, 0x89ABCDEF, 0x01234567)
        self.st_force_busy = False
        self.st_left = None
        self.st_starts = 0
        # display
        self.disp = []
        self.errors = []

    # ----------------------------------------------------------------- driving
    def w(self, name, val):
        if self.cache.get(name) != val:
            self.cache[name] = val
            getattr(self.dut, name).value = val

    def live(self, i, c):
        return (((i + 1) << 40) | ((c * (i + 3) + 0x5A5A) & 0xFFFFFFFFFF)) & ((1 << 48) - 1)

    async def init(self):
        d = self.dut
        for n in ("rx_data", "rx_valid", "tx_ready", "mem_rdata", "run_accept", "seq_busy",
                  "seq_done", "seq_error", "ctr_data", "st_busy", "st_done", "st_pass_full",
                  "st_pass_strip", "st_fl_full", "st_fl_strip"):
            getattr(d, n).value = 0
        await start_clock(d)
        await reset(d)
        pf, ps, ff, fs = self.st_vals
        d.st_pass_full.value = pf
        d.st_pass_strip.value = ps
        d.st_fl_full.value = ff
        d.st_fl_strip.value = fs
        cocotb.start_soon(self.loop())

    def err(self, msg):
        self.errors.append(f"cycle {self.cycle}: {msg}")

    async def loop(self):
        d = self.dut
        rng = self.rng
        L = self.l_rd
        while True:
            await RisingEdge(d.clk)
            c = self.cycle = self.cycle + 1
            # ---- rx byte
            b = self.rxq.popleft() if self.rxq else None
            if b is None:
                self.w("rx_valid", 0)
            else:
                self.w("rx_valid", 1)
                self.w("rx_data", b)
                self.last_rx_cycle = c
            # ---- tx backpressure
            if self.bp_left > 0:
                self.bp_left -= 1
                self.ready = 0
            elif self.bp_prob and rng.random() < self.bp_prob:
                self.bp_left = rng.randint(0, 12)
                self.ready = 0
            else:
                self.ready = 1
            self.w("tx_ready", self.ready)
            # ---- memory read data (exactly L_RD after mem_re; garbage around it)
            if c in self.rd_sched:
                self.w("mem_rdata", self.rd_sched.pop(c))
            # ---- counters: read latency 1 from ctr_idx
            self.w("ctr_data", self.snap[self.idx_prev] if self.idx_prev < len(self.snap) else 0)
            # ---- sequencer stub
            acc = 1 if (self.accept_at is not None and c >= self.accept_at) else 0
            self.w("run_accept", acc)
            if self.busy_left > 0:
                self.busy_left -= 1
                if self.busy_left == 0:
                    self.seq_done = 1
            self.w("seq_busy", 1 if (self.force_busy or self.busy_left > 0) else 0)
            self.w("seq_done", self.seq_done)
            # ---- self-test stub
            st_done = 0
            if self.st_left is not None:
                self.st_left -= 1
                if self.st_left == 0:
                    st_done = 1
                    self.st_left = None
            self.w("st_done", st_done)
            self.w("st_busy", 1 if (self.st_force_busy or self.st_left is not None) else 0)

            await FallingEdge(d.clk)
            # ---- tx
            if self.ready and uval(d.tx_valid):
                byte = uval(d.tx_data)
                if not self.cur:
                    self.cur_start = c
                self.cur.append(byte)
                if self.cur[0] != 0x5A:
                    self.err(f"response does not start with 0x5A: {self.cur.hex()}")
                    self.cur = bytearray()
                elif len(self.cur) >= 4 and len(self.cur) == 5 + (self.cur[2] | self.cur[3] << 8):
                    self.responses.append((self.cur_start, bytes(self.cur)))
                    self.cur = bytearray()
                    self.resp_event.set()
            # ---- memory
            if uval(d.mem_we):
                self.n_writes += 1
                if c - self.last_we < 2:
                    self.err(f"mem_we in consecutive cycles ({self.last_we}, {c})")
                self.last_we = c
                reg = uval(d.mem_region)
                addr = uval(d.mem_addr)
                wd = uval(d.mem_wdata)
                be = uval(d.mem_be)
                if self.cmd_region is None or reg != self.cmd_region:
                    self.err(f"mem_we region {reg}, command region {self.cmd_region}")
                nbytes, wb = self.regions.get(reg, (0, 1))
                if addr * wb >= nbytes:
                    self.err(f"mem_we addr {addr} out of region {reg}")
                elif be >> wb:
                    self.err(f"mem_be {be:#x} enables lanes >= {wb} in region {reg}")
                else:
                    for i in range(wb):
                        if be >> i & 1:
                            self.mem[reg][addr * wb + i] = (wd >> (8 * i)) & 0xFF
            if uval(d.mem_re):
                self.n_reads += 1
                if c - self.last_re < L:
                    self.err(f"mem_re with a read outstanding ({self.last_re}, {c})")
                self.last_re = c
                reg = uval(d.mem_region)
                addr = uval(d.mem_addr)
                if self.cmd_region is None or reg != self.cmd_region:
                    self.err(f"mem_re region {reg}, command region {self.cmd_region}")
                nbytes, wb = self.regions.get(reg, (0, 1))
                if addr * wb >= nbytes:
                    self.err(f"mem_re addr {addr} out of region {reg}")
                    word = 0
                else:
                    word = int.from_bytes(self.mem[reg][addr * wb:(addr + 1) * wb], "little")
                    # lanes above the region width carry garbage (packet must not use them)
                    word |= (rng.getrandbits(8 * (16 - wb)) << (8 * wb)) if wb < 16 else 0
                garbage = rng.getrandbits(128)
                if L >= 2:
                    self.rd_sched[c + 1] = garbage
                self.rd_sched[c + L] = word
                self.rd_sched[c + L + 1] = garbage ^ ((1 << 128) - 1)
            # ---- sequencer handshake
            rv = uval(d.run_valid)
            if self.rv_prev and not rv and not self._accepted_last:
                self.err("run_valid dropped without run_accept")
            self._accepted_last = False
            if rv and self.run_pending_since is None:
                self.run_pending_since = c
                self.accept_at = c + 1 + rng.randint(*self.accept_delay)
            if rv and acc:
                self.accepted.append(uval(d.run_desc).to_bytes(24, "little"))
                self._accepted_last = True
                self.run_pending_since = None
                self.accept_at = None
                self.seq_done = 0
                self.busy_left = rng.randint(*self.busy_len)
            self.rv_prev = rv
            # ---- counters
            if uval(d.ctr_snap):
                self.snap = [self.live(i, c) for i in range(len(self.snap))]
                self.snaps.append(list(self.snap))
            self.idx_prev = uval(d.ctr_idx)
            # ---- self-test
            if uval(d.st_start):
                self.st_starts += 1
                self.st_left = rng.randint(5, 60)
            if uval(d.disp_valid):
                self.disp.append((uval(d.disp_util), uval(d.disp_mode)))

    _accepted_last = False

    # ------------------------------------------------------------- transactions
    def send(self, frame: bytes, gap_max: int = 2, gap_p: float = 0.3):
        for b in frame:
            self.rxq.append(b)
            if gap_max and self.rng.random() < gap_p:
                self.rxq.extend([None] * self.rng.randint(1, gap_max))

    async def response(self, max_cycles: int = 200000):
        """Wait for the next complete response frame; returns (start_cycle, bytes)."""
        start = self.cycle
        while not self.responses:
            self.resp_event.clear()
            await First(self.resp_event.wait(), ClockCycles(self.dut.clk, 2000))
            if self.cycle - start > max_cycles:
                raise AssertionError(f"no response within {max_cycles} cycles "
                                     f"(dbg_state={uval(self.dut.dbg_state)}, partial={self.cur.hex()})")
        return self.responses.popleft()

    async def xact(self, frame: bytes, **kw):
        self.send(frame, **kw)
        return await self.response()

    async def quiet(self, cycles: int):
        """Idle the line and assert that no response bytes appear."""
        n0 = len(self.responses)
        while self.rxq:                       # idle time counts from the last queued byte
            await ClockCycles(self.dut.clk, 16)
        await ClockCycles(self.dut.clk, cycles)
        check(len(self.responses) == n0 and not self.cur, "unexpected response bytes",
              responses=[r[1].hex() for r in list(self.responses)[n0:]], partial=self.cur.hex())

    def apply_write(self, region, addr, data):
        self.exp_mem[region][addr:addr + len(data)] = data

    def check_mem(self, label):
        for r in self.mem:
            if self.mem[r] != self.exp_mem[r]:
                diffs = [i for i in range(len(self.mem[r])) if self.mem[r][i] != self.exp_mem[r][i]]
                check(False, f"[{label}] memory image mismatch", region=r, n_diff=len(diffs),
                      first_byte_addrs=diffs[:8], got=[self.mem[r][i] for i in diffs[:8]],
                      expected=[self.exp_mem[r][i] for i in diffs[:8]])

    def check_errors(self):
        check(not self.errors, "bus protocol errors", first=self.errors[:5], n=len(self.errors))


class Checker:
    """Collects (label, request, got response, expected status, expected payload);
    verifies all against golden response frames in one oracle batch."""

    def __init__(self):
        self.items = []

    def add(self, label, req, got, st, payload=b""):
        self.items.append((label, req, got, st, bytes(payload)))

    def verify(self):
        if not self.items:
            return 0
        exp = rspframes([(st, p) for (_, _, _, st, p) in self.items])
        for (label, req, got, st, p), e in zip(self.items, exp):
            check(got == e, f"[{label}] response mismatch", request=req[:24].hex() + ("..." if len(req) > 24 else ""),
                  req_len=len(req), expected=e[:40].hex(), got=got[:40].hex(), exp_status=st,
                  got_status=got[1] if len(got) > 1 else None, exp_len=len(e), got_len=len(got))
        n = len(self.items)
        self.items = []
        return n


async def setup(dut, name, **kw):
    rng = seed_rng(dut, name)
    tb = TB(dut, rng)
    for k, v in kw.items():
        setattr(tb, k, v)
    await tb.init()
    return tb, Checker()


def desc(mode=0, flags=1, m=8, k=16, n=16, a=0, w=0, o=0, rq=0, resv=0) -> bytes:
    return (le(mode, 1) + le(flags, 1) + le(m, 2) + le(k, 2) + le(n, 2) + le(a, 4) + le(w, 4)
            + le(o, 4) + le(rq, 2) + le(resv, 2))


async def wait_seq_idle(tb):
    while tb.busy_left > 0 or tb.run_pending_since is not None:
        await ClockCycles(tb.dut.clk, 8)


# =============================================================================
@cocotb.test()
async def test_ping(dut):
    """PING: ACK, len 9, "SPLT", PROTO_VERSION, BUILD_ID (LE). Garbage before sync ignored."""
    tb, ck = await setup(dut, "test_ping")
    dut._log.info(f"BUILD_ID override = {BUILD_ID:#010x}")
    (f,) = reqframes([(PING, b"")])
    payload = le(pkg_const("PING_MAGIC_LE"), 4) + le(pkg_const("PROTO_VERSION"), 1) + le(BUILD_ID, 4)
    check(payload[:4] == b"SPLT", "PING magic from package", got=payload[:4])
    _, r = await tb.xact(f)
    ck.add("ping", f, r, ACK, payload)
    # garbage (no 0xA5) before the frame
    junk = bytes(x for x in (tb.rng.randint(0, 255) for _ in range(40)) if x != 0xA5)
    _, r = await tb.xact(junk + f)
    ck.add("ping after garbage", f, r, ACK, payload)
    ck.verify()
    tb.check_errors()


@cocotb.test()
async def test_write_read_all_regions(dut):
    """WRITE_MEM/READ_MEM round trip in every region: first and last byte, unaligned
    addresses, partial words, max 1019 data bytes, READ_MEM rlen 1024."""
    tb, ck = await setup(dut, "test_write_read_all_regions")
    rng = tb.rng
    for reg, (nbytes, wb) in tb.regions.items():
        cases = [(0, 1), (nbytes - 1, 1), (1, wb + 1), (wb - 1, 2), (3, 2 * wb + 3),
                 (nbytes - 1019, 1019), (rng.randint(0, nbytes - 1019), 1019),
                 (rng.randint(0, nbytes - 64) | 1, rng.randint(1, 63))]
        for addr, n in cases:
            data = bytes(rng.randint(0, 255) for _ in range(n))
            f, = reqframes([(WRITE_MEM, le(reg, 1) + le(addr, 4) + data)])
            tb.cmd_region = reg
            _, r = await tb.xact(f)
            ck.add(f"write r{reg} @{addr}+{n}", f, r, ACK)
            tb.apply_write(reg, addr, data)
            tb.check_mem(f"after write r{reg} @{addr}+{n}")
        reads = [(0, 1), (nbytes - 1, 1), (nbytes - 1024, 1024), (0, 1024 if nbytes >= 1024 else nbytes),
                 (wb + 1, 2 * wb - 1)]
        for addr, n in reads:
            f, = reqframes([(READ_MEM, le(reg, 1) + le(addr, 4) + le(n, 2))])
            _, r = await tb.xact(f)
            ck.add(f"read r{reg} @{addr}+{n}", f, r, ACK, tb.exp_mem[reg][addr:addr + n])
    dut._log.info(f"{tb.n_writes} mem writes, {tb.n_reads} mem reads; {ck.verify()} responses golden-exact")
    tb.check_errors()


@cocotb.test()
async def test_bad_crc(dut):
    """Corrupted frames -> NACK_CRC (len 0) and no memory side effect, any opcode."""
    tb, ck = await setup(dut, "test_bad_crc")
    rng = tb.rng
    tb.cmd_region = 0
    for i in range(40):
        op = rng.choice([WRITE_MEM, READ_MEM, PING, RUN_GEMM, SELFTEST, 0x77, SET_DISPLAY])
        if op == WRITE_MEM:
            p = le(rng.randint(0, 3), 1) + le(rng.randint(0, 900), 4) + bytes(rng.randint(0, 255) for _ in range(rng.randint(1, 100)))
        elif op == RUN_GEMM:
            p = desc()
        else:
            p = bytes(rng.randint(0, 255) for _ in range(rng.randint(0, 8)))
        f, = reqframes([(op, p)])
        # corrupt the CRC byte or a body byte, but keep the len field (so the frame length is unchanged)
        bad = bytearray(f)
        pos = rng.choice([len(f) - 1, 1] + ([rng.randint(4, len(f) - 2)] if len(f) > 5 else []))
        bad[pos] ^= rng.randint(1, 255)
        _, r = await tb.xact(bytes(bad))
        ck.add(f"bad crc op {op:#x} byte {pos}", bytes(bad), r, NACK_CRC)
    check(tb.n_writes == 0 and tb.st_starts == 0 and not tb.accepted, "side effect from a bad-CRC frame",
          writes=tb.n_writes, st_starts=tb.st_starts, runs=len(tb.accepted))
    tb.check_mem("bad crc")
    ck.verify()
    tb.check_errors()


@cocotb.test()
async def test_unknown_opcode_and_precedence(dut):
    """Unknown opcodes -> NACK_OPCODE. Precedence: CRC > OPCODE > BUSY > RANGE."""
    tb, ck = await setup(dut, "test_unknown_opcode_and_precedence")
    rng = tb.rng
    unknown = [op for op in range(256) if op not in KNOWN_OPS]
    for op in [0x00, 0x02, 0x12, 0x22, 0x31, 0x41, 0x51, 0xFF] + rng.sample(unknown, 12):
        f, = reqframes([(op, bytes(rng.randint(0, 255) for _ in range(rng.randint(0, 5))))])
        _, r = await tb.xact(f)
        ck.add(f"unknown op {op:#x}", f, r, NACK_OPCODE)
    # CRC beats OPCODE
    f, = reqframes([(0x77, b"\x01\x02")])
    bad = f[:-1] + bytes([f[-1] ^ 0x55])
    _, r = await tb.xact(bad)
    ck.add("crc>opcode", bad, r, NACK_CRC)
    tb.force_busy = True
    await ClockCycles(dut.clk, 2)
    # OPCODE beats BUSY
    f, = reqframes([(0x77, b"")])
    _, r = await tb.xact(f)
    ck.add("opcode>busy", f, r, NACK_OPCODE)
    # CRC beats BUSY
    f, = reqframes([(WRITE_MEM, le(0, 1) + le(0, 4) + b"\x11")])
    bad = f[:-1] + bytes([f[-1] ^ 1])
    _, r = await tb.xact(bad)
    ck.add("crc>busy", bad, r, NACK_CRC)
    # BUSY beats RANGE (length and field errors) for every busy-gated opcode
    busy_range = [(WRITE_MEM, le(9, 1) + le(0, 4) + b"\x00"), (WRITE_MEM, b"\x00" * 3),
                  (READ_MEM, le(0, 1) + le(0, 4) + le(0, 2)), (READ_MEM, b"\x00" * 6),
                  (RUN_GEMM, desc(mode=2)), (RUN_GEMM, desc()[:23]), (SELFTEST, b"\x00"),
                  (WRITE_MEM, le(0, 1) + le(0, 4) + b"\x22"), (READ_MEM, le(0, 1) + le(0, 4) + le(4, 2)),
                  (RUN_GEMM, desc()), (SELFTEST, b"")]
    for op, p in busy_range:
        f, = reqframes([(op, p)])
        _, r = await tb.xact(f)
        ck.add(f"busy op {op:#x} len {len(p)}", f, r, BUSY)
    check(tb.n_writes == 0 and tb.n_reads == 0 and not tb.accepted and tb.st_starts == 0,
          "side effect while BUSY", writes=tb.n_writes, reads=tb.n_reads, runs=len(tb.accepted),
          st=tb.st_starts)
    # always-served opcodes while busy: PING, POLL (-> BUSY state byte), READ_COUNTERS, SET_DISPLAY
    payload_ping = le(pkg_const("PING_MAGIC_LE"), 4) + le(pkg_const("PROTO_VERSION"), 1) + le(BUILD_ID, 4)
    f, = reqframes([(PING, b"")])
    _, r = await tb.xact(f)
    ck.add("ping while busy", f, r, ACK, payload_ping)
    f, = reqframes([(POLL, b"")])
    _, r = await tb.xact(f)
    ck.add("poll while busy", f, r, ACK, bytes([POLL_BUSY]))
    f, = reqframes([(READ_COUNTERS, b"")])
    _, r = await tb.xact(f)
    ck.add("counters while busy", f, r, ACK, b"".join(le(v, 6) for v in tb.snaps[-1]))
    f, = reqframes([(SET_DISPLAY, le(500, 2) + b"\x01")])
    _, r = await tb.xact(f)
    ck.add("display while busy", f, r, ACK)
    # range errors on always-served opcodes are still reported while busy
    f, = reqframes([(PING, b"\x00")])
    _, r = await tb.xact(f)
    ck.add("ping len1 while busy", f, r, NACK_RANGE)
    tb.force_busy = False
    # st_busy also makes the link busy
    tb.st_force_busy = True
    await ClockCycles(dut.clk, 2)
    f, = reqframes([(READ_MEM, le(0, 1) + le(0, 4) + le(4, 2))])
    _, r = await tb.xact(f)
    ck.add("read while st_busy", f, r, BUSY)
    f, = reqframes([(POLL, b"")])
    _, r = await tb.xact(f)
    ck.add("poll while st_busy", f, r, ACK, bytes([POLL_BUSY]))
    tb.st_force_busy = False
    ck.verify()
    tb.check_errors()


@cocotb.test()
async def test_nack_range_lengths_and_fields(dut):
    """NACK_RANGE (len 0) for every wrong payload length per opcode and every field range
    rule of SPEC §3.8; boundary-legal cases ACK."""
    tb, ck = await setup(dut, "test_nack_range_lengths_and_fields")
    cases = []   # (label, op, payload, status, expected payload)
    for n in (1, 2, 24):
        cases.append((f"PING len {n}", PING, b"\x00" * n, NACK_RANGE, b""))
        cases.append((f"POLL len {n}", POLL, b"\x00" * n, NACK_RANGE, b""))
        cases.append((f"READ_COUNTERS len {n}", READ_COUNTERS, b"\x00" * n, NACK_RANGE, b""))
        cases.append((f"SELFTEST len {n}", SELFTEST, b"\x00" * n, NACK_RANGE, b""))
    for n in (0, 1, 2, 4, 5):
        cases.append((f"SET_DISPLAY len {n}", SET_DISPLAY, b"\x00" * n, NACK_RANGE, b""))
    for n in (0, 1, 5):
        cases.append((f"WRITE_MEM len {n}", WRITE_MEM, b"\x00" * n, NACK_RANGE, b""))
    for n in (0, 6, 8, 12):
        cases.append((f"READ_MEM len {n}", READ_MEM, (le(0, 1) + le(0, 4) + le(1, 2) + b"\x00" * 8)[:n], NACK_RANGE, b""))
    for n in (0, 1, 23, 25, 48):
        cases.append((f"RUN_GEMM len {n}", RUN_GEMM, (desc() + desc())[:n], NACK_RANGE, b""))
    for reg in (4, 5, 0x80, 0xFF):
        cases.append((f"WRITE region {reg}", WRITE_MEM, le(reg, 1) + le(0, 4) + b"\x01", NACK_RANGE, b""))
        cases.append((f"READ region {reg}", READ_MEM, le(reg, 1) + le(0, 4) + le(1, 2), NACK_RANGE, b""))
    for reg, (nbytes, _) in tb.regions.items():
        cases.append((f"WRITE r{reg} past end", WRITE_MEM, le(reg, 1) + le(nbytes - 3, 4) + b"\x01" * 4, NACK_RANGE, b""))
        cases.append((f"WRITE r{reg} addr=size", WRITE_MEM, le(reg, 1) + le(nbytes, 4) + b"\x01", NACK_RANGE, b""))
        cases.append((f"WRITE r{reg} addr 2^32-1", WRITE_MEM, le(reg, 1) + le(0xFFFFFFFF, 4) + b"\x01", NACK_RANGE, b""))
        cases.append((f"READ r{reg} past end", READ_MEM, le(reg, 1) + le(nbytes - 3, 4) + le(4, 2), NACK_RANGE, b""))
        cases.append((f"READ r{reg} addr 2^32-1", READ_MEM, le(reg, 1) + le(0xFFFFFFFF, 4) + le(1, 2), NACK_RANGE, b""))
        cases.append((f"READ r{reg} at end", READ_MEM, le(reg, 1) + le(nbytes - 4, 4) + le(4, 2), ACK, bytes(4)))
    cases.append(("READ rlen 0", READ_MEM, le(0, 1) + le(0, 4) + le(0, 2), NACK_RANGE, b""))
    cases.append(("READ rlen 1025", READ_MEM, le(0, 1) + le(0, 4) + le(1025, 2), NACK_RANGE, b""))
    cases.append(("READ rlen 65535", READ_MEM, le(0, 1) + le(0, 4) + le(65535, 2), NACK_RANGE, b""))
    cases.append(("SET_DISPLAY util 1001", SET_DISPLAY, le(1001, 2) + b"\x00", NACK_RANGE, b""))
    cases.append(("SET_DISPLAY util 65535", SET_DISPLAY, le(65535, 2) + b"\x00", NACK_RANGE, b""))
    cases.append(("SET_DISPLAY mode 2", SET_DISPLAY, le(0, 2) + b"\x02", NACK_RANGE, b""))
    cases.append(("SET_DISPLAY mode 255", SET_DISPLAY, le(0, 2) + b"\xff", NACK_RANGE, b""))
    cases.append(("SET_DISPLAY util 1000 mode 1", SET_DISPLAY, le(1000, 2) + b"\x01", ACK, b""))
    cases.append(("SET_DISPLAY util 0 mode 0", SET_DISPLAY, le(0, 2) + b"\x00", ACK, b""))
    cases.append(("SET_DISPLAY util 937 mode 1", SET_DISPLAY, le(937, 2) + b"\x01", ACK, b""))
    frames = reqframes([(op, p) for (_, op, p, _, _) in cases])
    n_disp0 = len(tb.disp)
    for (label, op, p, st, ep), f in zip(cases, frames):
        tb.cmd_region = p[0] if op in (READ_MEM, WRITE_MEM) and p else None
        _, r = await tb.xact(f)
        ck.add(label, f, r, st, ep)
    check(tb.n_writes == 0, "NACKed WRITE_MEM wrote memory", writes=tb.n_writes)
    check(tb.disp[n_disp0:] == [(1000, 1), (0, 0), (937, 1)], "SET_DISPLAY outputs (disp_util, disp_mode)",
          got=tb.disp[n_disp0:])
    dut._log.info(f"{ck.verify()} range/length cases golden-exact")
    tb.check_errors()


def gemm_cases():
    """SPEC §3.5 rules 1-8: (label, descriptor, accept?). Each reject violates exactly one rule."""
    A, W, O, RQ = (pkg_const("ABUF_WORDS"), pkg_const("WBUF_WORDS"), pkg_const("OBUF_WORDS"),
                   pkg_const("RQ_ENTRIES"))
    c = [
        ("legal FULL", desc(), True),
        ("legal STRIP (ENABLE_STRIP=1)", desc(mode=1), True),
        ("legal int32", desc(flags=2), True),
        ("mode INDEP", desc(mode=2), False), ("mode 3", desc(mode=3), False), ("mode 255", desc(mode=255), False),
        ("flags 0", desc(flags=0), False), ("flags 3", desc(flags=3), False),
        ("flags 0x81", desc(flags=0x81), False), ("flags 0x05", desc(flags=0x05), False),
        ("M 0", desc(m=0), False), ("M 65", desc(m=65), False), ("M 64", desc(m=64), True),
        ("M 1", desc(m=1), True), ("M 0x8008", desc(m=0x8008), False),
        ("K 0", desc(k=0), False), ("K 257", desc(k=257), False), ("K 256", desc(k=256), True),
        ("K 1", desc(k=1), True), ("K 0x8010", desc(k=0x8010), False),
        ("N 0", desc(n=0), False), ("N 257", desc(n=257), False), ("N 256", desc(n=256), True),
        ("N 1", desc(n=1), True), ("N 0x8010", desc(n=0x8010), False),
        ("reserved 1", desc(resv=1), False), ("reserved 0x8000", desc(resv=0x8000), False),
        # rule 4: a_base + ceil(M/8)*K <= ABUF_WORDS
        ("A extent exact (M64 K256)", desc(m=64, k=256, a=0), True),
        ("A extent +1 (M64 K256 a=1)", desc(m=64, k=256, a=1), False),
        ("A extent exact (M9 K100)", desc(m=9, k=100, a=A - 200), True),
        ("A extent +1 (M9 K100)", desc(m=9, k=100, a=A - 199), False),
        ("A base 2^32-1", desc(a=0xFFFFFFFF), False),
        # rule 5: w_base + ceil(N/16)*K <= WBUF_WORDS
        ("W extent exact (N256 K256)", desc(n=256, k=256, w=0), True),
        ("W extent +1 (N256 K256 w=1)", desc(n=256, k=256, w=1), False),
        ("W extent exact (N17 K7)", desc(n=17, k=7, w=W - 14), True),
        ("W extent +1 (N17 K7)", desc(n=17, k=7, w=W - 13), False),
        ("W base 2^32-1", desc(w=0xFFFFFFFF), False),
        # rule 6: out_base + NB*M_pad*X <= OBUF_WORDS
        ("C int8 exact (M64 N256)", desc(m=64, n=256, o=0), True),
        ("C int8 +1 (M64 N256 o=1)", desc(m=64, n=256, o=1), False),
        ("C int8 exact (M9 N17)", desc(m=9, n=17, o=O - 32), True),
        ("C int8 +1 (M9 N17)", desc(m=9, n=17, o=O - 31), False),
        ("C int32 exact (M64 N64)", desc(flags=2, m=64, n=64, o=0), True),
        ("C int32 over (M64 N65)", desc(flags=2, m=64, n=65, o=0), False),
        ("C int32 exact (M1 N1 o=O-32)", desc(flags=2, m=1, n=1, o=O - 32), True),
        ("C int32 +1 (M1 N1 o=O-31)", desc(flags=2, m=1, n=1, o=O - 31), False),
        ("C base 2^32-1", desc(o=0xFFFFFFFF), False),
        # rule 7: rq_base + N <= RQ_ENTRIES (both flags)
        ("RQ exact (N256 rq0)", desc(n=256, rq=0), True),
        ("RQ +1 (N256 rq1)", desc(n=256, rq=1), False),
        ("RQ exact (N16 rq240)", desc(n=16, rq=RQ - 16), True),
        ("RQ +1 (N16 rq241)", desc(n=16, rq=RQ - 15), False),
        ("RQ +1 int32 (N16 rq241)", desc(flags=2, n=16, rq=RQ - 15), False),
        ("RQ base 65535", desc(rq=65535), False),
    ]
    return c


@cocotb.test()
async def test_run_gemm_validation(dut):
    """RUN_GEMM: each SPEC §3.5 rule violated alone -> NACK_RANGE and no run; legal and
    boundary-legal descriptors -> handshake (run_desc == payload) then ACK."""
    tb, ck = await setup(dut, "test_run_gemm_validation")
    strip_en = int(os.environ.get("PKT_STRIP_EN", "1"))
    cases = gemm_cases()
    # Expected accept/reject: golden Descriptor::from_bytes + validate via tb/oracle
    # (`descvalid`, with this build's STRIP_EN). The SPEC-text intent of each case is
    # cross-checked against golden; a disagreement is a golden/spec finding, not RTL.
    gold = oracle([f"descvalid {d.hex()} {strip_en}" for (_, d, _) in cases])
    checked = []
    for (label, d, intent), g in zip(cases, gold):
        ok = g.split()[0] == "1"
        intent_here = intent and not (strip_en == 0 and d[0] == 1)
        check(ok == intent_here, f"[{label}] golden validate disagrees with SPEC §3.5 intent",
              desc=d.hex(), golden=g, spec_intent=intent_here, strip_en=strip_en)
        checked.append((label + ("" if ok else f" [{g.split()[1]}]"), d, ok))
    cases = checked
    frames = reqframes([(RUN_GEMM, d) for (_, d, _) in cases])
    for (label, d, ok), f in zip(cases, frames):
        await wait_seq_idle(tb)
        n0 = len(tb.accepted)
        _, r = await tb.xact(f)
        ck.add(label, f, r, ACK if ok else NACK_RANGE)
        if ok:
            check(len(tb.accepted) == n0 + 1 and tb.accepted[-1] == d, f"[{label}] run_desc handshake",
                  accepted=len(tb.accepted) - n0, run_desc=tb.accepted[-1].hex() if tb.accepted else None,
                  expected=d.hex())
        else:
            check(len(tb.accepted) == n0, f"[{label}] rejected descriptor started a job")
    n_ok = sum(1 for c in cases if c[2])
    dut._log.info(f"STRIP_EN={strip_en}: {len(cases)} descriptors ({n_ok} legal, {len(cases) - n_ok} rejected; "
                  f"expectations from golden validate); "
                  f"{ck.verify()} responses golden-exact")
    tb.check_errors()


@cocotb.test()
async def test_poll_states_counters_selftest(dut):
    """POLL idle -> busy -> done (held until the next RUN_GEMM); RUN_GEMM while running -> BUSY;
    READ_COUNTERS returns the snapshot taken at ctr_snap, 72 B in index order; SELFTEST
    response len 10 = pass_full, pass_strip, fletcher_full, fletcher_strip."""
    tb, ck = await setup(dut, "test_poll_states_counters_selftest", busy_len=(3000, 3000),
                         accept_delay=(5, 5))
    f_poll, f_run, f_ctr, f_st = reqframes([(POLL, b""), (RUN_GEMM, desc()), (READ_COUNTERS, b""),
                                            (SELFTEST, b"")])
    _, r = await tb.xact(f_poll)
    ck.add("poll idle", f_poll, r, ACK, bytes([POLL_IDLE]))
    _, r = await tb.xact(f_run)
    ck.add("run", f_run, r, ACK)
    _, r = await tb.xact(f_poll)
    ck.add("poll busy", f_poll, r, ACK, bytes([POLL_BUSY]))
    _, r = await tb.xact(f_run)
    ck.add("run while running", f_run, r, BUSY)
    _, r = await tb.xact(f_st)
    ck.add("selftest while running", f_st, r, BUSY)
    _, r = await tb.xact(f_ctr)
    ck.add("counters while running", f_ctr, r, ACK, b"".join(le(v, 6) for v in tb.snaps[-1]))
    check(len(r) == 5 + 72, "READ_COUNTERS length", got=len(r))
    await wait_seq_idle(tb)
    for _ in range(3):
        _, r = await tb.xact(f_poll)
        ck.add("poll done (held)", f_poll, r, ACK, bytes([POLL_DONE]))
        await ClockCycles(dut.clk, 500)
    _, r = await tb.xact(f_ctr)
    ck.add("counters after done", f_ctr, r, ACK, b"".join(le(v, 6) for v in tb.snaps[-1]))
    pf, ps, ff, fs = tb.st_vals
    _, r = await tb.xact(f_st)
    ck.add("selftest", f_st, r, ACK, le(pf, 1) + le(ps, 1) + le(ff, 4) + le(fs, 4))
    check(tb.st_starts == 1, "st_start pulses", got=tb.st_starts)
    tb.st_vals = (0, 1, 0xFFFF0000, 0x0000FFFF)
    dut.st_pass_full.value, dut.st_pass_strip.value = 0, 1
    dut.st_fl_full.value, dut.st_fl_strip.value = 0xFFFF0000, 0x0000FFFF
    _, r = await tb.xact(f_st)
    ck.add("selftest 2", f_st, r, ACK, le(0, 1) + le(1, 1) + le(0xFFFF0000, 4) + le(0x0000FFFF, 4))
    # a new RUN_GEMM clears DONE (busy again)
    tb.busy_len = (2000, 2000)
    _, r = await tb.xact(f_run)
    ck.add("run 2", f_run, r, ACK)
    _, r = await tb.xact(f_poll)
    ck.add("poll busy 2", f_poll, r, ACK, bytes([POLL_BUSY]))
    ck.verify()
    tb.check_errors()


@cocotb.test()
async def test_timeouts_truncation_oversize(dut):
    """Truncated frames (cut at every header position and mid-payload) are dropped silently
    after RX_TIMEOUT idle; the next frame works and nothing was written. A len > MAX_PAYLOAD
    frame is dropped and answered NACK_RANGE only after the line idles RX_TIMEOUT.
    Inter-byte gaps below the timeout are tolerated."""
    tb, ck = await setup(dut, "test_timeouts_truncation_oversize")
    rng = tb.rng
    payload_ping = le(pkg_const("PING_MAGIC_LE"), 4) + le(pkg_const("PROTO_VERSION"), 1) + le(BUILD_ID, 4)
    data = bytes(rng.randint(0, 255) for _ in range(40))
    f_w, f_ping = reqframes([(WRITE_MEM, le(1, 1) + le(100, 4) + data), (PING, b"")])
    tb.cmd_region = 1
    for cut in [1, 2, 3, 4, 5, 9, 20, len(f_w) - 1]:
        tb.send(f_w[:cut])
        await tb.quiet(TIMEOUT + 50)
        _, r = await tb.xact(f_ping)
        ck.add(f"ping after truncation at {cut}", f_ping, r, ACK, payload_ping)
    check(tb.n_writes == 0, "truncated WRITE_MEM wrote memory", writes=tb.n_writes)
    # gaps just below the timeout inside a frame are fine
    for i in range(3):
        pos = rng.randint(1, len(f_w) - 1)
        tb.send(f_w[:pos], gap_max=0)
        tb.rxq.extend([None] * (TIMEOUT - 10))
        tb.send(f_w[pos:], gap_max=0)
        _, r = await tb.response(max_cycles=4 * TIMEOUT)
        ck.add(f"write with gap {TIMEOUT - 10} at {pos}", f_w, r, ACK)
    tb.apply_write(1, 100, data)
    tb.check_mem("gap writes")
    # oversize len: dropped, NACK_RANGE after idle
    for ln in (pkg_const("MAX_PAYLOAD") + 1, 0xFFFF):
        f_big, = reqframes([(WRITE_MEM, bytes(ln))]) if ln < 4096 else [bytes([0xA5, WRITE_MEM]) + le(ln, 2)]
        head = f_big[:4 + 300]
        tb.send(head, gap_max=3)
        # a PING inside the dropped stream must not be answered
        tb.send(f_ping, gap_max=0)
        start, r = await tb.response(max_cycles=20 * TIMEOUT)
        ck.add(f"oversize len {ln}", head, r, NACK_RANGE)
        check(start - tb.last_rx_cycle >= TIMEOUT - 1, "oversize NACK sent before the idle timeout",
              nack_cycle=start, last_rx=tb.last_rx_cycle, timeout=TIMEOUT)
        _, r = await tb.xact(f_ping)
        ck.add("ping after oversize", f_ping, r, ACK, payload_ping)
    ck.verify()
    tb.check_errors()


@cocotb.test()
async def test_back_to_back_and_random(dut):
    """scaled(10000) random frames, each sent the cycle after the previous response ends:
    WRITE_MEM / READ_MEM (any region, unaligned, 1..1019 / 1..1024 B; a few >= 1000 B),
    PING, POLL, READ_COUNTERS, SET_DISPLAY, unknown opcode, bad CRC. Memory scoreboard;
    every response golden-exact; 0 errors."""
    tb, ck = await setup(dut, "test_back_to_back_and_random", busy_len=(1, 1))
    rng = tb.rng
    n = scaled(10000)
    ops = []
    for i in range(n):
        r = rng.random()
        big = rng.random() < 0.004
        reg = rng.randint(0, 3)
        nbytes, _ = tb.regions[reg]
        if r < 0.32:
            ln = rng.randint(1000, 1019) if big else rng.randint(1, 24)
            ln = min(ln, nbytes)
            addr = rng.randint(0, nbytes - ln)
            ops.append(("w", WRITE_MEM, le(reg, 1) + le(addr, 4) + bytes(rng.randint(0, 255) for _ in range(ln))))
        elif r < 0.58:
            ln = rng.randint(1000, 1024) if big else rng.randint(1, 24)
            ln = min(ln, nbytes)
            addr = rng.randint(0, nbytes - ln)
            ops.append(("r", READ_MEM, le(reg, 1) + le(addr, 4) + le(ln, 2)))
        elif r < 0.68:
            ops.append(("ping", PING, b""))
        elif r < 0.75:
            ops.append(("poll", POLL, b""))
        elif r < 0.79:
            ops.append(("ctr", READ_COUNTERS, b""))
        elif r < 0.83:
            ops.append(("disp", SET_DISPLAY, le(rng.randint(0, 1000), 2) + le(rng.randint(0, 1), 1)))
        elif r < 0.87:
            ops.append(("unk", rng.choice([op for op in range(256) if op not in KNOWN_OPS]),
                        bytes(rng.randint(0, 255) for _ in range(rng.randint(0, 6)))))
        else:
            ln = rng.randint(1, 24)
            addr = rng.randint(0, nbytes - ln)
            ops.append(("bad", WRITE_MEM, le(reg, 1) + le(addr, 4) + bytes(rng.randint(0, 255) for _ in range(ln))))
    frames = reqframes([(op, p) for (_, op, p) in ops])
    payload_ping = le(pkg_const("PING_MAGIC_LE"), 4) + le(pkg_const("PROTO_VERSION"), 1) + le(BUILD_ID, 4)
    big_frames = 0
    for (kind, op, p), f in zip(ops, frames):
        if kind == "bad":
            f = corrupt(f, rng)
        if len(p) >= 1000 or (kind == "r" and int.from_bytes(p[5:7], "little") >= 1000):
            big_frames += 1
        tb.cmd_region = p[0] if kind in ("w", "r") else None
        _, r = await tb.xact(f, gap_max=1, gap_p=0.05)
        if kind == "w":
            addr = int.from_bytes(p[1:5], "little")
            tb.apply_write(p[0], addr, p[5:])
            ck.add("rand write", f, r, ACK)
        elif kind == "r":
            addr = int.from_bytes(p[1:5], "little")
            ln = int.from_bytes(p[5:7], "little")
            ck.add("rand read", f, r, ACK, tb.exp_mem[p[0]][addr:addr + ln])
        elif kind == "ping":
            ck.add("rand ping", f, r, ACK, payload_ping)
        elif kind == "poll":
            ck.add("rand poll", f, r, ACK, bytes([POLL_IDLE]))
        elif kind == "ctr":
            ck.add("rand counters", f, r, ACK, b"".join(le(v, 6) for v in tb.snaps[-1]))
        elif kind == "disp":
            ck.add("rand display", f, r, ACK)
            check(tb.disp and tb.disp[-1] == (int.from_bytes(p[:2], "little"), p[2]), "disp outputs",
                  got=tb.disp[-1:], expected=(int.from_bytes(p[:2], "little"), p[2]))
        elif kind == "unk":
            ck.add("rand unknown op", f, r, NACK_OPCODE)
        else:
            ck.add("rand bad crc", f, r, NACK_CRC)
    tb.check_mem("random")
    nv = ck.verify()
    tb.check_errors()
    await tb.quiet(50)
    dut._log.info(f"{n} random frames back to back ({big_frames} with >= 1000 B payload), "
                  f"{tb.n_writes} mem writes, {tb.n_reads} mem reads: {nv} responses golden-exact, "
                  f"memory image exact, 0 errors")


@cocotb.test()
async def test_poll_error_priority(dut):
    """POLL priority with the seq_error level (reserved; core ties it to 0 in the MVP, so
    only the packet unit can exercise it): busy > error > done > idle (packet.sv header,
    SPEC §3.8 POLL states)."""
    tb, ck = await setup(dut, "test_poll_error_priority", busy_len=(3000, 3000), accept_delay=(5, 5))
    POLL_ERROR = 3
    f_poll, f_run = reqframes([(POLL, b""), (RUN_GEMM, desc())])
    dut.seq_error.value = 1
    _, r = await tb.xact(f_poll)
    ck.add("error, not done", f_poll, r, ACK, bytes([POLL_ERROR]))
    _, r = await tb.xact(f_run)
    ck.add("run", f_run, r, ACK)
    _, r = await tb.xact(f_poll)
    ck.add("busy beats error", f_poll, r, ACK, bytes([POLL_BUSY]))
    await wait_seq_idle(tb)
    _, r = await tb.xact(f_poll)
    ck.add("error beats done", f_poll, r, ACK, bytes([POLL_ERROR]))
    dut.seq_error.value = 0
    _, r = await tb.xact(f_poll)
    ck.add("done after error clears", f_poll, r, ACK, bytes([POLL_DONE]))
    ck.verify()
    tb.check_errors()


@cocotb.test()
async def test_rx_timeout_exact_edge(dut):
    """D-019 item 3: with frame-byte strobes at t and t+g the frame continues iff
    g <= RX_TIMEOUT; otherwise the parser resets (the rest of the frame is ignored).
    Checked at g = RX_TIMEOUT (continues) and g = RX_TIMEOUT + 1 (reset) at several
    split points, including mid-payload of a WRITE_MEM (no memory side effect)."""
    tb, ck = await setup(dut, "test_rx_timeout_exact_edge")
    payload_ping = le(pkg_const("PING_MAGIC_LE"), 4) + le(pkg_const("PROTO_VERSION"), 1) + le(BUILD_ID, 4)
    data = bytes(range(1, 17))
    f_ping, f_w = reqframes([(PING, b""), (WRITE_MEM, le(1, 1) + le(200, 4) + data)])
    tb.cmd_region = 1
    for f, name, pos_list in ((f_ping, "PING", (1, 2, 4)), (f_w, "WRITE_MEM", (1, 3, 7, 12))):
        for pos in pos_list:
            # g = RX_TIMEOUT: TIMEOUT-1 idle cycles between the two strobes -> continues
            tb.send(f[:pos], gap_max=0)
            tb.rxq.extend([None] * (TIMEOUT - 1))
            tb.send(f[pos:], gap_max=0)
            _, r = await tb.response(max_cycles=4 * TIMEOUT)
            if name == "PING":
                ck.add(f"{name} split {pos} g=RX_TIMEOUT", f, r, ACK, payload_ping)
            else:
                ck.add(f"{name} split {pos} g=RX_TIMEOUT", f, r, ACK)
                tb.apply_write(1, 200, data)
                tb.check_mem(f"g=RX_TIMEOUT split {pos}")
            # g = RX_TIMEOUT + 1: parser resets; the tail is not a frame -> no response
            n_w = tb.n_writes
            tb.mem[1][200:200 + len(data)] = bytes(len(data))       # detect any new write
            tb.exp_mem[1][200:200 + len(data)] = bytes(len(data))
            tb.send(f[:pos], gap_max=0)
            tb.rxq.extend([None] * TIMEOUT)
            tail = f[pos:]
            check(0xA5 not in tail, "test construction: tail contains a sync byte", tail=tail.hex())
            tb.send(tail, gap_max=0)
            await tb.quiet(2 * TIMEOUT)
            check(tb.n_writes == n_w, f"[{name} split {pos} g=RX_TIMEOUT+1] reset frame wrote memory")
            _, r = await tb.xact(f_ping)
            ck.add(f"ping after {name} split {pos} g=RX_TIMEOUT+1", f_ping, r, ACK, payload_ping)
    ck.verify()
    tb.check_errors()


@cocotb.test()
async def test_strict_request_response_discard(dut):
    """D-019 item 2: bytes that arrive while a command executes or while its response is
    being sent are discarded (whole frames included): exactly one response per accepted
    request, discarded WRITE_MEMs have no side effect, and the link works afterwards."""
    tb, ck = await setup(dut, "test_strict_request_response_discard")
    payload_ping = le(pkg_const("PING_MAGIC_LE"), 4) + le(pkg_const("PROTO_VERSION"), 1) + le(BUILD_ID, 4)
    big = bytes((i * 7) & 0xFF for i in range(1000))
    junk = bytes(range(1, 9))
    f_ping, f_wbig, f_wjunk = reqframes([(PING, b""), (WRITE_MEM, le(2, 1) + le(0, 4) + big),
                                         (WRITE_MEM, le(2, 1) + le(4000, 4) + junk)])
    tb.cmd_region = 2
    # 1) during execution: a 1000-byte WRITE_MEM executes for ~2000 cycles; a whole
    #    WRITE_MEM frame and a PING sent right behind it must be discarded.
    tb.send(f_wbig, gap_max=0)
    tb.send(f_wjunk, gap_max=0)
    tb.send(f_ping, gap_max=0)
    _, r = await tb.response()
    ck.add("big write (others discarded during execution)", f_wbig, r, ACK)
    tb.apply_write(2, 0, big)
    await tb.quiet(3 * TIMEOUT)
    tb.check_mem("discard during execution")
    # 2) during the response: hold tx_ready low so the PING response takes long, and send
    #    a full WRITE_MEM + PING while it is being transmitted.
    tb.send(f_ping, gap_max=0)
    while not tb.cur:                                  # wait for the first response byte
        await ClockCycles(dut.clk, 1)
    tb.bp_left = 400
    tb.send(f_wjunk, gap_max=0)
    tb.send(f_ping, gap_max=0)
    _, r = await tb.response()
    ck.add("ping (others discarded during response)", f_ping, r, ACK, payload_ping)
    await tb.quiet(3 * TIMEOUT)
    tb.check_mem("discard during response")
    _, r = await tb.xact(f_ping)
    ck.add("ping afterwards", f_ping, r, ACK, payload_ping)
    ck.verify()
    tb.check_errors()
