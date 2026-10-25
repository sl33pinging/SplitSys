# SplitSys demo video kit (2:00 cut, 60 s cut, filming and assembly)

Owner: demo-storyteller. This file, `docs/VIDEO_captions.srt` and `results/plots/video_cards/*` are the video kit.
The human films the board on a phone and records the screen; everything else is here.
All voiceover is solo, first person. Numbers are cited in **Card and voiceover sources** at the end.

**A correction to `docs/WINNERS_RESEARCH.md` §3/§4.** The research draft says that FULL is the right shape for large M. The committed
data says otherwise, so the video doesn't claim it:
- On all four benchmark shapes **STRIP is still ahead at M=64** (for example `ffn_up` 57.65% vs 55.65%, 28,418 vs 29,442 cycles). The gap is drain: 4 rows per tile against 8.
- **FULL was faster on 31 of 97 random shapes** (up to 1.32×), because 4×16 tiles pad worse than 8×8 for some N.
- That is why **auto mode** exists: the cycle model picks the mode for each job.

---

## (a) Final ~2:00 shot list with voiceover

Spoken words: about 270 (273 in this table, 267 in the SRT, which writes some numbers as digits), under the 280-word limit, about 2:00 at ~140 words per minute. Codes: **B** = phone shot (section c), **S** = screen capture (section d), **C** = card PNG in `results/plots/video_cards/`.

| Time | Picture | On-screen label (Clipchamp text, top-left) | Voiceover (exact) |
|---|---|---|---|
| 0:00–0:08 | **B1** power switch flip, cold boot, LEDs and 7-seg come up. Real time, no cut. | `Real time · boots from flash` | "This board is an AI accelerator I built solo. It just booted itself from its own flash." |
| 0:08–0:12 | **C 01_title.png** (or **B5** face-to-camera, see note 1) | — | "It's called SplitSys." |
| 0:12–0:16 | **B3** wide: board and monitor together, story text appearing | `Live on the board` | "It's running a real public language model, stories260K." |
| 0:16–0:26 | **S4** Scene 4 generation, cut in 2–3 s of **B2** (LEDs and 7-seg flicker) | `Live · 2× speed` if you speed it up (note 2) | "Every linear layer runs on this chip; attention and softmax stay on the laptop. About two seconds per token, and almost all of that is the serial link." |
| 0:26–0:30 | **C 05_llm.png** | — | "On-chip, a token takes 295 microseconds." |
| 0:30–0:38 | **C 02_idle_rows.png** | — | "When a model writes one token at a time, each matrix multiply is a single row tall. In a square eight-by-eight array, seven of eight rows sit idle." |
| 0:38–0:54 | **S1** Scene 1: FULL run at M=1 (one lit row), then the STRIP run (two orange slabs). Real time. | `Live · ffn_up M=1` | "Here's FULL mode at decode size: only the top row lights up. 6.95 percent utilization, from the hardware counters. Now the same 64 multipliers rewire at runtime into four by sixteen. The bottom half computes more output columns: 14.40 percent." |
| 0:54–1:00 | **C 03_stat.png** | — | "Across all four transformer matmuls, that's 2.02 to 2.07 times, measured on the board." |
| 1:00–1:08 | **S2** Scene 2 model-vs-measured table and curves (hollow model markers on measured points) | `Model vs measured` | "Before the board session, I wrote a cycle model of the chip. On 773 hardware runs, it predicted all twelve counters exactly," |
| 1:08–1:14 | **C 04_model_exact.png** | — | "and every output matched my golden model bit for bit." |
| 1:14–1:32 | **S2** continued: the M=64 benchmark bars, then the auto-mode panel | `Auto mode: the model picks` | "The split isn't always better. At M equals 64, STRIP is only slightly ahead on these benchmarks, and on 31 of 97 random shapes FULL was faster, because four-by-sixteen tiles pad worse. So the model picks the mode for every job." |
| 1:32–1:42 | **S3** Scene 3 architecture and roofline | `Cost of splitting` | "It costs 25 percent more operand bandwidth and 2,243 more lookup tables, mostly a wider requantizer. I measured that too." |
| 1:42–1:50 | **B4** finger presses BTN0, then LED0 and LED1 light | `Self-test · real time` | "A button press runs the on-board self-test: one FULL and one STRIP job." |
| 1:50–1:56 | **C 06_limits.png** | — | "This isn't a speed claim against a CPU or GPU. It's about making every multiplier count." |
| 1:56–2:00 | **C 07_end.png** (placeholders filled) | — | "SplitSys. Built solo at Cal Hacks 13.0." |

Notes:
1. **B5 option.** If you film a face-to-camera shot, use it at 0:08–0:12 instead of the title card, and say "I'm ⟦name⟧, and I built SplitSys solo." Then show the title card under B3 for one second as a lower-third, or skip it.
2. **Speed labels.** Scene 4 at 8 tokens is about 26 s of real time (12 steps at 2.16 s; recorded run 2026-10-25T04-59-16Z). The slot is 10 s. Either cut to the last 10 s (a cut, not a speed change; no label needed), or speed it up and put `2× speed` on screen for the whole sped-up span. Never speed up B1, S1 or B4; they are the "real time" proof.
3. Say "measured" or "model" for every number, as written. Don't ad-lib other numbers.
4. If any shot is from REPLAY (amber banner on the dashboard), keep the banner visible and add `REPLAY · recorded` to the label.

## (b) 60-second cut

Same footage and cards, trimmed. 111 spoken words.

| Time | Picture | Voiceover |
|---|---|---|
| 0:00–0:06 | B1 cold boot (real time) | "This is an AI accelerator I built solo. It boots from its own flash." |
| 0:06–0:18 | B3 into S4 (generation; `2× speed` label if sped up), B2 cut-in | "It runs a real language model: every linear layer on this chip, about two seconds a token, almost all of it the serial link." |
| 0:18–0:24 | C 02_idle_rows | "One token at a time leaves seven of eight rows idle." |
| 0:24–0:36 | S1 FULL then STRIP at M=1 | "So it rewires itself from eight by eight into four by sixteen. Utilization goes from 6.95 to 14.40 percent, measured." |
| 0:36–0:42 | C 03_stat | "2.02 to 2.07 times across four transformer matmuls." |
| 0:42–0:50 | C 04_model_exact | "My cycle model predicted every hardware counter exactly, on 773 runs." |
| 0:50–0:56 | C 06_limits | "FULL still wins on about a third of random shapes, so the model picks. No speed claims." |
| 0:56–1:00 | C 07_end | "SplitSys. Built solo at Cal Hacks." |

## (c) Phone shot checklist (the human films these)

**Phone settings, all shots:** landscape; 1080p at 30 fps (4K if storage allows; Clipchamp exports at 1080p either way); no zoom (move the phone closer instead; digital zoom blurs the 7-seg); prop the phone against something solid or use a stand, never hand-held for close-ups; tap the board on screen and **long-press to lock focus and exposure** (AE/AF lock), then drag the exposure slider down a step so LEDs don't bloom; turn off the room light directly above the board if it reflects off the PCB; turn off Live Photo, HDR video and beauty filters. Record 2–3 s of extra footage before and after each action. Keep the room quiet; the voiceover goes on later, so the ambient sound will be muted.

| Shot | Frame | Length | How |
|---|---|---|---|
| **B1** power-on cold boot | Close-up of the whole board, power switch at one edge of frame, LEDs and 7-seg visible. USB cable in, laptop host **not** running. | ~8 s, real time, one take, no cuts | Lock exposure on the board with power **on** first (LEDs lit), then switch it off and start recording. Flip the switch with a finger that enters from the side and leaves the frame. Check J14 is fitted. Film 2–3 takes; keep the cleanest. |
| **B2** LEDs and 7-seg during generation | Tight on LED2/LED3 (busy, UART) and the 7-segment display. The display text must be readable in the video. | 10–15 s | Start Scene 4 generation (section d, S4), then film. The 7-seg shows the mode letter and the last run's utilization, for example `S 14.4`. Film what it actually shows; don't stage a value. |
| **B3** wide: board and monitor | Board in the foreground (left third), monitor with Scene 4 behind it, both in focus if possible. | 8–10 s | Same generation run as B2, or a second one. Lock exposure on the **monitor** (it's the brightest thing in frame); the board LEDs will still read. Avoid the laptop screen in frame. |
| **B4** BTN0 self-test | Close-up of BTN0 and LED0/LED1. | ~6 s, real time | Press BTN0 once with one finger. LED0 (FULL pass) and LED1 (STRIP pass) light. Don't press while the host is running a job (it would answer BUSY). |
| **B5** (optional) face to camera | You, chest up, board visible on the table in front of you. Eye-level phone, window light in front of you, not behind. | 5 s | "I'm ⟦name⟧, and I built SplitSys solo." One take is fine. |

Transfer: AirDrop / Google Photos / USB cable to the laptop, keep originals (don't use messaging apps; they recompress).

## (d) Screen-capture checklist (Windows Snipping Tool)

**Setup, once:**
1. Board on KG3, latency timer at 1 ms (DEMO.md §6), then `powershell -ExecutionPolicy Bypass -File demo\start_demo.ps1 -Transport serial:COM3 -WindowPosition <monitor x,y>`. Wait for `BOARD LIVE`.
2. **Turn the attract loop off**: if the scenes cycle on their own, press `A` once. Otherwise the dashboard changes scene mid-clip.
3. Focus on: notifications off. Move the mouse pointer off the portable monitor.
4. Click once on the dashboard window (so it receives the keys), away from the Scene 4 prompt box.

**Recording each clip:** press **Win+Shift+R**, drag the rectangle over the portable monitor exactly (the whole 1920×1080 dashboard), click **Start**. Snipping Tool counts down 3 s. Do the key sequence below. Click **Stop** in the Snipping Tool bar. Save as MP4 (`S1.mp4` etc.). Real time; do not speed these up except as noted in (a) note 2.

| Clip | Key sequence | Length | Check before saving |
|---|---|---|---|
| **S4** live generation | Press `4`. Click the prompt box, type `Once upon a time` (already the default if pre-filled), set tokens to `8`, press `Esc` to leave the box. Start recording. Press `G`. Let it finish all 8 tokens (~26 s). Wait 2 s. Stop. | ~30 s | `BOARD LIVE` badge visible, no amber REPLAY banner; story text appears token by token. If the output text differs from the recorded run, that's fine: show what it shows. |
| **S1** FULL then STRIP (scripted demo) | Press `1`. Start recording. Wait 2 s on the idle grid. Press `Space`. The dashboard runs ffn_up M=1 FULL, M=1 STRIP, M=64 FULL, M=64 STRIP (1.5 s pause between runs) and then jumps to Scene 2. Stop 3 s after Scene 2 appears. | 15–25 s | The M=1 FULL run shows one lit row and ~6.95%; the STRIP run shows ~14.40%. The M=64 part and the jump to Scene 2 are spare footage for 1:14–1:32. |
| **S2** model vs measured | Press `2`. Start recording. Hold 20 s (no input). Stop. | 20 s | Utilization-vs-M chart with hollow model markers, benchmark bars, model-vs-measured table, auto-mode panel all visible. |
| **S3** architecture | Press `3`. Start recording. Hold 15 s. Stop. | 15 s | Block diagram and roofline visible. |

If you need a pan or zoom on a panel (for example the model-vs-measured table), do it in Clipchamp with crop/zoom, not by moving the window.

## (e) Clipchamp assembly recipe

Clipchamp menu names change between versions; the steps describe what to do, and the exact button may differ.

1. **New video**, aspect ratio **16:9**.
2. **Import**: B1–B5 phone clips, S1–S4 screen clips, the seven cards `results/plots/video_cards/01_title.png` … `07_end.png`, and your voiceover audio (step 6).
3. **Video track (main)**, in this order, using the timecodes in (a): B1 → C01 (or B5) → B3 → S4 with a B2 cut-in → C05 → C02 → S1 → C03 → S2 → C04 → S2 (bars, auto panel) → S3 → B4 → C06 → C07. Cards are still images: drag each card's edge to the slot length in (a). Use straight cuts; at most a 0.3 s crossfade into and out of cards.
4. **Mute all clip audio** (select clip → audio → volume 0), or keep B1's switch click at low volume.
5. **Text track (labels)**: add the on-screen labels from (a) as Text, top-left, white on a dark semi-transparent box, ≥ 48 px at 1080p. Any sped-up span gets `2× speed` for its whole length. Any REPLAY footage gets `REPLAY · recorded`.
6. **Voiceover**: record with Windows **Sound Recorder** (quiet room, phone or headset mic about 15 cm away), reading section (a) one row at a time. Leave a 1 s gap between rows, so you can slide each row onto its slot. Import, put it on the audio track, and split it so each line starts at its timecode.
7. **Captions, burned in**: turn on Clipchamp's auto-captions for the voiceover, then correct every number against `docs/VIDEO_captions.srt` (auto-captions mishear "6.95", "4×16", "stories260K"). Style: white, bold, ≥ 48 px at 1080p, black outline or a dark box, bottom centre, at most 2 lines. If Clipchamp can't burn in captions on your version, add the SRT lines as Text instead. Also keep the SRT for the YouTube upload (step 9).
8. **Fill the end-card placeholders** before export: either edit the two strings in `results/plots/video_cards/make_cards.py` (07_end block), then run `python results/plots/video_cards/make_cards.py` and re-render with the command in (g), or cover the orange boxes with Clipchamp Text. No `⟦HUMAN` text may remain in the export.
9. **Export 1080p.** Watch it once end to end, muted, reading only the captions. Upload to **YouTube**, visibility **Unlisted** is fine. Under Subtitles, upload `docs/VIDEO_captions.srt` (for the 60 s cut the burned-in captions are enough). Paste the link into Devpost. Keep the MP4 on the laptop desktop as the booth fallback (DEMO.md §6).

## (f) Captions

`docs/VIDEO_captions.srt` holds the 2:00 voiceover as 26 cues, timed to (a). Each cue has at most 2 lines and ≤ 48 characters per line. If your recorded voiceover drifts, shift the cues in Clipchamp or YouTube's subtitle editor rather than changing the words.

## (g) Cards

`results/plots/video_cards/`: `make_cards.py` writes self-contained HTML (dashboard palette from DEMO.md §3, system fonts, no external URLs). Each PNG is 1920×1080, rendered with `scripts/screenshot`:

```
python results/plots/video_cards/make_cards.py
for b in 01_title 02_idle_rows 03_stat 04_model_exact 05_llm 06_limits 07_end; do
  scripts/screenshot "file:///C:/Users/danie/FPGA_Projects/Calhack2026/results/plots/video_cards/$b.html" \
    "/c/Users/danie/FPGA_Projects/Calhack2026/results/plots/video_cards/$b.png" 1920,1080
done
```

| Card | Content |
|---|---|
| 01_title | SplitSys: a splittable systolic array for LLM decode · Cal Hacks 13.0, built solo |
| 02_idle_rows | One token at a time → 7 of 8 rows idle; 8×8 grid with row 0 lit; ceiling 12.5%, measured 6.95% |
| 03_stat | 2.02–2.07× utilization at M=1, measured on the board; per-matmul FULL → STRIP |
| 04_model_exact | 773 hardware GEMMs · 12 counters each · 0 mismatches (0 of 9,276 counter values off the model; 0 output mismatches) |
| 05_llm | stories260K: every linear layer on the FPGA; attention/softmax/norms on the laptop; ~2 s/token wall time, 295 µs on-chip |
| 06_limits | Honest limits: no CPU/GPU speed claim, linear layers only, tiny model, FULL wins 31/97, cost of the split |
| 07_end | SplitSys · ⟦HUMAN: name⟧ · ⟦HUMAN: repo URL⟧ (placeholders, fill before export) |

## Card and voiceover sources

<!-- Re-verified by demo-storyteller on 2026-10-25 ~05:10 PDT against git-tracked files (git ls-files). -->
- **773 GEMMs, 0 mismatches, 0 counter values off the model:** the 773 git-tracked records in `results/hw/*.json` with `source: "hw"` and a `gemm` block. `check.mismatches` sums to 0; `golden_match` is true on 773/773. Every `counters` field equals the `model` field. That is 12 counters per run (8 scalars + 4 `mac_useful_q` entries; SPEC §3 `READ_COUNTERS` returns all 12) = 9,276 values, 0 different. Builds: KG1 0x52A66216 272, KG2 0x38A3950A 301, KG3 0x21DD6850 200 (per-build split from DEMO.md §7 source note). 393,129,309 MACs and 0 stall cycles: DEMO.md §7 source note [HW]; not recounted for this file.
- **2.02–2.07× at M=1:** `results/hw/2026-10-24T20-40-{20,33,41,59}Z-*-m1-{full,strip}.json` (KG1), `derived.util`: qkv 0.069515 → 0.143928 (2.0705), attn_out 0.069414 → 0.143498 (2.0673), ffn_up 0.069527 → 0.143982 (2.0709), ffn_down 0.104150 → 0.210873 (2.0247). Live KG3 runs at 2026-10-25T06-14-02Z/05Z (untracked) show the same 6.95% / 14.40%.
- **7 of 8 rows idle, 12.5% ceiling:** SPEC §4 mapping ceiling M/8 for FULL at M=1 (DEMO.md §8.2).
- **STRIP ahead at M=64 on the benchmarks:** same KG1 files at M=64: cycles FULL/STRIP qkv 22,082/21,314, attn_out 7,362/7,106, ffn_up 29,442/28,418 (util 55.65%/57.65%), ffn_down 19,650/19,394.
- **FULL faster on 31 of 97 random shapes, up to 1.32×:** KG1 records run in both modes: 125 shape pairs, STRIP took more cycles on 31, max ratio 1.3154 (3,566 vs 2,711 at M=22 K=250 N=17). The 31 are all among the 97 H-5 random pairs (DEMO.md §7 source note, recomputed here).
- **~2 s/token, 295 µs on-chip, 37 GEMMs/token, what runs where:** `results/hw/lm/2026-10-25T04-59-16Z-lm.json` (KG3, `serial:COM3@1562500`): `summary.wall_s_per_step` 2.1609, `cycles_per_step` 29,540 (= 295.4 µs at 100 MHz), 444/444 GEMMs bit-exact over 12 steps (444/12 = 37); `split.fpga` / `split.laptop`. Wall time is laptop time, with the 1 ms FTDI latency timer; not accelerator speed.
- **stories260K dim 64, 5 layers, vocab 512, MIT:** same file, `model` block.
- **+2,243 LUTs (+43.6%), +16 DSPs:** `results/cost.json` `breakdown.delta.total` {lut 2243, dsp 16}; full_only lut 5146 → 2243/5146 = 43.6%. "Mostly a wider requantizer": DEMO.md §8.1 (array + sequencer 667 LUT; requant ~1,500 LUT / 16 DSP) [B].
- **25% more operand bandwidth:** SPEC, 20 vs 16 B/cycle (DEMO.md §5 Scene 3).
- **Self-test (BTN0, LED0/LED1, one FULL and one STRIP GEMM):** SPEC §3.9.
- **Model written before the board session:** DEMO.md §8.2 (model crate committed at 72da652, Saturday 10:10 PDT; first hardware GEMM 13:26).
