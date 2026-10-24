#!/usr/bin/env python3
"""Write results/cost.json from two Vivado builds (owner: fpga-integrator).

    python fpga/tcl/write_cost.py [split_tag] [full_only_tag]
    defaults: split_tag = top, full_only_tag = top_nostrip

Reads fpga/reports/<tag>/summary.json written by fpga/tcl/build.tcl and refuses
to write unless both builds passed, are clean (git_dirty false), come from the
same commit, and have ENABLE_STRIP 1 (split) and 0 (full_only). Output format:
docs/DEMO.md section 4, "source": "hw-build".

fmax_mhz is build.tcl's estimate 1000 / (period - WNS) at the 100 MHz
constraint, not a swept maximum frequency.
"""
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]


def load(tag):
    p = ROOT / "fpga" / "reports" / tag / "summary.json"
    return p, json.loads(p.read_text(encoding="utf-8"))


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

    out = {
        "source": "hw-build",
        "full_only": pick(f),
        "split": pick(s),
        "git_hash": s["git_hash"],
        "build_id": {"split": s["build_id"], "full_only": f["build_id"]},
        "part": s["part"],
        "vivado": s["vivado"],
        "clock_mhz": round(1000.0 / s["clk_period_ns"], 3),
        "fmax_note": "fmax_mhz = 1000/(10 ns - WNS) from the post-route timing at the 100 MHz constraint; an estimate, not a swept Fmax",
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
