#!/usr/bin/env python3
"""SplitSys system testbench (owner: verification-engineer).

Drives the simulated `top` through its REAL UART pins: each worker starts one
tb/cosim harness (Verilator `top` + TCP bridge that serializes bytes onto
UART_RXD and decodes UART_TXD), then talks the SPEC §3.8 protocol over TCP.

Expected values come only from Rust:
  * request/response frames (sync, len, CRC-8): tb/oracle `reqframe`/`rspframe`
    (= splitsys_golden::packet); every received frame must be byte-identical to
    the Rust-built frame for the status/payload it carries.
  * GEMM outputs: splitsys-vec job JSON `expected.obuf_hex`, compared on
    `expected.compare_ranges` only (D-006).
  * counters: `splitsys-cycle predict <job.json>` (all 12, exact).
  * self-test: sw/crates/vec/out/selftest_expected.json (Fletcher values).
Python only builds payload byte layouts (region, address, length) and applies
the SPEC §3.7 invariants.

Suites (--suites, comma separated): full, strip, directed, modeswitch, selftest.
Usage (normally via tb/system/Makefile):
  system_test.py --harness <Vtop> --vec <splitsys-vec> --cycle <splitsys-cycle>
                 --seed 1 --n-full 200 --n-strip 100 --workers 16 --suites full,...
"""
from __future__ import annotations

import argparse
import json
import os
import socket
import subprocess
import sys
import threading
import time
import traceback
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "common"))
import re  # noqa: E402

from splitsys_tb import PKG_PATH, oracle, pkg  # noqa: E402  (shared helpers, read-only)

REPO = Path(__file__).resolve().parents[2]


def _enum_members() -> dict[str, int]:
    """typedef enum members of splitsys_pkg (`OP_PING = 8'h01,`); splitsys_tb.pkg() covers localparams."""
    return {n: int(v, 16) for n, v in re.findall(r"\b([A-Z][A-Z0-9_]*)\s*=\s*8'h([0-9a-fA-F]+)\s*[,}]",
                                                  PKG_PATH.read_text())}


_ENUMS = _enum_members()


def pkg_const(name: str) -> int:
    p = pkg()
    if name in p:
        return p[name]
    if name in _ENUMS:
        return _ENUMS[name]
    raise AssertionError(f"constant {name} not found in {PKG_PATH}")
VEC_OUT = REPO / "sw" / "crates" / "vec" / "out"

OP = {k: pkg_const(f"OP_{k}") for k in
      ("PING", "WRITE_MEM", "READ_MEM", "RUN_GEMM", "POLL", "READ_COUNTERS", "SELFTEST", "SET_DISPLAY")}
ST = {k: pkg_const(f"ST_{k}") for k in ("ACK", "NACK_CRC", "NACK_OPCODE", "NACK_RANGE", "BUSY")}
POLL_IDLE, POLL_BUSY, POLL_DONE = (pkg_const(x) for x in ("POLL_IDLE", "POLL_BUSY", "POLL_DONE"))
REGION = {"WBUF": pkg_const("REGION_WBUF"), "ABUF": pkg_const("REGION_ABUF"),
          "OBUF": pkg_const("REGION_OBUF"), "RQPARAM": pkg_const("REGION_RQPARAM")}
MAX_PAYLOAD = pkg_const("MAX_PAYLOAD")
WMAX = MAX_PAYLOAD - 5                     # WRITE_MEM data bytes per frame
L_ACCEPT, L_DONE = pkg_const("L_ACCEPT"), pkg_const("L_DONE")
N_COUNTERS, COUNTER_BYTES = pkg_const("N_COUNTERS"), pkg_const("COUNTER_BITS") // 8
PROTO_VERSION = pkg_const("PROTO_VERSION")
CTR_NAMES = ["cycles_total", "cycles_setup", "cycles_stream", "cycles_flush", "cycles_drain",
             "cycles_stall", "mac_useful", "mac_useful_q0", "mac_useful_q1", "mac_useful_q2",
             "mac_useful_q3", "tiles"]
for i, nm in enumerate(CTR_NAMES):  # order is normative (D-007); assert it against the package
    assert pkg_const("CTR_" + nm.upper()) == i, nm

PRINT_LOCK = threading.Lock()


def log(msg: str) -> None:
    with PRINT_LOCK:
        print(msg, flush=True)


class Fail(AssertionError):
    pass


# ------------------------------------------------------------------ Rust frames
def req_frames(items: list[tuple[int, bytes]]) -> list[bytes]:
    """Request frames built by splitsys_golden::packet::request_frame (one oracle batch)."""
    out = oracle([f"reqframe {op} {p.hex() or '-'}" for op, p in items])
    return [bytes.fromhex(x) for x in out]


def check_rsp_frames(rsps: list[tuple[bytes, int, bytes]], where: str) -> None:
    """Every received frame must equal the Rust-built response frame for its status/payload."""
    if not rsps:
        return
    exp = oracle([f"rspframe {st} {p.hex() or '-'}" for _, st, p in rsps])
    for (raw, st, p), e in zip(rsps, exp):
        if raw.hex() != e:
            raise Fail(f"{where}: response frame differs from golden framing: got {raw.hex()[:80]} "
                       f"expected {e[:80]} (status {st}, len {len(p)})")


def u16(x: int) -> bytes:
    return x.to_bytes(2, "little")


def u32(x: int) -> bytes:
    return x.to_bytes(4, "little")


# ------------------------------------------------------------------ the board
class Board:
    """One harness process = one simulated board (state persists across jobs)."""

    def __init__(self, harness: str, name: str, extra: list[str]):
        self.name = name
        self.proc = subprocess.Popen([harness, "--port", "0", "--ctrl-port", "0", *extra],
                                     stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        ports = {}
        self.preamble = []
        while len(ports) < 2:
            line = self.proc.stdout.readline()
            if not line:
                raise Fail(f"{name}: harness exited early: {''.join(self.preamble)}")
            self.preamble.append(line)
            parts = line.split()
            if parts and parts[0] in ("COSIM_PORT", "COSIM_CTRL_PORT"):
                ports[parts[0]] = int(parts[1])
        self.tail: list[str] = []
        threading.Thread(target=self._drain_stdout, daemon=True).start()
        self.d = socket.create_connection(("127.0.0.1", ports["COSIM_PORT"]))
        self.d.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.d.settimeout(900)
        self.c = socket.create_connection(("127.0.0.1", ports["COSIM_CTRL_PORT"]))
        self.c.settimeout(900)
        self.cbuf = b""
        self.rsps: list[tuple[bytes, int, bytes]] = []   # pending frame checks
        self.frames = 0
        self.const = dict(zip(("ping", "poll", "ctr", "selftest"),
                              req_frames([(OP["PING"], b""), (OP["POLL"], b""),
                                          (OP["READ_COUNTERS"], b""), (OP["SELFTEST"], b"")])))

    def _drain_stdout(self):
        for line in self.proc.stdout:
            self.tail.append(line)
            if len(self.tail) > 50:
                self.tail.pop(0)

    # control channel
    def ctl(self, cmd: str) -> list[str]:
        self.c.sendall((cmd + "\n").encode())
        while b"\n" not in self.cbuf:
            chunk = self.c.recv(4096)
            if not chunk:
                raise Fail(f"{self.name}: control socket closed")
            self.cbuf += chunk
        line, self.cbuf = self.cbuf.split(b"\n", 1)
        parts = line.decode().split()
        if not parts or parts[0] != "ok":
            raise Fail(f"{self.name}: control '{cmd}' -> {line!r}")
        return parts[1:]

    def cycle(self) -> int:
        return int(self.ctl("cycle")[0])

    def led(self) -> int:
        return int(self.ctl("led")[0], 16)

    # data channel
    def _recv_exact(self, n: int) -> bytes:
        b = b""
        while len(b) < n:
            chunk = self.d.recv(n - len(b))
            if not chunk:
                raise Fail(f"{self.name}: data socket closed (harness tail: {''.join(self.tail[-5:])})")
            b += chunk
        return b

    def xact(self, frame: bytes, expect_status: int | None = ST["ACK"], what: str = "") -> tuple[int, bytes]:
        self.d.sendall(frame)
        hdr = self._recv_exact(4)
        if hdr[0] != 0x5A:
            raise Fail(f"{self.name}: {what}: bad response sync {hdr.hex()} (request {frame[:8].hex()}..)")
        ln = int.from_bytes(hdr[2:4], "little")
        if ln > MAX_PAYLOAD:
            raise Fail(f"{self.name}: {what}: response len {ln} > MAX_PAYLOAD")
        rest = self._recv_exact(ln + 1)
        raw = hdr + rest
        st, payload = hdr[1], rest[:ln]
        self.rsps.append((raw, st, payload))
        self.frames += 1
        if expect_status is not None and st != expect_status:
            raise Fail(f"{self.name}: {what}: status {st} expected {expect_status} "
                       f"(request {frame[:12].hex()}.., payload {payload.hex()[:40]})")
        return st, payload

    def flush_checks(self, where: str) -> None:
        rs, self.rsps = self.rsps, []
        check_rsp_frames(rs, f"{self.name}: {where}")

    def counters(self) -> list[int]:
        _, p = self.xact(self.const["ctr"], what="READ_COUNTERS")
        if len(p) != N_COUNTERS * COUNTER_BYTES:
            raise Fail(f"{self.name}: READ_COUNTERS len {len(p)}")
        return [int.from_bytes(p[COUNTER_BYTES * i:COUNTER_BYTES * (i + 1)], "little") for i in range(N_COUNTERS)]

    def poll(self) -> int:
        _, p = self.xact(self.const["poll"], what="POLL")
        if len(p) != 1:
            raise Fail(f"{self.name}: POLL len {len(p)}")
        return p[0]

    def close(self):
        try:
            st = self.ctl("stats")
            self.ctl("quit")
        except Exception:  # noqa: BLE001
            st = None
        try:
            self.proc.wait(timeout=30)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        return st


# ------------------------------------------------------------------ predictions
def predict(cycle_bin: str, job: dict, tmpdir: Path) -> dict:
    p = tmpdir / f"pred_{job['name']}_{threading.get_ident()}.json"
    p.write_text(json.dumps(job))
    out = subprocess.run([cycle_bin, "predict", str(p)], capture_output=True, text=True)
    if out.returncode != 0:
        raise Fail(f"splitsys-cycle predict failed for {job['name']}: {out.stderr}")
    return json.loads(out.stdout)


def model_vector(pred: dict) -> list[int]:
    m = pred["model"]
    return [m["cycles_total"], m["cycles_setup"], m["cycles_stream"], m["cycles_flush"], m["cycles_drain"],
            m["cycles_stall"], m["mac_useful"], *m["mac_useful_q"], m["tiles"]]


def check_counters(got: list[int], pred: dict, job: dict, where: str) -> None:
    exp = model_vector(pred)
    diffs = [f"{CTR_NAMES[i]}: got {g} model {e} (diff {g - e:+d})" for i, (g, e) in enumerate(zip(got, exp)) if g != e]
    M, K, N = job["M"], job["K"], job["N"]
    inv = []
    if got[6] != M * N * K:
        inv.append(f"mac_useful {got[6]} != M*N*K {M * N * K}")
    if sum(got[7:11]) != got[6]:
        inv.append(f"sum(mac_useful_q) {sum(got[7:11])} != mac_useful {got[6]}")
    if got[0] != sum(got[1:6]) + L_ACCEPT + L_DONE:
        inv.append(f"cycles_total {got[0]} != setup+stream+flush+drain+stall+L_ACCEPT+L_DONE "
                   f"{sum(got[1:6]) + L_ACCEPT + L_DONE}")
    if diffs or inv:
        raise Fail(f"{where}: COUNTERS mode={job['mode']} M={M} K={K} N={N} flags={job['desc']['flags']}: "
                   + "; ".join(diffs + ["INVARIANT " + x for x in inv])
                   + f" | latencies {pred['latencies']} provisional={pred['latencies_provisional']}")


# ------------------------------------------------------------------ output decode
def locate(job: dict, off: int) -> str:
    """Human hint for an OBUF byte offset (relative to obuf_byte_addr)."""
    M = job["M"]
    m_pad = 8 * ((M + 7) // 8)
    int32 = job["desc"]["flags"] == pkg_const("FLAGS_INT32")
    word, lane = divmod(off, 16)
    if int32:
        row_word, q = divmod(word, 4)
        nb, m = divmod(row_word, m_pad)
        n = 16 * nb + 4 * q + lane // 4
        what = f"int32 byte {lane % 4}"
    else:
        nb, m = divmod(word, m_pad)
        n = 16 * nb + lane
        what = "int8"
    if job["desc"]["mode"] == pkg_const("MODE_FULL"):
        r, c = m % 8, n % 8
        tile = f"FULL tile (mb={m // 8}, nb8={n // 8}) PE({r},{c})"
    else:
        i, cc = m % 4, n % 16
        r, c = (i, cc) if cc < 8 else (4 + i, cc - 8)
        tile = f"STRIP tile (mb4={m // 4}, nb={n // 16}) PE({r},{c})"
    q = 2 * (r // 4) + (c // 4)
    return f"C[{m},{n}] {what} -> {tile} Q{q}"


# ------------------------------------------------------------------ one job
def chunks(base: int, data: bytes, size: int):
    for i in range(0, len(data), size):
        yield base + i, data[i:i + size]


def run_job(b: Board, job: dict, pred: dict, stats: dict) -> None:
    name = job["name"]
    exp = job["expected"]
    ob_addr, ob_len = exp["obuf_byte_addr"], exp["obuf_len"]
    ob_exp = bytes.fromhex(exp["obuf_hex"])
    items: list[tuple[int, bytes]] = []
    for ld in job["load"]:
        data = bytes.fromhex(ld["hex"])
        for a, d in chunks(ld["byte_addr"], data, WMAX):
            items.append((OP["WRITE_MEM"], bytes([ld["region"]]) + u32(a) + d))
    # poison the whole C extent with the complement of the expected image, so a
    # byte the RTL never writes cannot match by leftover luck
    poison = bytes((~x) & 0xFF for x in ob_exp)
    for a, d in chunks(ob_addr, poison, WMAX):
        items.append((OP["WRITE_MEM"], bytes([REGION["OBUF"]]) + u32(a) + d))
    n_writes = len(items)
    items.append((OP["RUN_GEMM"], bytes.fromhex(job["desc_hex"])))
    reads = list(chunks(ob_addr, bytes(ob_len), MAX_PAYLOAD))
    for a, d in reads:
        items.append((OP["READ_MEM"], bytes([REGION["OBUF"]]) + u32(a) + u16(len(d))))
    frames = req_frames(items)

    for i in range(n_writes):
        _, p = b.xact(frames[i], what=f"{name} WRITE_MEM #{i}")
        if p:
            raise Fail(f"{b.name}: {name}: WRITE_MEM ACK with len {len(p)}")
    c0 = b.cycle()
    _, p = b.xact(frames[n_writes], what=f"{name} RUN_GEMM")
    if p:
        raise Fail(f"{b.name}: {name}: RUN_GEMM ACK with len {len(p)}")
    st = b.poll()
    if st not in (POLL_BUSY, POLL_DONE):
        raise Fail(f"{b.name}: {name}: POLL after accepted RUN_GEMM = {st}, expected BUSY/DONE")
    polls = 1
    while st != POLL_DONE:
        b.ctl("idle")                      # advance until LED2 (busy) drops
        st = b.poll()
        polls += 1
        if st not in (POLL_BUSY, POLL_DONE):
            raise Fail(f"{b.name}: {name}: POLL = {st} while waiting for DONE")
        if polls > 50:
            raise Fail(f"{b.name}: {name}: still BUSY after {polls} polls (cycle {b.cycle()})")
    sim_cycles = b.cycle() - c0
    got_ctr = b.counters()
    check_counters(got_ctr, pred, job, f"{b.name}: {name}")
    got = bytearray()
    for k, (a, d) in enumerate(reads):
        _, p = b.xact(frames[n_writes + 1 + k], what=f"{name} READ_MEM @{a}")
        if len(p) != len(d):
            raise Fail(f"{b.name}: {name}: READ_MEM len {len(p)} expected {len(d)}")
        got += p
    bad = []
    for off, ln in exp["compare_ranges"]:
        for o in range(off, off + ln):
            if got[o] != ob_exp[o]:
                bad.append(o)
    if bad:
        o = bad[0]
        raise Fail(f"{b.name}: {name}: OUTPUT {len(bad)} in-bounds byte(s) differ; first at obuf byte "
                   f"{ob_addr + o} (rel {o}): got {got[o]:#04x} expected {ob_exp[o]:#04x} "
                   f"({'poison = never written' if got[o] == poison[o] else 'wrong value'}); {locate(job, o)}; "
                   f"mode={job['mode']} M={job['M']} K={job['K']} N={job['N']} flags={job['desc']['flags']} "
                   f"desc={job['desc_hex']}; other bad: "
                   + ", ".join(locate(job, x) for x in bad[1:6]))
    b.flush_checks(name)
    stats["jobs"] += 1
    stats["macs"] += job["M"] * job["N"] * job["K"]
    stats["gemm_cycles"] += got_ctr[0]
    stats["sim_cycles"] += sim_cycles


# ------------------------------------------------------------------ suites
def job_cost(j: dict) -> int:
    return sum(ld["len"] for ld in j["load"]) + 2 * j["expected"]["obuf_len"]


class Runner:
    def __init__(self, a):
        self.a = a
        self.tmp = Path(a.work) / "pred"
        self.tmp.mkdir(parents=True, exist_ok=True)
        self.results: list[dict] = []
        self.lock = threading.Lock()

    def preds(self, jobs: list[dict]) -> list[dict]:
        with ThreadPoolExecutor(16) as ex:
            return list(ex.map(lambda j: predict(self.a.cycle, j, self.tmp), jobs))

    def record(self, suite: str, name: str, ok: bool, err: str = "", **kw):
        with self.lock:
            self.results.append({"suite": suite, "case": name, "ok": ok, "error": err, **kw})
        if not ok:
            log(f"FAIL [{suite}] {name}: {err}")

    def board(self, tag: str) -> Board:
        extra = []
        if self.a.rand_reset:
            extra = ["+verilator+rand+reset+2", f"+verilator+seed+{self.a.seed}"]
        if self.a.trace_dir:
            Path(self.a.trace_dir).mkdir(parents=True, exist_ok=True)
            extra += ["--trace", str(Path(self.a.trace_dir) / f"{tag}.fst")]
        return Board(self.a.harness, tag, extra)

    def run_list(self, suite: str, tag: str, jobs: list[dict], preds: list[dict]) -> None:
        """Run jobs in order on one board, no reset in between. Keeps going after a failure."""
        stats = {"jobs": 0, "macs": 0, "gemm_cycles": 0, "sim_cycles": 0}
        t0 = time.time()
        try:
            b = self.board(tag)
        except Exception as e:  # noqa: BLE001
            for j in jobs:
                self.record(suite, j["name"], False, f"board start failed: {e}")
            return
        try:
            self.ping(b, suite)
            for j, p in zip(jobs, preds):
                try:
                    run_job(b, j, p, stats)
                    self.record(suite, j["name"], True, mode=j["mode"], M=j["M"], K=j["K"], N=j["N"])
                except Fail as e:
                    self.record(suite, j["name"], False, str(e), mode=j["mode"], M=j["M"], K=j["K"], N=j["N"])
                    b.rsps.clear()
                    # resync: a failed job may leave a frame half-read; restart the board
                    if "OUTPUT" not in str(e) and "COUNTERS" not in str(e):
                        b.close()
                        b = self.board(tag + "r")
        except Exception as e:  # noqa: BLE001
            self.record(suite, f"{tag}-infra", False, f"{type(e).__name__}: {e}\n{traceback.format_exc()}")
        finally:
            st = b.close()
            if st is not None and int(st[2]) != 0:
                self.record(suite, f"{tag}-uart", False, f"{st[2]} UART TX framing errors")
            log(f"  [{suite}/{tag}] {stats['jobs']}/{len(jobs)} jobs ok, {stats['macs']} MACs, "
                f"{stats['gemm_cycles']} GEMM cycles, uart rx/tx {st[0] if st else '?'}/{st[1] if st else '?'} B, "
                f"{time.time() - t0:.0f}s")

    def ping(self, b: Board, suite: str) -> None:
        _, p = b.xact(b.const["ping"], what="PING")
        if len(p) != 9 or p[:4] != bytes.fromhex("53504c54") or p[4] != PROTO_VERSION:
            raise Fail(f"{b.name}: PING payload {p.hex()}")
        b.flush_checks("ping")

    def parallel(self, suite: str, jobs: list[dict], workers: int) -> None:
        if self.a.only:
            jobs = [j for j in jobs if self.a.only in j["name"]]
            if not jobs:
                return
        preds = self.preds(jobs)
        bins: list[list[int]] = [[] for _ in range(max(1, min(workers, len(jobs))))]
        load = [0] * len(bins)
        for i in sorted(range(len(jobs)), key=lambda i: -job_cost(jobs[i])):   # LPT
            k = load.index(min(load))
            bins[k].append(i)
            load[k] += job_cost(jobs[i])
        log(f"[{suite}] {len(jobs)} jobs on {len(bins)} boards")
        with ThreadPoolExecutor(len(bins)) as ex:
            futs = [ex.submit(self.run_list, suite, f"{suite}{k}", [jobs[i] for i in sorted(bn)],
                              [preds[i] for i in sorted(bn)]) for k, bn in enumerate(bins)]
            for f in futs:
                f.result()

    # ---- suites
    def vec_gemm(self, mode: str, seed: int, n: int) -> list[dict]:
        out = Path(self.a.work) / "vec"
        subprocess.run([self.a.vec, "gemm", "--seed", str(seed), "--n", str(n), "--mode", mode,
                        "--max-m", "64", "--max-k", "256", "--max-n", "256", "--out", str(out)],
                       check=True, capture_output=True)
        d = json.loads((out / "gemm" / f"gemm_{mode}_s{seed}_n{n}.json").read_text())
        log(f"  vec gemm {mode} seed={seed} n={n}: generator {d['meta'].get('git_hash')} dirty={d['meta'].get('git_dirty')}")
        return d["jobs"]

    def suite_full(self):
        self.parallel("full", self.vec_gemm("full", self.a.seed, self.a.n_full), self.a.workers)

    def suite_strip(self):
        self.parallel("strip", self.vec_gemm("strip", self.a.seed, self.a.n_strip), self.a.workers)

    def suite_directed(self):
        d = json.loads((VEC_OUT / "directed.json").read_text())
        log(f"  directed.json generator {d['meta'].get('git_hash')}")
        self.parallel("directed", d["jobs"], self.a.workers)

    def suite_modeswitch(self):
        """FULL->STRIP->FULL... on one board with no reset, several independent chains."""
        n = self.a.n_switch
        chains = []
        for c in range(self.a.switch_chains):
            f = self.vec_gemm("full", 1000 + 10 * self.a.seed + c, n)
            s = self.vec_gemm("strip", 2000 + 10 * self.a.seed + c, n)
            seq = []
            for i in range(n):
                seq += [f[i], s[i]]
            seq.append(f[0])  # end on FULL again
            chains.append(seq)
        allpreds = self.preds([j for ch in chains for j in ch])
        log(f"[modeswitch] {len(chains)} chains x {len(chains[0])} jobs (F,S,F,S,...,F) without reset")
        with ThreadPoolExecutor(len(chains)) as ex:
            futs, k = [], 0
            for ci, ch in enumerate(chains):
                futs.append(ex.submit(self.run_list, "modeswitch", f"switch{ci}", ch, allpreds[k:k + len(ch)]))
                k += len(ch)
            for f in futs:
                f.result()

    def suite_selftest(self):
        exp = json.loads((VEC_OUT / "selftest_expected.json").read_text())
        jf, js = (j for j in exp["jobs"])
        assert jf["mode"] == "full" and js["mode"] == "strip"
        pred_strip = predict(self.a.cycle, js["job"], self.tmp)
        st_payload_exp = (bytes([1, 1]) + u32(jf["fletcher32"]) + u32(js["fletcher32"]))
        log(f"[selftest] expected fletcher FULL {jf['fletcher32_hex']} STRIP {js['fletcher32_hex']} "
            f"(generator {exp['meta']['git_hash']})")
        cases = [self.selftest_opcode, self.selftest_btn0, self.selftest_busy]
        with ThreadPoolExecutor(len(cases)) as ex:
            futs = [ex.submit(self._case, "selftest", c.__name__, c, js, pred_strip, st_payload_exp) for c in cases]
            for f in futs:
                f.result()

    def _case(self, suite, name, fn, *args):
        b = None
        try:
            b = self.board(name)
            self.ping(b, suite)
            fn(b, *args)
            b.flush_checks(name)
            self.record(suite, name, True)
        except Exception as e:  # noqa: BLE001
            self.record(suite, name, False, f"{type(e).__name__}: {e}")
        finally:
            if b:
                st = b.close()
                if st is not None and int(st[2]) != 0:
                    self.record(suite, f"{name}-uart", False, f"{st[2]} UART TX framing errors")

    def selftest_opcode(self, b: Board, js, pred_strip, st_exp):
        if b.poll() != POLL_IDLE:
            raise Fail("POLL after power-on is not IDLE")
        if b.led() & 0b11:
            raise Fail(f"LED0/LED1 lit before any self-test: LED={b.led():04x}")
        for rnd in range(2):     # twice: the self-test must be repeatable without reset
            _, p = b.xact(b.const["selftest"], what="SELFTEST")
            if p != st_exp:
                raise Fail(f"SELFTEST round {rnd}: payload {p.hex()} expected {st_exp.hex()} "
                           f"(pass_full, pass_strip, fl_full LE, fl_strip LE)")
            check_counters(b.counters(), pred_strip, js["job"], f"selftest opcode round {rnd} (STRIP counters)")
            b.ctl("step 4")
            if b.led() & 0b111 != 0b011:
                raise Fail(f"after SELFTEST LED[2:0]={b.led() & 7:03b}, expected 011 (pass_strip, pass_full, !busy)")
        # a user STRIP job, then FULL job, then self-test again: shared buffers reused
        jobs = self.vec_gemm("strip", 3000 + self.a.seed, 2)[:1] + self.vec_gemm("full", 3000 + self.a.seed, 2)[:1]
        stats = {"jobs": 0, "macs": 0, "gemm_cycles": 0, "sim_cycles": 0}
        for j, p in zip(jobs, self.preds(jobs)):
            run_job(b, j, p, stats)
        _, p = b.xact(b.const["selftest"], what="SELFTEST after user jobs")
        if p != st_exp:
            raise Fail(f"SELFTEST after user jobs: payload {p.hex()} expected {st_exp.hex()}")
        check_counters(b.counters(), pred_strip, js["job"], "selftest after user jobs (STRIP counters)")
        # BTN1 = synchronous reset: LEDs clear, POLL back to IDLE, link still alive
        b.ctl("btn 2")
        b.ctl("step 200")
        b.ctl("btn 0")
        b.ctl("step 200")
        if b.led() & 0b111:
            raise Fail(f"after BTN1 reset LED[2:0]={b.led() & 7:03b}, expected 000")
        self.ping(b, "selftest")
        if b.poll() != POLL_IDLE:
            raise Fail("POLL after BTN1 reset is not IDLE")

    def selftest_btn0(self, b: Board, js, pred_strip, st_exp):
        if b.led() & 0b111:
            raise Fail(f"LED[2:0] before BTN0 = {b.led() & 7:03b}")
        b.ctl("btn 1")          # press BTN0
        b.ctl("step 100")       # > DEB_CYC (16) + synchronizer
        b.ctl("btn 0")
        b.ctl("step 100")
        b.ctl("idle")
        b.ctl("step 4")
        led = b.led()
        if led & 0b111 != 0b011:
            raise Fail(f"after BTN0 self-test LED[2:0]={led & 7:03b}, expected 011 (LED0 FULL pass, LED1 STRIP pass)")
        check_counters(b.counters(), pred_strip, js["job"], "BTN0 self-test (STRIP counters)")
        # second press: still passes
        b.ctl("btn 1")
        b.ctl("step 100")
        b.ctl("btn 0")
        b.ctl("step 100")
        b.ctl("idle")
        b.ctl("step 4")
        if b.led() & 0b111 != 0b011:
            raise Fail(f"after 2nd BTN0 LED[2:0]={b.led() & 7:03b}")
        # and the host SELFTEST agrees afterwards
        _, p = b.xact(b.const["selftest"], what="SELFTEST after BTN0")
        if p != st_exp:
            raise Fail(f"SELFTEST after BTN0: payload {p.hex()} expected {st_exp.hex()}")

    def selftest_busy(self, b: Board, js, pred_strip, st_exp):
        """System-level BUSY rules (SPEC §3.8) while a long job runs; PING/POLL/READ_COUNTERS served live."""
        jobs = self.vec_gemm("full", 4000 + self.a.seed, 2)
        big = jobs[1]                                  # edge shape (max_m, max_k, max_n) = 64x256x256
        assert (big["M"], big["K"], big["N"]) == (64, 256, 256), (big["M"], big["K"], big["N"])
        pred = self.preds([big])[0]
        items = []
        for ld in big["load"]:
            for a, d in chunks(ld["byte_addr"], bytes.fromhex(ld["hex"]), WMAX):
                items.append((OP["WRITE_MEM"], bytes([ld["region"]]) + u32(a) + d))
        nw = len(items)
        items += [(OP["RUN_GEMM"], bytes.fromhex(big["desc_hex"])),
                  (OP["RUN_GEMM"], bytes.fromhex(big["desc_hex"])),
                  (OP["WRITE_MEM"], bytes([REGION["ABUF"], 0, 0, 0, 0, 0x55])),
                  (OP["READ_MEM"], bytes([REGION["ABUF"], 0, 0, 0, 0]) + u16(8))]
        fr = req_frames(items)
        for i in range(nw):
            b.xact(fr[i], what="load")
        b.xact(fr[nw], what="RUN_GEMM")
        b.xact(fr[nw + 1], ST["BUSY"], "RUN_GEMM while busy")
        b.xact(b.const["selftest"], ST["BUSY"], "SELFTEST while busy")
        b.xact(fr[nw + 2], ST["BUSY"], "WRITE_MEM while busy")
        b.xact(fr[nw + 3], ST["BUSY"], "READ_MEM while busy")
        self.ping(b, "busy")
        if b.poll() != POLL_BUSY:
            raise Fail("POLL during a 64x256x256 job is not BUSY")
        live = b.counters()
        final_exp = model_vector(pred)
        if not (0 < live[0] < final_exp[0]):
            raise Fail(f"live cycles_total {live[0]} not in (0, {final_exp[0]})")
        b.ctl("idle")
        if b.poll() != POLL_DONE:
            raise Fail("POLL after job is not DONE")
        check_counters(b.counters(), pred, big, "busy-case final counters")
        _, p = b.xact(b.const["selftest"], what="SELFTEST after busy job")
        if p != st_exp:
            raise Fail(f"SELFTEST after busy job: payload {p.hex()} expected {st_exp.hex()}")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--harness", required=True)
    ap.add_argument("--vec", required=True)
    ap.add_argument("--cycle", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--seed", type=int, default=int(os.environ.get("SEED", "1")))
    ap.add_argument("--n-full", type=int, default=200)
    ap.add_argument("--n-strip", type=int, default=100)
    ap.add_argument("--n-switch", type=int, default=6)
    ap.add_argument("--switch-chains", type=int, default=4)
    ap.add_argument("--workers", type=int, default=16)
    ap.add_argument("--suites", default="full,strip,directed,modeswitch,selftest")
    ap.add_argument("--rand-reset", action="store_true", help="randomize X-initial state (+verilator+rand+reset+2)")
    ap.add_argument("--summary", default="")
    ap.add_argument("--only", default="", help="run only jobs whose name contains this (full/strip/directed)")
    ap.add_argument("--trace-dir", default="", help="FST per board (needs a WAVES=1 harness build)")
    ap.add_argument("--meta", default="{}", help="provenance JSON (rtl/tb hashes, dirty inputs) for the summary")
    a = ap.parse_args()
    meta = json.loads(a.meta)
    log(f"system_test: SEED={a.seed} suites={a.suites} workers={a.workers} harness={a.harness} meta={meta}")
    r = Runner(a)
    p0 = r.preds([{"name": "latency_probe", "mode": "full", "M": 1, "K": 1, "N": 1, "requant": True}])[0]
    meta["model_latencies"] = p0["latencies"]
    meta["model_latencies_provisional"] = p0["latencies_provisional"]
    meta["model_git_hash"] = p0["model"].get("model_git_hash")
    log(f"  cycle model: latencies {p0['latencies']} provisional={p0['latencies_provisional']} "
        f"model_git_hash={meta['model_git_hash']}")
    t0 = time.time()
    suites = [s for s in a.suites.split(",") if s]
    with ThreadPoolExecutor(len(suites)) as ex:
        futs = {s: ex.submit(getattr(r, f"suite_{s}")) for s in suites}
        for s, f in futs.items():
            try:
                f.result()
            except Exception as e:  # noqa: BLE001
                r.record(s, f"{s}-suite", False, f"{type(e).__name__}: {e}\n{traceback.format_exc()}")
    dt = time.time() - t0
    by = {}
    for x in r.results:
        s = by.setdefault(x["suite"], [0, 0])
        s[0 if x["ok"] else 1] += 1
    log("== system summary (SEED=%d, %.0fs)" % (a.seed, dt))
    for s, (p, f) in by.items():
        log(f"   {s:11s} {p:4d} passed {f:4d} failed")
    fails = [x for x in r.results if not x["ok"]]
    if a.summary:
        Path(a.summary).write_text(json.dumps({"seed": a.seed, "seconds": dt, "meta": meta, "by_suite": by,
                                               "results": r.results}, indent=1))
    if fails:
        log(f"== FAILED: {len(fails)} case(s); first: [{fails[0]['suite']}] {fails[0]['case']}: {fails[0]['error'][:600]}")
        return 1
    log(f"== PASSED: {len(r.results)} cases")
    return 0


if __name__ == "__main__":
    sys.exit(main())
