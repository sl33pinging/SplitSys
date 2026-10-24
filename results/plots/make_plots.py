#!/usr/bin/env python3
"""SplitSys results plots (owner: demo-storyteller).

Builds every static plot for the README and Devpost from JSON in results/:

    results/model/sweep.json      MODEL  utilization vs M, every counter      (perf-modeler)
    results/model/roofline.json   MODEL  arithmetic intensity vs MACs/cycle   (perf-modeler)
    results/hw/*.json             HW     measured runs, DEMO.md section 4 schema (host)
    results/sim/*.json            SIM    co-simulation runs, same schema      (host via tb/cosim)
    results/cost.json             BUILD  ENABLE_STRIP=1 vs =0 resources      (fpga-integrator)

Nothing is typed in by hand: every plotted value and every number in a title is
read from those files. Measured points (HW, SIM) are overlaid on the model only
when such files exist; until then the plots are MODEL-only and say so.

Output: results/plots/<name>_<theme>.{svg,png} for theme in (dark, light), and
results/plots/manifest.json (inputs with sha256, headline numbers used in titles).
Output is deterministic: no timestamps, fixed SVG hash salt, no PNG software tag.

Run (WSL, venv python):
    scripts/wsl 'python results/plots/make_plots.py'
    scripts/wsl 'python results/plots/make_plots.py --check'   # regenerate twice, compare bytes
Measured overlays: --measured hw,sim (default). Only git-committed results/{hw,sim} records are
used unless --include-untracked is given (preview only; never publish such plots).

Mode colors: FULL blue, STRIP orange (DEMO.md section 3). The dark theme uses the
dashboard values; the light theme uses darker steps of the same hues so marks keep
>= 3:1 contrast on white (checked with the dataviz palette validator).
"""

from __future__ import annotations

import argparse
import glob
import hashlib
import json
import math
import os
import sys
import tempfile

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
import matplotlib.ticker  # noqa: E402,F401
from matplotlib.lines import Line2D  # noqa: E402
from matplotlib.patches import Patch  # noqa: E402

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
GEMM_ORDER = ["qkv", "attn_out", "ffn_up", "ffn_down"]
HEADLINE_GEMM = "ffn_up"  # the scripted demo GEMM (DEMO.md section 5)

THEMES = {
    "dark": {
        "bg": "#0B0D10", "panel": "#14181D", "grid": "#262C33", "text": "#E8ECF1",
        "muted": "#9AA4AF", "FULL": "#4C9AFF", "STRIP": "#FF9F43",
        "model_face": "none", "model_edge": "#FFFFFF", "shade": "#1B2128",
    },
    "light": {
        "bg": "#FFFFFF", "panel": "#FFFFFF", "grid": "#E3E7EC", "text": "#14181D",
        "muted": "#5B6570", "FULL": "#2F7DE1", "STRIP": "#D9700F",
        "model_face": "none", "model_edge": "#14181D", "shade": "#F1F3F6",
    },
}


# --------------------------------------------------------------------------- data


def sha256(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def rel(path):
    return os.path.relpath(path, ROOT).replace(os.sep, "/")


def load_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def key_of(gemm):
    return (gemm["name"], int(gemm["M"]), gemm["mode"].upper())


def load_model():
    sweep_p = os.path.join(ROOT, "results", "model", "sweep.json")
    roof_p = os.path.join(ROOT, "results", "model", "roofline.json")
    sweep, roof = load_json(sweep_p), load_json(roof_p)
    for doc, p in ((sweep, sweep_p), (roof, roof_p)):
        if doc.get("source") != "model":
            sys.exit(f"{rel(p)}: source is {doc.get('source')!r}, expected 'model'")
    if sweep["model_git_hash"] != roof["model_git_hash"]:
        sys.exit("sweep.json and roofline.json have different model_git_hash")
    pts = {}
    for p in sweep["points"]:
        g = p["gemm"]
        if not g.get("requant", True):
            continue  # plots are int8 (requant on) only, like the benchmarks
        pts[key_of(g)] = {
            "util": p["derived"]["util_model"],
            "ceiling": p["derived"]["mapping_ceiling"],
            "cycles_total": p["model"]["cycles_total"],
            "tiles": p["model"]["tiles"],
        }
    return {
        "sweep": sweep, "roof": roof, "points": pts,
        "hash": sweep["model_git_hash"],
        "provisional": bool(sweep.get("latencies_provisional", True)),
        "inputs": [(rel(sweep_p), sha256(sweep_p)), (rel(roof_p), sha256(roof_p))],
    }


def git_tracked(subdir):
    """Set of repo-relative paths tracked by git under subdir, or None if git is unavailable."""
    import subprocess
    try:
        r = subprocess.run(["git", "-C", ROOT, "ls-files", "--", subdir], capture_output=True,
                           text=True, check=True)
    except (OSError, subprocess.CalledProcessError):
        return None
    return set(r.stdout.split())


def load_measured(source, include_untracked=False):
    """Load results/<source>/*.json (DEMO.md section 4). Returns {key: record} using the
    newest run_id per key. Runs that failed the golden check or invariants are excluded
    and reported, never plotted. By default only git-committed records are used, so a
    published plot can always be traced to a commit."""
    files = sorted(glob.glob(os.path.join(ROOT, "results", source, "*.json")))
    tracked = None if include_untracked else git_tracked(f"results/{source}")
    if tracked is None and not include_untracked:
        print(f"[make_plots] warning: git unavailable; using all {source} files", file=sys.stderr)
    out, inputs, rejected = {}, [], []
    for f in files:
        if tracked is not None and rel(f) not in tracked:
            print(f"[make_plots] {source}: skipped uncommitted {rel(f)}", file=sys.stderr)
            continue
        try:
            r = load_json(f)
        except (OSError, ValueError) as e:
            rejected.append((rel(f), f"unreadable: {e}"))
            continue
        if not isinstance(r, dict) or "gemm" not in r or "counters" not in r:
            continue  # session.json, sweeps, etc.
        if r.get("source") != source:
            rejected.append((rel(f), f"source {r.get('source')!r} != {source!r}"))
            continue
        chk = r.get("check", {})
        if not (chk.get("golden_match") is True and chk.get("mismatches") == 0
                and chk.get("invariants_ok") is True):
            rejected.append((rel(f), "failed golden/invariant check"))
            continue
        g = r["gemm"]
        if not g.get("requant", True):
            continue
        c = r["counters"]
        if c["cycles_total"] <= 0:
            rejected.append((rel(f), "cycles_total <= 0"))
            continue
        util = c["mac_useful"] / (64 * c["cycles_total"])  # SPEC section 3.7
        k = key_of(g)
        m = r.get("model", {})
        counter_diffs = sorted(k for k in c if k in m and c[k] != m[k])
        rec = {"util": util, "cycles_total": c["cycles_total"], "run_id": r.get("run_id", ""),
               "build_id": r.get("build_id"), "git_hash": str(r.get("git_hash", "")),
               "file": rel(f), "model_git_hash": m.get("model_git_hash"),
               "counter_diffs": counter_diffs}
        if k not in out or rec["run_id"] > out[k]["run_id"]:
            out[k] = rec
        inputs.append((rel(f), sha256(f)))
        st = RECORD_STATS.setdefault(source, {"records": 0, "counter_diffs": 0, "builds": set()})
        st["records"] += 1
        st["counter_diffs"] += 1 if counter_diffs else 0
        st["builds"].add(str(r.get("build_id")))
    for f, why in rejected:
        print(f"[make_plots] {source}: excluded {f}: {why}", file=sys.stderr)
    return out, inputs


# Per-source totals over every loaded record (not only the newest per key), for source lines.
RECORD_STATS: dict = {}


def load_cost():
    p = os.path.join(ROOT, "results", "cost.json")
    if not os.path.exists(p):
        return None, []
    return load_json(p), [(rel(p), sha256(p))]


# --------------------------------------------------------------------------- style


def style_axes(ax, th):
    ax.set_facecolor(th["panel"])
    for s in ("top", "right"):
        ax.spines[s].set_visible(False)
    for s in ("left", "bottom"):
        ax.spines[s].set_color(th["grid"])
    ax.tick_params(colors=th["muted"], labelsize=10)
    ax.xaxis.label.set_color(th["muted"])
    ax.yaxis.label.set_color(th["muted"])
    ax.grid(True, color=th["grid"], linewidth=0.6)
    ax.set_axisbelow(True)


def new_fig(th, w=10.0, h=5.6, ncols=1, nrows=1, **kw):
    fig, axes = plt.subplots(nrows, ncols, figsize=(w, h), facecolor=th["bg"], **kw)
    return fig, axes


SIM_NOTE = ("SIM = Verilator co-simulation of the RTL, driven by the real host through its "
            "UART pins. Counters equal the model exactly,\nas expected with identical "
            "latencies; the real test of the model is hardware.")


def title_and_source(fig, th, title, subtitle, source_line):
    fig.text(0.012, 0.975, title, ha="left", va="top", fontsize=14, fontweight="bold",
             color=th["text"])
    fig.text(0.012, 0.915, subtitle, ha="left", va="top", fontsize=10, color=th["muted"])
    fig.text(0.012, 0.015, source_line, ha="left", va="bottom", fontsize=8.5,
             color=th["muted"], family="monospace")


def sim_only(hw, sim):
    """SIM records whose shape has no HW measurement. Where the board measured a shape, the
    HW point is drawn and the SIM point is not: identical values would otherwise overdraw
    the HW marker, and HW is the stronger evidence."""
    return {k: v for k, v in sim.items() if k not in hw}


def source_line(model, measured_sets):
    """One line per source. 'drawn' counts only benchmark points that exist in the model
    sweep (random-shape records are loaded and checked, but have no place on these axes)."""
    lat = "latencies PROVISIONAL" if model["provisional"] else "latencies RTL-confirmed, D-017"
    lines = [f"source: MODEL results/model/*.json, model_git_hash {model['hash']} ({lat})"]
    hw = dict(measured_sets).get("HW") or {}
    for label, recs in measured_sets:
        if recs:
            st = RECORD_STATS.get(label.lower(), {})
            builds = sorted(st.get("builds") or {str(r["build_id"]) for r in recs.values()})
            nrec = st.get("records", len(recs))
            dirty = any(r["git_hash"].endswith("-dirty") for r in recs.values())
            ndiff = st.get("counter_diffs", sum(1 for r in recs.values() if r["counter_diffs"]))
            drawn = recs if label == "HW" else sim_only(hw, recs)
            n = sum(1 for k in drawn if k in model["points"])
            what = "co-sim, not HW" if label == "SIM" else "board, on-chip counters"
            shown = (f"{n} benchmark point{'s' if n != 1 else ''} drawn" if n else
                     "not drawn (HW measured every plotted point)")
            lines.append(f"{label} ({what}): {nrec} int8 record{'s' if nrec != 1 else ''}, "
                         f"build {','.join(builds)}{' (dirty tree)' if dirty else ''}, "
                         f"{ndiff} with a counter != model; {shown}")
        elif label == "HW":
            lines.append("HW: none yet")
    return "\n".join(lines)


def title_values(model, hw, sim, keys):
    """Numbers quoted in a title come from HW when every needed point was measured on the
    board, else from SIM when every point was co-simulated, else from the MODEL. The
    returned tag says which, and is printed at the start of the title."""
    if hw and all(k in hw for k in keys):
        return "HW", {k: hw[k]["util"] for k in keys}
    if sim and all(k in sim for k in keys):
        return "SIM", {k: sim[k]["util"] for k in keys}
    return "MODEL", {k: model["points"][k]["util"] for k in keys}


def legend(ax, th, handles, loc="best", ncol=1):
    lg = ax.legend(handles=handles, loc=loc, ncol=ncol, fontsize=9, frameon=True)
    lg.get_frame().set_facecolor(th["panel"])
    lg.get_frame().set_edgecolor(th["grid"])
    for t in lg.get_texts():
        t.set_color(th["text"])
    return lg


def pct(x):
    return f"{100 * x:.1f}%"


# --------------------------------------------------------------------------- plots


def measured_markers(ax, th, mode, xs_vals, recs, marker):
    if not xs_vals:
        return
    xs, ys = zip(*xs_vals)
    ax.plot(xs, ys, linestyle="none", marker=marker, markersize=5.5, color=th[mode],
            markeredgecolor=th[mode], markeredgewidth=0, zorder=5)


def util_series(model, gemm, mode):
    ms = model["sweep"]["m_values"]
    return [(m, model["points"][(gemm, m, mode)]["util"]) for m in ms
            if (gemm, m, mode) in model["points"]]


def measured_series(recs, gemm, mode, ms):
    return [(m, recs[(gemm, m, mode)]["util"]) for m in ms if (gemm, m, mode) in recs]


def draw_util(ax, th, model, gemm, hw, sim, show_labels=True, label_size=9):
    ms = model["sweep"]["m_values"]
    ax.axvspan(min(ms) / 1.3, 4 * 1.15, color=th["shade"], zorder=0, linewidth=0)
    for mode in ("FULL", "STRIP"):
        s = util_series(model, gemm, mode)
        xs, ys = zip(*s)
        ax.plot(xs, ys, color=th[mode], linewidth=2, linestyle=(0, (4, 2)), zorder=3)
        # Model = hollow ring, drawn larger than measured markers so both stay visible.
        ax.plot(xs, ys, linestyle="none", marker="o", markersize=11,
                markerfacecolor=th["model_face"], markeredgecolor=th["model_edge"],
                markeredgewidth=1.5, zorder=6)
        measured_markers(ax, th, mode, measured_series(hw, gemm, mode, ms), hw, "o")
        measured_markers(ax, th, mode, measured_series(sim_only(hw, sim), gemm, mode, ms),
                         sim, "s")
        if show_labels:
            # Direct labels at decode (M=1) and prefill (M=64) only.
            for m, y in (s[0], s[-1]):
                # Label with the best available source (HW > SIM > MODEL), as in the title.
                k = (gemm, m, mode)
                y = hw[k]["util"] if k in hw else (sim[k]["util"] if k in sim else y)
                first = m == ms[0]
                up = mode == "STRIP"
                ax.annotate(f"{mode} {pct(y)}", (m, y), textcoords="offset points",
                            xytext=(9 if first else -6, (4 if up else -4) if first
                                    else (10 if up else -16)),
                            ha="left" if first else "right",
                            va=("bottom" if up else "top") if first else "baseline",
                            fontsize=label_size, color=th["text"])
    ax.set_xscale("log", base=2)
    ax.set_xticks(ms)
    ax.set_xticklabels([str(m) for m in ms])
    ax.set_xlim(min(ms) / 1.3, max(ms) * 1.3)
    ax.set_ylim(0, 1.0)
    ax.yaxis.set_major_formatter(matplotlib.ticker.PercentFormatter(1.0, decimals=0))


def util_handles(th, hw, sim):
    h = [Line2D([], [], color=th["FULL"], lw=2, ls=(0, (4, 2)), label="FULL 8×8"),
         Line2D([], [], color=th["STRIP"], lw=2, ls=(0, (4, 2)), label="STRIP 4×16"),
         Line2D([], [], ls="none", marker="o", markersize=8, markerfacecolor="none",
                markeredgecolor=th["model_edge"], markeredgewidth=1.5,
                label="MODEL (cycle model)")]
    if hw:
        h.append(Line2D([], [], ls="none", marker="o", markersize=8, color=th["muted"],
                        label="HW measured (counters)"))
    if sim_only(hw, sim):
        h.append(Line2D([], [], ls="none", marker="s", markersize=8, color=th["muted"],
                        label="SIM (co-simulation)"))
    h.append(Patch(facecolor=th["shade"], edgecolor="none", label="decode regime M ≤ 4"))
    return h


def plot_util_headline(th, model, hw, sim, facts):
    g = HEADLINE_GEMM
    keys = [(g, m, mode) for m in (1, 64) for mode in ("FULL", "STRIP")]
    tag, v = title_values(model, hw, sim, keys)
    f1, s1 = v[(g, 1, "FULL")], v[(g, 1, "STRIP")]
    f64, s64 = v[(g, 64, "FULL")], v[(g, 64, "STRIP")]
    facts["util_headline"] = {"source": tag, "gemm": g, "M1_FULL": f1, "M1_STRIP": s1,
                              "M1_ratio": s1 / f1, "M64_FULL": f64, "M64_STRIP": s64,
                              "M64_ratio": s64 / f64}
    fig, ax = new_fig(th)
    fig.subplots_adjust(left=0.08, right=0.98, top=0.78, bottom=0.20)
    style_axes(ax, th)
    draw_util(ax, th, model, g, hw, sim)
    ax.set_xlabel("M (rows of the activation matrix; log₂ scale)")
    ax.set_ylabel("utilization = mac_useful / (64 · cycles_total)")
    legend(ax, th, util_handles(th, hw, sim), loc="upper left")
    title_and_source(
        fig, th,
        f"{tag}: STRIP lifts decode utilization {s1 / f1:.2f}× at M=1; "
        f"the modes converge once M ≥ 8",
        f"{g} (K=64, N=256), int8 requant on. M=1: FULL {pct(f1)} → STRIP {pct(s1)}. "
        f"M=64: FULL {pct(f64)}, STRIP {pct(s64)} (STRIP drains 4 rows/tile vs 8)."
        + ("\n" + SIM_NOTE if tag == "SIM" else ""),
        source_line(model, [("HW", hw), ("SIM", sim)]))
    return fig


def plot_util_all(th, model, hw, sim, facts):
    tag, v = title_values(model, hw, sim,
                          [(g, 1, md) for g in GEMM_ORDER for md in ("FULL", "STRIP")])
    fig, axes = new_fig(th, w=11.0, h=7.4, ncols=2, nrows=2, sharex=True, sharey=True)
    fig.subplots_adjust(left=0.08, right=0.98, top=0.80, bottom=0.15, hspace=0.28,
                        wspace=0.08)
    ratios = {}
    for ax, g in zip(axes.flat, GEMM_ORDER):
        style_axes(ax, th)
        draw_util(ax, th, model, g, hw, sim, label_size=8)
        sh = model["sweep"]["points"]
        K = N = None
        for pnt in sh:
            if pnt["gemm"]["name"] == g:
                K, N = pnt["gemm"]["K"], pnt["gemm"]["N"]
                break
        ax.set_title(f"{g}  (K={K}, N={N})", color=th["text"], fontsize=11, loc="left")
        ratios[g] = v[(g, 1, "STRIP")] / v[(g, 1, "FULL")]
    for ax in axes[1]:
        ax.set_xlabel("M (log₂)")
    for ax in axes[:, 0]:
        ax.set_ylabel("utilization")
    legend(axes[1, 1], th, util_handles(th, hw, sim), loc="lower right")
    lo, hi = min(ratios.values()), max(ratios.values())
    facts["util_all_M1_ratio_range"] = {"source": tag, "range": [lo, hi]}
    title_and_source(
        fig, th,
        f"{tag}: on all four block matmuls, STRIP lifts M=1 utilization "
        f"{lo:.2f}–{hi:.2f}×",
        "One transformer block, d_model=64, int8 requant on. Dashed lines connect model "
        "points; shaded = decode regime (M ≤ 4)." + ("\n" + SIM_NOTE if tag == "SIM" else ""),
        source_line(model, [("HW", hw), ("SIM", sim)]))
    return fig


def plot_bench(th, model, hw, sim, facts):
    tag, v = title_values(model, hw, sim, [(g, m, md) for g in GEMM_ORDER for m in (1, 64)
                                      for md in ("FULL", "STRIP")])
    n_meas = n_model = 0
    measured = hw if hw else sim
    mlabel = "HW" if hw else ("SIM" if sim else None)
    fig, axes = new_fig(th, w=11.0, h=5.8, ncols=2, sharey=True)
    fig.subplots_adjust(left=0.07, right=0.98, top=0.80, bottom=0.19, wspace=0.06)
    width = 0.36
    for ax, M in zip(axes, (1, 64)):
        style_axes(ax, th)
        ax.grid(True, axis="y")
        ax.grid(False, axis="x")
        for i, g in enumerate(GEMM_ORDER):
            for j, mode in enumerate(("FULL", "STRIP")):
                x = i + (j - 0.5) * (width + 0.04)
                mv = model["points"][(g, M, mode)]["util"]
                meas = measured.get((g, M, mode)) if measured else None
                if meas is not None:
                    ax.bar(x, meas["util"], width, color=th[mode], zorder=2)
                    ax.plot([x - width / 2, x + width / 2], [mv, mv],
                            color=th["model_edge"], linewidth=2.2, zorder=3)
                    val = meas["util"]
                    n_meas += 1
                else:
                    # MODEL only: hollow bar in the mode color, so it can never be
                    # mistaken for a measured (filled) bar.
                    ax.bar(x, mv, width, facecolor="none", edgecolor=th[mode],
                           linewidth=1.8, hatch="////", zorder=2)
                    val = mv
                    n_model += 1
                ax.annotate(pct(val), (x, val), textcoords="offset points", xytext=(0, 3),
                            ha="center", fontsize=8, color=th["text"])
        ax.set_xticks(range(len(GEMM_ORDER)))
        ax.set_xticklabels(GEMM_ORDER)
        ax.set_title(f"M = {M}  ({'decode' if M == 1 else 'prefill'})", color=th["text"],
                     fontsize=11, loc="left")
        ax.set_ylim(0, 1.0)
        ax.yaxis.set_major_formatter(matplotlib.ticker.PercentFormatter(1.0, decimals=0))
    axes[0].set_ylabel("utilization")
    handles = []
    if n_meas:
        handles += [Patch(color=th["FULL"], label=f"FULL ({mlabel})"),
                    Patch(color=th["STRIP"], label=f"STRIP ({mlabel})"),
                    Line2D([], [], color=th["model_edge"], lw=2.2, label="MODEL tick")]
    if n_model:
        handles += [Patch(facecolor="none", edgecolor=th["FULL"], hatch="////", lw=1.8,
                          label="FULL (MODEL only)"),
                    Patch(facecolor="none", edgecolor=th["STRIP"], hatch="////", lw=1.8,
                          label="STRIP (MODEL only)")]
    legend(axes[1], th, handles, loc="upper left")
    r1 = [v[(g, 1, "STRIP")] / v[(g, 1, "FULL")] for g in GEMM_ORDER]
    r64 = [v[(g, 64, "STRIP")] / v[(g, 64, "FULL")] for g in GEMM_ORDER]
    facts["bench_ratio_M1"] = {"source": tag, "range": [min(r1), max(r1)]}
    facts["bench_ratio_M64"] = {"source": tag, "range": [min(r64), max(r64)]}
    title_and_source(
        fig, th,
        f"{tag}: STRIP pays at decode ({min(r1):.2f}–{max(r1):.2f}×), "
        f"barely moves prefill ({min(r64):.3f}–{max(r64):.3f}×)",
        "Benchmark GEMMs (SPEC §1.1), int8 requant on. The prefill edge is drain length "
        "(4 vs 8 rows per tile), not mapping." + ("\n" + SIM_NOTE if tag == "SIM" else ""),
        source_line(model, [("HW", hw), ("SIM", sim)]))
    return fig


def plot_roofline(th, model, hw, sim, facts):
    roof = model["roof"]
    peak = roof["roofs"]["compute_macs_per_cycle"]
    bws = {b["mode"]: b["bytes_per_cycle"] for b in roof["roofs"]["bandwidth"]}
    pts = roof["points"]
    worst_frac, n_above = 1.0, 0
    for q in pts:
        r = min(peak, bws[q["mode"]] * q["arithmetic_intensity"])
        if q["macs_per_cycle"] > r + 1e-9:
            n_above += 1
        worst_frac = min(worst_frac, q["macs_per_cycle"] / r)
    if n_above:
        sys.exit(f"roofline.json: {n_above} model points above their roof; refusing to plot")
    ai = {(q["name"], q["M"], q["mode"]): q["arithmetic_intensity"] for q in pts}
    g = HEADLINE_GEMM
    facts["roofline"] = {"peak": peak, "bw": bws, "points_above_roof": n_above,
                         "worst_fraction_of_roof": worst_frac,
                         "ai_M1": {m: ai[(g, 1, m)] for m in ("FULL", "STRIP")}}
    tag = "MODEL"  # intensities, roofs and the below-roof check are model quantities
    fig, ax = new_fig(th)
    fig.subplots_adjust(left=0.08, right=0.98, top=0.82, bottom=0.20)
    style_axes(ax, th)
    xs_all = [q["arithmetic_intensity"] for q in pts]
    x0, x1 = min(xs_all) / 1.6, max(xs_all) * 1.6
    xx = [x0 * (x1 / x0) ** (i / 200) for i in range(201)]
    for mode in ("FULL", "STRIP"):
        ax.plot(xx, [min(peak, bws[mode] * x) for x in xx], color=th[mode], linewidth=1.4,
                alpha=0.9, zorder=2)
        xk = peak / bws[mode]
        ax.axvline(xk, color=th[mode], linewidth=0.6, linestyle=":", zorder=1)
    ax.axhline(peak, color=th["muted"], linewidth=1.2, zorder=2)
    ax.annotate(f"compute roof {peak} MAC/cycle (8×8 PEs)", (x1 / 1.05, peak),
                textcoords="offset points", xytext=(0, 4), ha="right", fontsize=8.5,
                color=th["text"])
    for mode, marker in (("FULL", "o"), ("STRIP", "D")):
        sel = [q for q in pts if q["mode"] == mode]
        ax.plot([q["arithmetic_intensity"] for q in sel], [q["macs_per_cycle"] for q in sel],
                linestyle="none", marker=marker, markersize=8, markerfacecolor="none",
                markeredgecolor=th[mode], markeredgewidth=1.5, zorder=4)
    # Measured overlay: MACs/cycle from counters at the model's arithmetic intensity
    # (intensity is a property of the shape and mode, not of the measurement).
    for recs, mk in ((hw, "o"), (sim_only(hw, sim), "s")):
        for (name, M, mode), r in recs.items():
            if (name, M, mode) in ai:
                ax.plot(ai[(name, M, mode)], r["util"] * peak, linestyle="none", marker=mk,
                        markersize=4.5, color=th[mode], markeredgewidth=0, zorder=5)
    for mode in ("FULL", "STRIP"):
        q = next(q for q in pts if q["name"] == g and q["M"] == 1 and q["mode"] == mode)
        ax.annotate(f"{g} M=1 {mode}", (q["arithmetic_intensity"], q["macs_per_cycle"]),
                    textcoords="offset points", xytext=(9, -12 if mode == "FULL" else 6),
                    fontsize=8.5, color=th["text"])
    ax.set_xscale("log", base=2)
    ax.set_yscale("log", base=2)
    ax.set_xlim(x0, x1)
    ax.set_ylim(2, peak * 1.8)
    ax.set_xlabel("arithmetic intensity (useful MACs per operand byte streamed, log₂)")
    ax.set_ylabel("achieved MACs/cycle (log₂)")
    handles = [Line2D([], [], color=th["FULL"], lw=1.4,
                      label=f"FULL operand roof {bws['FULL']} B/cycle"),
               Line2D([], [], color=th["STRIP"], lw=1.4,
                      label=f"STRIP operand roof {bws['STRIP']} B/cycle"),
               Line2D([], [], ls="none", marker="o", markersize=8, markerfacecolor="none",
                      markeredgecolor=th["FULL"], markeredgewidth=1.5, label="FULL (MODEL)"),
               Line2D([], [], ls="none", marker="D", markersize=8, markerfacecolor="none",
                      markeredgecolor=th["STRIP"], markeredgewidth=1.5, label="STRIP (MODEL)")]
    if hw:
        handles.append(Line2D([], [], ls="none", marker="o", markersize=7, color=th["muted"],
                              label="HW measured"))
    if any(k in ai for k in sim_only(hw, sim)):
        handles.append(Line2D([], [], ls="none", marker="s", markersize=7, color=th["muted"],
                              label="SIM"))
    legend(ax, th, handles, loc="lower right")
    a_f, a_s = ai[(g, 1, "FULL")], ai[(g, 1, "STRIP")]
    title_and_source(
        fig, th,
        f"{tag}: STRIP moves decode right ({a_f:g} → {a_s:g} MAC/B for {g}) at "
        f"{bws['STRIP']} vs {bws['FULL']} B/cycle",
        f"4 GEMMs × M=1..64. All points below their roof (lowest {100 * worst_frac:.0f}%): "
        f"the gap is per-tile setup/flush/drain (model assumes 0 stalls).",
        source_line(model, [("HW", hw), ("SIM", sim)]))
    return fig


def plot_cost(th, model, cost, facts):
    """Only drawn when results/cost.json exists. No placeholder image is produced.
    Left: whole-design resources, FULL-only build vs split build. Right: where the
    LUT/FF/DSP delta lives (cost.json breakdown.delta), so the 16-wide requant path is
    not silently charged to the array muxes."""
    a, b = cost["full_only"], cost["split"]
    delta = cost.get("breakdown", {}).get("delta", {})
    fig = plt.figure(figsize=(11.0, 5.8), facecolor=th["bg"])
    gs = fig.add_gridspec(1, 2, left=0.07, right=0.98, top=0.68, bottom=0.15, wspace=0.28,
                          width_ratios=[1.0, 1.15])
    ax = fig.add_subplot(gs[0])
    style_axes(ax, th)
    ax.grid(False, axis="x")
    res = [("lut", "LUT"), ("ff", "FF"), ("dsp", "DSP48E1"), ("bram", "BRAM")]
    # Normalize each resource to its FULL-only value so all four share one axis.
    w = 0.36
    for i, (k, label) in enumerate(res):
        for j, (build, col) in enumerate(((a, th["FULL"]), (b, th["STRIP"]))):
            x = i + (j - 0.5) * (w + 0.04)
            val = build[k] / a[k] if a[k] else 0.0
            ax.bar(x, val, w, color=col, zorder=2)
            ax.annotate(f"{build[k]:,}", (x, val), textcoords="offset points", xytext=(0, 3),
                        ha="center", fontsize=8, color=th["text"])
    ax.set_xticks(range(len(res)))
    ax.set_xticklabels([r[1] for r in res])
    ax.set_ylim(0, 1.95)
    ax.set_ylabel("relative to the FULL-only build")
    ax.set_title("whole design (post-route)", color=th["text"], fontsize=11, loc="left")
    legend(ax, th, [Patch(color=th["FULL"], label="ENABLE_STRIP=0 (FULL only)"),
                    Patch(color=th["STRIP"], label="ENABLE_STRIP=1 (split)")],
           loc="upper right")
    ax2 = fig.add_subplot(gs[1])
    style_axes(ax2, th)
    ax2.grid(False, axis="y")
    blocks = [k for k in ("array", "requant", "sequencer") if k in delta]
    other = {k: delta["total"][k] - sum(delta[bk][k] for bk in blocks)
             for k in ("lut", "ff")} if "total" in delta else None
    rows = [(bk, delta[bk]) for bk in blocks]
    if other is not None:
        rows.append(("everything else", {"lut": other["lut"], "ff": other["ff"], "dsp": 0}))
    names = {"array": "array muxes", "requant": "requant 8–15", "sequencer": "sequencer"}
    ys = list(range(len(rows)))[::-1]
    h = 0.36
    for y, (bk, d) in zip(ys, rows):
        ax2.barh(y + h / 2 + 0.02, d["lut"], h, color=th["muted"], zorder=2)
        ax2.barh(y - h / 2 - 0.02, d["ff"], h, color=th["grid"], edgecolor=th["muted"],
                 linewidth=0.8, zorder=2)
        txt = f"+{d['lut']:,} LUT" if d["lut"] >= 0 else f"{d['lut']:,} LUT"
        if d.get("dsp"):
            txt += f", +{d['dsp']} DSP"
        ax2.annotate(txt, (max(d["lut"], 0), y + h / 2), textcoords="offset points",
                     xytext=(4, -3), fontsize=8.5, color=th["text"], va="center")
        ax2.annotate(f"{d['ff']:+,} FF", (max(d["ff"], 0), y - h / 2), textcoords="offset points",
                     xytext=(4, -3), fontsize=8.5, color=th["text"], va="center")
    ax2.set_yticks(ys)
    ax2.set_yticklabels([names.get(r[0], r[0]) for r in rows], color=th["text"])
    ax2.set_xlabel("split − FULL-only")
    xmax = max(max(d["lut"], d["ff"]) for _, d in rows)
    ax2.set_xlim(min(0, min(min(d["lut"], d["ff"]) for _, d in rows)) * 1.2, xmax * 1.45)
    ax2.set_title("where the delta lives (LUT, FF)", color=th["text"], fontsize=11,
                  loc="left")
    dl, df, dd = b["lut"] - a["lut"], b["ff"] - a["ff"], b["dsp"] - a["dsp"]
    mux_lut = delta.get("array", {}).get("lut", 0) + delta.get("sequencer", {}).get("lut", 0)
    facts["cost"] = {"source": cost.get("source"), "git_hash": cost.get("git_hash"),
                     "delta_lut": dl, "delta_lut_pct": dl / a["lut"], "delta_ff": df,
                     "delta_dsp": dd, "array_plus_sequencer_lut": mux_lut,
                     "fmax_full_only": a["fmax_mhz"], "fmax_split": b["fmax_mhz"]}
    wp = cost.get("worst_path", {}).get("split", {})
    wpa = cost.get("worst_path", {}).get("full_only", {})
    meets = ("Both builds meet timing (WNS ≥ 0)." if a.get("wns_ns", -1) >= 0
             and b.get("wns_ns", -1) >= 0 else "A build FAILS timing (WNS < 0).")
    placement = (wp.get("logic_levels") == 0 and wp.get("route_pct", 0) >= 90)
    why = ("a 0-logic-level net is placement, not STRIP logic, so the Fmax delta is not a "
           "clean cost of splitting." if placement else
           "check this path before attributing the Fmax delta to splitting.")
    fig.text(0.012, 0.975,
             f"BUILD: the split costs +{dl:,} LUT (+{100 * dl / a['lut']:.1f}%) and +{dd} DSP; "
             f"only +{mux_lut:,} LUT is muxing",
             ha="left", va="top", fontsize=14, fontweight="bold", color=th["text"])
    fig.text(0.012, 0.915,
             "The FULL-only build also drops requant lanes 8–15 (FULL drains 8 lanes), so most "
             "of the delta is STRIP's 16-wide output path.\n"
             f"{meets} Fmax est. {a['fmax_mhz']} → {b['fmax_mhz']} MHz (1000/(period − WNS) "
             f"at {cost.get('clock_mhz', 100):g} MHz, not a sweep).\n"
             f"Split worst path: {wp.get('source', '?').rsplit('/', 1)[0]}"
             f" → {wp.get('destination', '?').rsplit('/', 1)[0]}, {wp.get('logic_levels', '?')} logic "
             f"levels, {wp.get('route_pct', 0):.0f}% routing (FULL-only: "
             f"{wpa.get('logic_levels', '?')} levels, {wpa.get('route_pct', 0):.0f}% routing):\n"
             f"{why}",
             ha="left", va="top", fontsize=9.5, color=th["muted"])
    fig.text(0.012, 0.015,
             f"source: BUILD ({cost.get('source', '?')}) results/cost.json, git {cost.get('git_hash')}, "
             f"{cost.get('part', '')}, Vivado {cost.get('vivado', '')}",
             ha="left", va="bottom", fontsize=8.5, color=th["muted"], family="monospace")
    return fig


# --------------------------------------------------------------------------- driver


def save(fig, outdir, name, theme, written):
    for ext in ("svg", "png"):
        p = os.path.join(outdir, f"{name}_{theme}.{ext}")
        md = {"Date": None} if ext == "svg" else {"Software": None}
        fig.savefig(p, format=ext, dpi=150 if ext == "png" else None,
                    facecolor=fig.get_facecolor(), metadata=md)
        written.append(p)
    plt.close(fig)


def build(outdir, measured=("hw", "sim"), include_untracked=False):
    RECORD_STATS.clear()
    matplotlib.rcParams.update({
        "svg.hashsalt": "splitsys", "svg.fonttype": "path", "font.family": "DejaVu Sans",
        "hatch.linewidth": 0.6, "path.simplify": False,
    })
    model = load_model()
    hw, hw_in = load_measured("hw", include_untracked) if "hw" in measured else ({}, [])
    sim, sim_in = load_measured("sim", include_untracked) if "sim" in measured else ({}, [])
    cost, cost_in = load_cost()
    for label, recs in (("hw", hw), ("sim", sim)):
        for k, r in recs.items():
            if r["model_git_hash"] and r["model_git_hash"] != model["hash"]:
                print(f"[make_plots] warning: {r['file']} model_git_hash "
                      f"{r['model_git_hash']} != results/model {model['hash']}",
                      file=sys.stderr)
    os.makedirs(outdir, exist_ok=True)
    facts, written = {}, []
    for theme, th in THEMES.items():
        matplotlib.rcParams["hatch.color"] = th["muted"]
        save(plot_util_headline(th, model, hw, sim, facts), outdir, "util_vs_m", theme, written)
        save(plot_util_all(th, model, hw, sim, facts), outdir, "util_vs_m_all", theme, written)
        save(plot_bench(th, model, hw, sim, facts), outdir, "bench_bars", theme, written)
        save(plot_roofline(th, model, hw, sim, facts), outdir, "roofline", theme, written)
        if cost is not None:
            save(plot_cost(th, model, cost, facts), outdir, "cost", theme, written)
    manifest = {
        "generator": "results/plots/make_plots.py",
        "matplotlib": matplotlib.__version__,
        "model_git_hash": model["hash"],
        "latencies_provisional": model["provisional"],
        "measured_requested": list(measured),
        "sources_present": ["model"] + (["hw"] if hw else []) + (["sim"] if sim else [])
        + (["cost"] if cost else []),
        "inputs": [{"path": p, "sha256": h} for p, h in model["inputs"] + hw_in + sim_in
                   + cost_in],
        "headline_facts": json.loads(json.dumps(facts), parse_float=lambda s: round(float(s), 6)),
        "measured_records": {k: {"records": v["records"], "counter_diffs": v["counter_diffs"],
                                 "builds": sorted(v["builds"])}
                             for k, v in sorted(RECORD_STATS.items())},
        "plots": sorted(os.path.basename(p) for p in written),
        "missing": ([] if hw else ["results/hw/*.json (no hardware runs yet)"])
        + ([] if cost else ["results/cost.json (cost plot not drawn)"]),
    }
    with open(os.path.join(outdir, "manifest.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(manifest, f, indent=2, sort_keys=True)
        f.write("\n")
    written.append(os.path.join(outdir, "manifest.json"))
    return written


def digest_dir(paths):
    return {os.path.basename(p): sha256(p) for p in paths}


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", default=os.path.join(ROOT, "results", "plots"))
    ap.add_argument("--measured", default="hw,sim",
                    help="comma list of measured sources to overlay: hw, sim (default: both). "
                         "SIM points are drawn as squares and labeled SIM.")
    ap.add_argument("--include-untracked", action="store_true",
                    help="also use measured records not yet committed to git (preview only)")
    ap.add_argument("--check", action="store_true",
                    help="also regenerate into a temp dir and require byte-identical output")
    args = ap.parse_args()
    measured = tuple(x for x in args.measured.split(",") if x)
    bad = [x for x in measured if x not in ("hw", "sim")]
    if bad:
        ap.error(f"unknown --measured source(s): {bad}")
    written = build(args.out, measured, args.include_untracked)
    for p in written:
        print(rel(p))
    if args.check:
        # Project rule: scratch output under ~/.cache, never /tmp.
        cache = os.path.join(os.path.expanduser("~"), ".cache")
        os.makedirs(cache, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="splitsys-plots-", dir=cache) as td:
            again = build(td, measured, args.include_untracked)
            a, b = digest_dir(written), digest_dir(again)
            diff = sorted(k for k in a if a[k] != b.get(k))
            if diff or set(a) != set(b):
                print(f"[make_plots] NOT deterministic: {diff}", file=sys.stderr)
                return 1
        print(f"[make_plots] deterministic: {len(written)} files byte-identical across 2 runs")
    return 0


if __name__ == "__main__":
    sys.exit(main())
