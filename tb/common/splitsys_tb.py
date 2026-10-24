"""Shared cocotb helpers for SplitSys unit tests (owner: verification-engineer).

Rules this module enforces for every test:
  * Expected numerics come only from Rust (`splitsys_golden` via tb/oracle, or
    sw/crates/vec output). Nothing here computes a numeric expectation.
  * Every randomized test derives its RNG from SEED (env) + test name, and logs it.
  * Latency constants are read from the generated package rtl/gen/splitsys_pkg.sv.

Timing convention used by all tests ("cycle c"):
  await RisingEdge(clk)  -> cycle c begins, registers hold their post-edge value
  drive inputs for cycle c
  await FallingEdge(clk) -> sample outputs of cycle c
So a register stage of delay D shows input-of-cycle-t in cycle t+D, and a
combinational path shows it in cycle t.
"""

from __future__ import annotations

import os
import random
import re
import subprocess
import zlib
from functools import lru_cache
from pathlib import Path

import cocotb
from cocotb.clock import Clock
from cocotb.triggers import FallingEdge, RisingEdge

REPO_ROOT = Path(os.environ.get("SPLITSYS_REPO_ROOT", Path(__file__).resolve().parents[2]))
RTL_DIR = Path(os.environ.get("SPLITSYS_RTL_DIR", REPO_ROOT / "rtl"))
PKG_PATH = RTL_DIR / "gen" / "splitsys_pkg.sv"
CLK_PERIOD_NS = 10


# --------------------------------------------------------------------- package
@lru_cache(maxsize=None)
def pkg() -> dict[str, int]:
    """Integer localparams of splitsys_pkg (decimal or sized hex/bin/dec literals)."""
    text = PKG_PATH.read_text()
    out: dict[str, int] = {}
    pat = re.compile(r"localparam\s+[^=;]*?\b([A-Z][A-Z0-9_]*)\s*=\s*([^;]+);")
    for name, val in pat.findall(text):
        val = val.strip()
        if val.startswith("{") and val.endswith("}"):
            # concatenation of sized literals, MSB first: {8'd4, 8'd4, ...}
            acc = 0
            ok = True
            for item in val[1:-1].split(","):
                mi = re.fullmatch(r"\s*(\d+)'([hdbHDB])([0-9a-fA-F_]+)\s*", item)
                if not mi:
                    ok = False
                    break
                w = int(mi.group(1))
                b = {"h": 16, "d": 10, "b": 2}[mi.group(2).lower()]
                acc = (acc << w) | (int(mi.group(3).replace("_", ""), b) & ((1 << w) - 1))
            if ok:
                out[name] = acc
            continue
        m = re.fullmatch(r"(\d+)?'([hdbHDB])([0-9a-fA-F_]+)", val)
        if m:
            base = {"h": 16, "d": 10, "b": 2}[m.group(2).lower()]
            out[name] = int(m.group(3).replace("_", ""), base)
        elif re.fullmatch(r"\d+", val):
            out[name] = int(val)
        elif val in ("1'b1", "1'b0"):
            out[name] = int(val[-1])
    return out


def pkg_const(name: str) -> int:
    p = pkg()
    if name not in p:
        raise AssertionError(f"constant {name} not found in {PKG_PATH}")
    return p[name]


# ------------------------------------------------------------------ randomness
def seed_rng(dut, test_name: str) -> random.Random:
    base = int(os.environ.get("SEED", "1"))
    derived = (base * 0x9E3779B1 + zlib.crc32(test_name.encode())) & 0xFFFFFFFF
    dut._log.info(f"SEED={base} (test {test_name}, derived rng seed {derived:#010x}); "
                  f"reproduce: SEED={base} COCOTB_TEST_FILTER={test_name}")
    return random.Random(derived)


def scaled(n: int) -> int:
    return max(1, int(round(n * float(os.environ.get("SCALE", "1")))))


def rand_i8(rng: random.Random) -> int:
    """int8 with extra weight on the extremes (-128, -1, 0, 1, 127)."""
    r = rng.random()
    if r < 0.15:
        return rng.choice((-128, -127, -1, 0, 1, 126, 127))
    return rng.randint(-128, 127)


# ---------------------------------------------------------------------- oracle
def _oracle_bin() -> str:
    env = os.environ.get("SPLITSYS_ORACLE")
    if env:
        return env
    cache = os.environ.get("SPLITSYS_CACHE", os.path.expanduser("~/.cache/splitsys"))
    return os.path.join(cache, "tb-oracle-target", "release", "splitsys-tb-oracle")


def oracle(lines: list[str]) -> list[str]:
    """Batch query of tb/oracle (thin CLI over splitsys_golden). One answer per line."""
    if not lines:
        return []
    proc = subprocess.run([_oracle_bin()], input="\n".join(lines) + "\n",
                          capture_output=True, text=True, check=False)
    if proc.returncode != 0:
        raise AssertionError(f"oracle failed (rc={proc.returncode}): {proc.stderr[-2000:]}")
    out = proc.stdout.splitlines()
    if len(out) != len(lines):
        raise AssertionError(f"oracle returned {len(out)} lines for {len(lines)} requests")
    return out


def golden_gemm(m: int, k: int, n: int, a: list[int], w: list[int]) -> list[int]:
    return [int(x) for x in oracle([gemm_req(m, k, n, a, w)])[0].split()]


def gemm_req(m: int, k: int, n: int, a: list[int], w: list[int]) -> str:
    assert len(a) == m * k and len(w) == k * n
    return f"gemm {m} {k} {n} " + " ".join(map(str, a)) + " " + " ".join(map(str, w))


# ---------------------------------------------------------------- clock/reset
async def start_clock(dut, clk_name: str = "clk"):
    clk = getattr(dut, clk_name)
    cocotb.start_soon(Clock(clk, CLK_PERIOD_NS, unit="ns").start())
    return clk


async def reset(dut, cycles: int = 3, clk_name: str = "clk", rst_name: str = "rst"):
    clk = getattr(dut, clk_name)
    rst = getattr(dut, rst_name)
    await RisingEdge(clk)
    rst.value = 1
    for _ in range(cycles):
        await RisingEdge(clk)
    rst.value = 0


def sval(sig) -> int:
    """Signed integer value of a handle."""
    v = sig.value
    return v.to_signed() if hasattr(v, "to_signed") else int(v)


def uval(sig) -> int:
    v = sig.value
    return v.to_unsigned() if hasattr(v, "to_unsigned") else int(v)


class CycleFail(AssertionError):
    pass


def check(cond: bool, what: str, **ctx) -> None:
    """Fail with a minimal, structured description (inputs/expected/got/cycle/where)."""
    if not cond:
        detail = ", ".join(f"{k}={v}" for k, v in ctx.items())
        raise CycleFail(f"{what}: {detail}")


__all__ = [
    "pkg", "pkg_const", "seed_rng", "scaled", "rand_i8", "oracle", "golden_gemm", "gemm_req",
    "start_clock", "reset", "sval", "uval", "check", "CycleFail", "RisingEdge", "FallingEdge",
]
