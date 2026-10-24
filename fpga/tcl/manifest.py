#!/usr/bin/env python3
"""Bitstream manifest for Phase H (owner: fpga-integrator).

    python fpga/tcl/manifest.py [tag ...]
    default tags: the four bitstreams fpga/tcl/s4_rebuild.sh builds (see TAGS)

For each tag, reads fpga/reports/<tag>/summary.json and hashes
fpga/build/<tag>/top.bit (gitignored, so the hash is the link between the
committed report and the file that gets programmed). Writes
fpga/reports/manifest.json and rewrites the table between the
<!-- MANIFEST:BEGIN --> / <!-- MANIFEST:END --> markers in
docs/HW_CHECKLIST.md. Exits 1 if any tag's build did not pass, is dirty, or has
no bitstream on disk.
"""
import hashlib
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
# top_uartswap is no longer a default tag: H-2 (2026-10-24) confirmed the
# default UART pins (FPGA RX = B16, TX = A16), so the swapped full top is never
# programmed and fpga/tcl/s4_rebuild.sh does not build it. Its 52a66216 reports
# stay committed in fpga/reports/top_uartswap/ for history; list it explicitly
# (python fpga/tcl/manifest.py ... top_uartswap) only if it is rebuilt.
# heartbeat_uart_echo_uartswap and packet_mem_uartswap were dropped for the same
# reason (2026-10-24); their 52a66216 reports also stay committed for history.
TAGS = ["heartbeat_uart_echo", "packet_mem", "top", "top_nostrip"]
USE = {
    "heartbeat_uart_echo": "H-1, H-2",
    "heartbeat_uart_echo_uartswap": "H-2 fallback (UART pins swapped)",
    "packet_mem": "H-3",
    "packet_mem_uartswap": "H-3 if H-2 needed the swap",
    "top": "H-4 .. H-7 (KG source)",
    "top_uartswap": "H-4 .. H-7 (KG1 source) if H-2 needed the swap",
    "top_nostrip": "cost build only (results/cost.json); never programmed for the demo",
}


def main():
    tags = sys.argv[1:] or TAGS
    rows, errs = [], []
    for t in tags:
        s = json.loads((ROOT / "fpga/reports" / t / "summary.json").read_text(encoding="utf-8"))
        bit = ROOT / "fpga/build" / t / "top.bit"
        md5 = hashlib.md5(bit.read_bytes()).hexdigest() if bit.is_file() else None
        if s.get("pass") is not True:
            errs.append(f"{t}: build did not pass")
        if s.get("git_dirty") != "false":
            errs.append(f"{t}: git_dirty {s.get('git_dirty')}")
        if md5 is None:
            errs.append(f"{t}: {bit.relative_to(ROOT)} missing")
        rows.append({"tag": t, "use": USE.get(t, ""), "bitfile": f"fpga/build/{t}/top.bit",
                     "build_id": s["build_id"], "git_hash": s["git_hash"], "md5": md5,
                     "wns_ns": s["wns_ns"], "whs_ns": s["whs_ns"], "uart_swap": s.get("uart_swap", 0),
                     "enable_strip": s.get("enable_strip"), "phys_opt": s.get("phys_opt", "Default")})
    (ROOT / "fpga/reports/manifest.json").write_text(json.dumps(rows, indent=2) + "\n", encoding="utf-8")

    lines = ["| Tag | Used at | build_id | git | WNS / WHS (ns) | UART pins | bit md5 |",
             "| --- | --- | --- | --- | --- | --- | --- |"]
    for r in rows:
        pins = "RX A16 / TX B16 (swap)" if r["uart_swap"] == 1 else "RX B16 / TX A16"
        lines.append(f"| `{r['tag']}` | {r['use']} | `{r['build_id']}` | `{r['git_hash']}` | "
                     f"{r['wns_ns']:+.3f} / {r['whs_ns']:+.3f} | {pins} | `{r['md5']}` |")
    table = "\n".join(lines)
    cl = ROOT / "docs/HW_CHECKLIST.md"
    text = cl.read_text(encoding="utf-8")
    b, e = "<!-- MANIFEST:BEGIN -->", "<!-- MANIFEST:END -->"
    if b not in text or e not in text:
        errs.append("manifest markers missing in docs/HW_CHECKLIST.md")
    else:
        i, j = text.index(b) + len(b), text.index(e)
        cl.write_text(text[:i] + "\n" + table + "\n" + text[j:], encoding="utf-8", newline="\n")
    print(table)
    for x in errs:
        print("manifest: " + x, file=sys.stderr)
    return 1 if errs else 0


if __name__ == "__main__":
    sys.exit(main())
