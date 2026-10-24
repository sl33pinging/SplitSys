#!/usr/bin/env python3
"""Write results/cost.json from two Vivado builds (owner: fpga-integrator).

    python fpga/tcl/write_cost.py [split_tag] [full_only_tag]
    defaults: split_tag = top, full_only_tag = top_nostrip

Reads fpga/reports/<tag>/summary.json written by fpga/tcl/build.tcl and refuses
to write unless both builds passed, are clean (git_dirty false), come from the
same commit, and have ENABLE_STRIP 1 (split) and 0 (full_only). Output format:
docs/DEMO.md section 4, "source": "hw-build".

Also reads fpga/reports/<tag>/utilization_hier.rpt (report_utilization
-hierarchical; build.tcl writes it, fpga/tcl/report_hier.tcl regenerates it from
a checkpoint) for a per-block breakdown (array, requant, sequencer, core, total).
With flatten_hierarchy=rebuilt some small instances (e.g. the OBUF/RQPARAM
buffers) are not listed separately, so the blocks do not sum to the total.

fmax_mhz is build.tcl's estimate 1000 / (period - WNS) at the 100 MHz
constraint, not a swept maximum frequency.
"""
import json
import re
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]


def load(tag):
    p = ROOT / "fpga" / "reports" / tag / "summary.json"
    return p, json.loads(p.read_text(encoding="utf-8"))


BLOCKS = {"array": "u_array", "requant": "u_rq", "sequencer": "u_seq", "core": "u_core", "total": "top"}


def breakdown(tag):
    p = ROOT / "fpga" / "reports" / tag / "utilization_hier.rpt"
    rows = {}
    for line in p.read_text(encoding="utf-8").splitlines():
        cells = [c.strip() for c in line.split("|")]
        if len(cells) < 12 or not cells[1] or cells[1] == "Instance":
            continue
        inst = cells[1]
        try:
            nums = [float(c) for c in cells[3:11]]
        except ValueError:
            continue
        rows.setdefault(inst, nums)  # first occurrence (top-down)
    out = {}
    for name, inst in BLOCKS.items():
        if inst not in rows:
            raise SystemExit(f"write_cost: {inst} not in {p}")
        lut, _logic, _lutram, _srl, ff, b36, b18, dsp = rows[inst]
        out[name] = {"lut": int(lut), "ff": int(ff), "dsp": int(dsp), "bram36": int(b36), "bram18": int(b18)}
    return out


def worst_path(tag):
    """First setup path in timing_summary.rpt: slack, source, destination, logic levels, route share."""
    t = (ROOT / "fpga" / "reports" / tag / "timing_summary.rpt").read_text(encoding="utf-8")
    t = t[t.index("Max Delay Paths"):]
    g = lambda pat: re.search(pat, t).group(1).strip()
    return {"slack_ns": float(g(r"Slack \(\w+\) :\s+(-?[\d.]+)ns")),
            "source": g(r"Source:\s+(\S+)"), "destination": g(r"Destination:\s+(\S+)"),
            "logic_levels": int(g(r"Logic Levels:\s+(\d+)")),
            "route_pct": float(g(r"route [\d.]+ns \(([\d.]+)%\)"))}


def main():
    split_tag = sys.argv[1] if len(sys.argv) > 1 else "top"
    full_tag = sys.argv[2] if len(sys.argv) > 2 else "top_nostrip"
    sp, s = load(split_tag)
    fp, f = load(full_tag)
    errs = []
    for tag, d, strip in ((split_tag, s, "1"), (full_tag, f, "0")):
        if d.get("pass") is not True:
            errs.append(f"{tag}: build did not pass: {d.get('failures')}")
        if d.get("git_dirty") != "false":
            errs.append(f"{tag}: git_dirty is {d.get('git_dirty')}")
        if d.get("top") != "top":
            errs.append(f"{tag}: top is {d.get('top')}, expected top")
        if str(d.get("enable_strip")) != strip:
            errs.append(f"{tag}: enable_strip is {d.get('enable_strip')}, expected {strip}")
    if s.get("git_hash") != f.get("git_hash"):
        errs.append(f"different commits: {s.get('git_hash')} vs {f.get('git_hash')}")
    if s.get("uart_swap") != f.get("uart_swap"):
        errs.append("different UART_SWAP settings")
    if errs:
        for e in errs:
            print("write_cost: " + e, file=sys.stderr)
        return 1

    def pick(d):
        return {"lut": d["lut"], "ff": d["ff"], "dsp": d["dsp"], "bram": d["bram"],
                "fmax_mhz": d["fmax_est_mhz"], "wns_ns": d["wns_ns"]}

    bs, bf = breakdown(split_tag), breakdown(full_tag)
    delta = {k: {m: bs[k][m] - bf[k][m] for m in ("lut", "ff", "dsp")} for k in BLOCKS}
    out = {
        "source": "hw-build",
        "full_only": pick(f),
        "split": pick(s),
        "git_hash": s["git_hash"],
        "build_id": {"split": s["build_id"], "full_only": f["build_id"]},
        "part": s["part"],
        "vivado": s["vivado"],
        "clock_mhz": round(1000.0 / s["clk_period_ns"], 3),
        "fmax_note": ("fmax_mhz = 1000/(10 ns - WNS) from the post-route timing at the 100 MHz constraint; "
                      "an estimate, not a swept Fmax. Check worst_path before attributing the Fmax delta "
                      "to splitting: a routing-dominated control net (reset, mux select) is placement, "
                      "not STRIP logic"),
        "worst_path": {"split": worst_path(split_tag), "full_only": worst_path(full_tag)},
        "breakdown": {"split": bs, "full_only": bf, "delta": delta},
        "note": ("split minus full_only includes the STRIP datapath (array W-entry/A-source muxes, "
                 "sequencer STRIP control) AND requant lanes 8-15 (STRIP's 16-wide output: "
                 "16 DSPs), which ENABLE_STRIP=0 trims because FULL drains 8 lanes; see breakdown.delta"),
        "reports": {"split": f"fpga/reports/{split_tag}/summary.json",
                    "full_only": f"fpga/reports/{full_tag}/summary.json"},
    }
    dst = ROOT / "results" / "cost.json"
    dst.write_text(json.dumps(out, indent=2) + "\n", encoding="utf-8")
    print(f"write_cost: wrote {dst.relative_to(ROOT)}")
    print(json.dumps(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
