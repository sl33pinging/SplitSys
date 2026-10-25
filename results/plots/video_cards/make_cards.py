#!/usr/bin/env python3
"""Write the SplitSys video cards as self-contained 1920x1080 HTML files.

Render each to PNG with scripts/screenshot (see docs/VIDEO.md section g).
No external URLs, system fonts only, dashboard palette from docs/DEMO.md section 3.
Every number below is cited in docs/VIDEO.md "Card sources".
Owner: demo-storyteller.
"""
import os

HERE = os.path.dirname(os.path.abspath(__file__))

CSS = """
*{box-sizing:border-box;margin:0;padding:0}
html,body{width:1920px;height:1080px;background:#0B0D10;color:#E8ECF1;overflow:hidden;
 font-family:ui-sans-serif,system-ui,"Segoe UI",sans-serif}
.card{position:absolute;inset:0;padding:90px 140px 150px 140px;display:flex;flex-direction:column;justify-content:center}
.mono{font-family:ui-monospace,"Cascadia Mono",Consolas,monospace}
.kicker{font-size:34px;color:#9AA4AF;letter-spacing:.04em;text-transform:uppercase;margin-bottom:28px}
h1{font-size:150px;font-weight:700;line-height:1.05;letter-spacing:-.01em}
h2{font-size:76px;font-weight:700;line-height:1.1}
.sub{font-size:44px;color:#9AA4AF;margin-top:34px;line-height:1.3}
.big{font-size:230px;font-weight:700;line-height:1;color:#FF9F43}
.full{color:#4C9AFF}.strip{color:#FF9F43}.pass{color:#3DDC97}.dim{color:#9AA4AF}
.src{position:absolute;left:140px;right:140px;bottom:44px;font-size:24px;color:#9AA4AF;
 border-top:1px solid #262C33;padding-top:16px;line-height:1.35}
.badge{display:inline-block;font-size:26px;padding:4px 14px;border:2px solid #3DDC97;color:#3DDC97;
 border-radius:6px;margin:0 14px;vertical-align:middle;font-family:ui-monospace,"Cascadia Mono",Consolas,monospace}
.row{display:flex;gap:36px;margin-top:56px}
.panel{background:#14181D;border:1px solid #262C33;border-radius:12px;padding:30px 34px;flex:1}
.panel .t{font-size:32px;color:#9AA4AF}
.panel .v{font-size:52px;font-weight:700;margin-top:8px}
.panel .d{font-size:36px;color:#E8ECF1;margin-top:8px}
ul{list-style:none;margin-top:40px}
li{font-size:40px;line-height:1.3;margin:0 0 24px 0;padding-left:48px;position:relative}
li:before{content:"";position:absolute;left:0;top:24px;width:22px;height:4px;background:#FF9F43}
.grid{display:grid;grid-template-columns:repeat(8,78px);grid-gap:8px;flex:none}
.pe{width:78px;height:78px;border-radius:6px;background:#1A1F25;border:1px solid #262C33}
.pe.on{background:#4C9AFF;border-color:#4C9AFF}
.ph{color:#0B0D10;background:#FF9F43;padding:0 18px;border-radius:8px}
"""


def page(name, body, source):
    html = ("<!doctype html><html><head><meta charset='utf-8'><title>" + name + "</title><style>" + CSS +
            "</style></head><body><div class='card'>" + body + "</div><div class='src'>" + source +
            "</div></body></html>")
    with open(os.path.join(HERE, name + ".html"), "w", encoding="utf-8") as f:
        f.write(html)


# 01 title
page("01_title", """
<div class='kicker'>Cal Hacks 13.0 &middot; built solo</div>
<h1>SplitSys</h1>
<div class='sub' style='font-size:62px;color:#E8ECF1;margin-top:30px'>a splittable systolic array for LLM decode</div>
<div class='sub'>an 8&times;8 int8 array on a Spartan-7 FPGA that rewires itself into 4&times;16 at runtime</div>
""", "Real hardware: RealDigital Urbana board (Spartan-7). Every number in this video comes from committed files in results/.")

# 02 idle rows
pes = "".join("<div class='pe%s'></div>" % (" on" if r == 0 else "") for r in range(8) for c in range(8))
page("02_idle_rows", """
<div style='display:flex;gap:100px;align-items:center'>
 <div>
  <div class='kicker'>LLM decode &middot; M = 1</div>
  <h2>One token at a time<br>&rarr; 7 of 8 rows idle</h2>
  <div class='sub'>Writing one token, each matrix multiply is<br>a single row tall. In the square 8&times;8 array<br>only row 0 has work; rows 1&ndash;7 sit dark.</div>
  <div class='sub' style='font-size:40px'>Best case 1/8 = <span class='mono'>12.5%</span> utilization.<br>Measured on <span class='mono'>ffn_up</span>: <span class='mono full'>6.95%</span><span class='badge'>HW</span></div>
 </div>
 <div class='grid'>""" + pes + """</div>
</div>
""", "Ceiling: docs/SPEC.md section 4 (M/8 at M=1). 6.95% = results/hw/2026-10-24T20-40-41Z-*-m1-full.json, derived.util 0.069527 (hardware counters, build 0x52A66216).")

# 03 stat
page("03_stat", """
<div class='kicker'>Same 64 multipliers, rewired: FULL 8&times;8 &rarr; STRIP 4&times;16</div>
<div class='big mono'>2.02&ndash;2.07&times;</div>
<div class='sub' style='color:#E8ECF1;font-size:54px;margin-top:20px'>utilization at M = 1, measured on the board<span class='badge'>HW</span></div>
<div class='row'>
 <div class='panel'><div class='t mono'>qkv</div><div class='v mono'><span class='full'>6.95%</span> &rarr; <span class='strip'>14.39%</span></div><div class='d mono'>2.07&times;</div></div>
 <div class='panel'><div class='t mono'>attn_out</div><div class='v mono'><span class='full'>6.94%</span> &rarr; <span class='strip'>14.35%</span></div><div class='d mono'>2.07&times;</div></div>
 <div class='panel'><div class='t mono'>ffn_up</div><div class='v mono'><span class='full'>6.95%</span> &rarr; <span class='strip'>14.40%</span></div><div class='d mono'>2.07&times;</div></div>
 <div class='panel'><div class='t mono'>ffn_down</div><div class='v mono'><span class='full'>10.41%</span> &rarr; <span class='strip'>21.09%</span></div><div class='d mono'>2.02&times;</div></div>
</div>
""", "results/hw/2026-10-24T20-40-{20,33,41,59}Z-*-m1-{full,strip}.json, derived.util from hardware counters, build 0x52A66216. Utilization inside the array; not a speed claim against a CPU or GPU.")

# 04 model exact
page("04_model_exact", """
<div class='kicker'>A cycle model I wrote before the board session</div>
<h2 style='font-size:96px'><span class='mono'>773</span> hardware GEMMs<br>&middot; <span class='mono'>12</span> counters each<br>&middot; <span class='mono pass'>0</span> mismatches</h2>
<div class='sub'>0 of 9,276 counter values differ from the model&rsquo;s prediction. No calibration.<br>Every output is also bit-exact vs a Rust golden model: 0 output mismatches.</div>
<div class='sub' style='font-size:36px'><span class='badge' style='margin-left:0'>HW</span>3 bitstreams, same datapath &middot; 393,129,309 MACs checked &middot; 0 stall cycles</div>
""", "773 git-tracked GEMM records in results/hw/*.json (KG1 0x52A66216: 272, KG2 0x38A3950A: 301, KG3 0x21DD6850: 200): counters vs model fields, check.mismatches, cycles_stall. Model: Rust crate @ 72da652.")

# 05 LLM
page("05_llm", """
<div class='kicker'>Live: a real (tiny) public language model</div>
<h2 style='font-size:88px'>stories260K &mdash; every linear<br>layer on the FPGA</h2>
<div class='row'>
 <div class='panel'><div class='t'>On the FPGA</div><div class='v' style='font-size:42px;line-height:1.3'>q, k, v, o and the 3 FFN matrices in all 5 layers, plus the classifier<br><span class='dim' style='font-size:34px'>37 int8 GEMMs per token, M = 1</span></div></div>
 <div class='panel'><div class='t'>On the laptop</div><div class='v' style='font-size:42px;line-height:1.3'>attention scores and softmax, RMSNorm, RoPE, SwiGLU, sampling</div></div>
</div>
<div class='sub' style='font-size:40px'><span class='mono'>~2 s/token</span> laptop wall time, mostly the serial link &middot; on-chip <span class='mono'>295 &micro;s</span>/token<span class='badge'>HW</span></div>
""", "results/hw/lm/2026-10-25T04-59-16Z-lm.json (KG3, 1,562,500 baud, 1 ms FTDI timer): wall_s_per_step 2.16, cycles_per_step 29,540 at 100 MHz, 444/444 GEMMs bit-exact; split field. stories260K: karpathy/tinyllamas (MIT).")

# 06 limits
page("06_limits", """
<div class='kicker'>Honest limits</div>
<h2>What this is not</h2>
<ul>
 <li>Not a speed claim vs CPU or GPU: 64 int8 MACs at 100 MHz, and ~2 s/token is bound by the serial link.</li>
 <li>Only the linear layers run on the FPGA. Attention, softmax and norms run on the laptop.</li>
 <li>stories260K is a tiny model: dim 64, 5 layers, vocab 512.</li>
 <li>STRIP isn&rsquo;t always faster: FULL won on 31 of 97 random shapes (up to 1.32&times;), so the cycle model picks the mode per job.</li>
 <li>The split costs +2,243 LUTs (+43.6%), +16 DSPs and 25% more operand bandwidth.</li>
</ul>
""", "31/97: H-5 random pairs in results/hw (KG1), worst M=22 K=250 N=17, 3,566 vs 2,711 cycles. Cost: results/cost.json (post-route, both builds). 20 vs 16 B/cycle: docs/SPEC.md. Model size: results/hw/lm model field.")

# 07 end
page("07_end", """
<h1>SplitSys</h1>
<div class='sub' style='font-size:60px;color:#E8ECF1'>Zhicheng (Daniel) Song · built solo</div>
<div class='sub' style='font-size:44px'>Code, RTL and every hardware log: GitHub link on the Devpost page</div>
<div class='sub' style='margin-top:60px'>Designed, verified and brought up solo at Cal Hacks 13.0</div>
""", "")

print("wrote 7 cards to", HERE)
