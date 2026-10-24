"""cocotb unit tests for rtl/uart_tx.sv and rtl/uart_rx.sv (SPEC v1.1 §3.8, §6 "UART").

Expected values are the bytes themselves (loopback / bit-banged identity); bit
timing comes from the package (BAUD_DIV, OVERSAMPLE) or the sim-only DIV_OVERRIDE
of the wrapper (tb/uart/uart_tb.sv, DIV_F). No golden vectors are needed.
"""

from __future__ import annotations

import cocotb
from cocotb.triggers import ClockCycles, FallingEdge, First, RisingEdge
from cocotb.utils import get_sim_time

from splitsys_tb import CLK_PERIOD_NS, check, pkg_const, reset, scaled, seed_rng, start_clock, uval

DIV_F = 2   # must match uart_tb DIV_F default


def bit_f() -> int:
    return DIV_F * pkg_const("OVERSAMPLE")


def now_cyc() -> int:
    return int(get_sim_time(unit="ns")) // CLK_PERIOD_NS


async def setup(dut):
    for s in ("f_tx_data", "f_tx_valid", "s_tx_data", "s_tx_valid"):
        getattr(dut, s).value = 0
    dut.f_loop.value = 1
    dut.f_rxd_ext.value = 1
    await start_clock(dut)
    await reset(dut)
    await ClockCycles(dut.clk, 5)


class RxMon:
    """Collects (cycle, byte) on rx_valid and cycles of rx_frame_err; checks exclusivity."""

    def __init__(self, dut, prefix="f"):
        self.dut, self.p = dut, prefix
        self.bytes, self.errs = [], []
        cocotb.start_soon(self._run())

    async def _run(self):
        v = getattr(self.dut, f"{self.p}_rx_valid")
        e = getattr(self.dut, f"{self.p}_rx_frame_err")
        d = getattr(self.dut, f"{self.p}_rx_data")
        while True:
            await _first_rise(v, e)
            await FallingEdge(self.dut.clk)
            vv, ee = uval(v), uval(e)
            check(not (vv and ee), f"[{self.p}] rx_valid and rx_frame_err in the same cycle",
                  cycle=now_cyc())
            if vv:
                self.bytes.append(uval(d))
            if ee:
                self.errs.append(now_cyc())


async def _first_rise(a, b):

    await First(RisingEdge(a), RisingEdge(b))


async def send_tx(dut, data, rng=None, gap_p=0.0, prefix="f"):
    """Producer on the tx valid/ready handshake; returns accepted bytes in order."""
    clk = dut.clk
    tv, td, tr = (getattr(dut, f"{prefix}_tx_{n}") for n in ("valid", "data", "ready"))
    for b in data:
        if rng is not None and rng.random() < gap_p:
            await ClockCycles(clk, rng.randint(1, 3 * bit_f()))
        await RisingEdge(clk)
        td.value = b
        tv.value = 1
        await FallingEdge(clk)
        if not uval(tr):
            await RisingEdge(tr)  # event-driven wait (no per-cycle Python)
            await FallingEdge(clk)
        check(uval(tr) == 1, "tx_ready dropped while valid held", cycle=now_cyc())
        await RisingEdge(clk)     # accepted at this edge (valid && ready)
        tv.value = 0


async def drive_rx_bits(dut, byte, bit_cyc, err=0.0, stop=1):
    """Bit-bang one 8N1 frame on f_rxd_ext; err = relative baud error (+0.03 = 3% slow)."""
    bits = [0] + [(byte >> i) & 1 for i in range(8)] + [stop]
    t = 0.0
    done = 0
    for b in bits:
        dut.f_rxd_ext.value = b
        t += bit_cyc * (1.0 + err)
        n = int(round(t)) - done
        await ClockCycles(dut.clk, n, rising=False)
        done += n
    dut.f_rxd_ext.value = 1


async def decode_tx(dut, n_frames, bit_cyc, txd_name="f_txd"):
    """Decode frames on txd, checking every bit at its first, middle and last cycle.
    Returns (bytes, start cycles)."""
    txd = getattr(dut, txd_name)
    out, starts = [], []
    for _ in range(n_frames):
        await FallingEdge(txd)
        t0 = now_cyc()
        starts.append(t0)
        val = 0
        off = 0
        for k in range(10):
            samples = []
            for j in (0, bit_cyc // 2, bit_cyc - 1):
                tgt = k * bit_cyc + j
                if tgt > off:
                    await ClockCycles(dut.clk, tgt - off, rising=False)
                    off = tgt
                elif tgt == off and off == 0:
                    await FallingEdge(dut.clk)
                samples.append(uval(txd))
            check(len(set(samples)) == 1, f"[{txd_name}] bit {k} not stable for exactly one bit time",
                  frame_start_cycle=t0, samples_first_mid_last=samples, bit_cyc=bit_cyc)
            bit = samples[0]
            if k == 0:
                check(bit == 0, "start bit", got=bit, frame_start_cycle=t0)
            elif k == 9:
                check(bit == 1, "stop bit", got=bit, frame_start_cycle=t0)
            else:
                val |= bit << (k - 1)
        out.append(val)
    return out, starts


@cocotb.test()
async def test_tx_frame_format_and_back_to_back(dut):
    """uart_tx: 8N1 LSB first, each bit exactly DIV*OVERSAMPLE clocks, a byte accepted only
    on valid&&ready, back-to-back frames with start-to-start <= 10 bits + 2 clocks."""
    await setup(dut)
    rng = seed_rng(dut, "test_tx_frame_format_and_back_to_back")
    data = [0x00, 0xFF, 0x55, 0xAA, 0x01, 0x80] + [rng.randint(0, 255) for _ in range(scaled(200))]
    bit = bit_f()
    dec = cocotb.start_soon(decode_tx(dut, len(data), bit))
    await send_tx(dut, data)
    got, starts = await dec
    check(got == data, "tx bytes", first_mismatch=next((i for i, (a, b) in enumerate(zip(got, data)) if a != b), None))
    periods = sorted({b - a for a, b in zip(starts, starts[1:])})
    dut._log.info(f"{len(data)} frames, bit={bit} clk, start-to-start periods {periods}")
    check(periods[0] >= 10 * bit and periods[-1] <= 10 * bit + 2,
          "back-to-back frame spacing", periods=periods, ten_bits=10 * bit)


@cocotb.test()
async def test_rx_baud_mismatch_and_gaps(dut):
    """uart_rx fed by a bit-banged line: nominal, +-3% baud error, random idle gaps incl. 0."""
    await setup(dut)
    dut.f_loop.value = 0
    rng = seed_rng(dut, "test_rx_baud_mismatch_and_gaps")
    mon = RxMon(dut)
    sent = []
    for err in (0.0, 0.03, -0.03, 0.015, -0.015):
        for _ in range(scaled(60)):
            b = rng.randint(0, 255)
            await drive_rx_bits(dut, b, bit_f(), err)
            sent.append(b)
            if rng.random() < 0.5:
                await ClockCycles(dut.clk, rng.randint(1, 2 * bit_f()))
    await ClockCycles(dut.clk, 2 * bit_f())
    check(mon.bytes == sent, "rx bytes", n_sent=len(sent), n_got=len(mon.bytes),
          first_mismatch=next((i for i, (a, b) in enumerate(zip(mon.bytes, sent)) if a != b), None))
    check(not mon.errs, "unexpected frame errors", at=mon.errs[:5])
    dut._log.info(f"{len(sent)} bytes received over baud errors 0/+-1.5%/+-3%")


@cocotb.test()
async def test_rx_frame_error_and_glitch(dut):
    """Stop bit low -> rx_frame_err pulse, byte discarded; a low glitch < half bit -> nothing.
    The receiver recovers: following good bytes are received exactly."""
    await setup(dut)
    dut.f_loop.value = 0
    mon = RxMon(dut)
    bit = bit_f()
    await drive_rx_bits(dut, 0x3C, bit, stop=0)
    await ClockCycles(dut.clk, 3 * bit)
    check(len(mon.errs) == 1 and mon.bytes == [], "frame error",
          frame_err_pulses=len(mon.errs), bytes=mon.bytes)
    # glitch: low for a quarter bit
    dut.f_rxd_ext.value = 0
    await ClockCycles(dut.clk, bit // 4)
    dut.f_rxd_ext.value = 1
    await ClockCycles(dut.clk, 2 * bit)
    check(uval(dut.f_rx_active) == 0, "rx_active after glitch")
    check(mon.bytes == [] and len(mon.errs) == 1, "glitch produced output",
          bytes=mon.bytes, errs=len(mon.errs))
    for b in (0xA5, 0x5A, 0x00, 0xFF):
        await drive_rx_bits(dut, b, bit)
    await ClockCycles(dut.clk, 2 * bit)
    check(mon.bytes == [0xA5, 0x5A, 0x00, 0xFF], "recovery bytes", got=mon.bytes)


@cocotb.test()
async def test_loopback_random(dut):
    """tx -> rx loopback at the fast sim divisor: scaled(10000) random bytes, random
    producer gaps, zero errors."""
    await setup(dut)
    rng = seed_rng(dut, "test_loopback_random")
    mon = RxMon(dut)
    data = [rng.randint(0, 255) for _ in range(scaled(10000))]
    await send_tx(dut, data, rng, gap_p=0.05)
    await ClockCycles(dut.clk, 25 * bit_f())   # held byte + shifting byte + rx latency
    check(mon.bytes == data, "loopback bytes", n_sent=len(data), n_got=len(mon.bytes),
          first_mismatch=next((i for i, (a, b) in enumerate(zip(mon.bytes, data)) if a != b), None))
    check(not mon.errs, "frame errors in loopback", n=len(mon.errs))
    dut._log.info(f"{len(data)} bytes looped back, 0 errors")


@cocotb.test()
async def test_spec_divisor_baud(dut):
    """DIV_OVERRIDE = 0 uses splitsys_pkg::BAUD_DIV: BAUD_DIV == round(CLK_HZ/(BAUD*OVERSAMPLE)),
    bit period measured == BAUD_DIV*OVERSAMPLE clocks (baud error < 1%), loopback exact."""
    await setup(dut)
    clk_hz, baud, os_ = pkg_const("CLK_HZ"), pkg_const("BAUD"), pkg_const("OVERSAMPLE")
    div = pkg_const("BAUD_DIV")
    check(div == round(clk_hz / (baud * os_)), "BAUD_DIV", div=div, expected=round(clk_hz / (baud * os_)))
    actual = clk_hz / (div * os_)
    check(abs(actual - baud) / baud < 0.01, "baud error", actual=actual, baud=baud)
    bit = div * os_
    mon = RxMon(dut, "s")
    data = [0x00, 0xA5, 0x5A, 0xFF, 0x13, 0x37]
    dec = cocotb.start_soon(decode_tx(dut, len(data), bit, "s_txd"))
    await send_tx(dut, data, prefix="s")
    got, starts = await dec
    await ClockCycles(dut.clk, 2 * bit)
    check(got == data, "spec-divisor tx decode", got=got)
    check(mon.bytes == data and not mon.errs, "spec-divisor loopback", got=mon.bytes, errs=mon.errs)
    dut._log.info(f"BAUD_DIV={div}, bit={bit} clk, actual baud {actual:.1f} vs {baud}")
