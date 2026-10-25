# What wins at Cal Hacks: hardware, solo and overall winners, applied to SplitSys

Research run: Sat Oct 24 2026, 22:55 to about 23:40 PDT. Sources were Devpost pages fetched with curl. WebFetch got HTTP 403 from Devpost. Raw notes are in `scratchpad\winners\proj\`, `proj12all\` and `raw\`, and video contact sheets are in `sb\`.

## 0. The most important finding: Cal Hacks 13.0 is happening now, and its rules differ from 11.0 and 12.0

Source: https://cal-hacks-13-0.devpost.com/ (fetched 2026-10-24).

- **Deadline: Oct 25, 2026, 9:00 AM PDT (hard).** You must also be at the venue by 8 AM.
- **Live judging:** "You will have three minutes to pitch and two minutes for judges to ask questions. You must have a pre-recorded or live demonstration." In 12.0 it was 2 minutes plus 2 minutes.
- **Categories that matter for SplitSys:** "Cal Hacks Best Hardware Hack" (Bambu Lab 3D printer), "Cal Hacks Best Solo Hack" (AirPods Pro 3), and **"Cal Hacks Most Technically Complex Hack"** (NuPhy Air60 HE). The last one is new in 13.0 and fits SplitSys well, so enter it too. There are also 1st/2nd/3rd Overall and Most Creative. The co-presenters are Anthropic, **Annapurna Labs**, Spacetime and Hyphenate. I found no Annapurna-specific prize in the list.
- **13.0 judging criteria (verbatim headings):** *Application* ("who they help, what problem they solve, and why people would actually use them"), *Intentionality* ("every design choice should feel purposeful"), *Functionality* ("a live demo that runs smoothly free of major bugs. Submissions should also include a clear, meaningful video demo on devpost for sponsors evaluating asynchronously"), *Creativity* ("inventive problem framing … Clearly show what makes your project stand out"), and *Technical Complexity* ("command of the frameworks and infrastructure … sound architectural choices and engineering depth").
- **Rule conflicts to check with organizers now (#ask-directors or the concierge desk):**
  1. The Devpost sidebar says **"Team required: 2 to 4 members"**, but there is a "Best Solo Hack" prize. Confirm that a one-person submission is eligible, and ask how to submit it.
  2. The rules say **"All work must be done during the hackathon."** If any SplitSys RTL, models or tooling existed before the event, ask the organizers how they want that disclosed. Then disclose it plainly on Devpost, for example with a "Built at Cal Hacks vs. before" line.
- **No "Best Solo Hack" category existed in 11.0 or 12.0** (prize lists: https://cal-hacks-11-0.devpost.com/ and https://cal-hacks-12-0.devpost.com/). So there is no past Best Solo Hack winner to copy. The "solo" evidence below comes from one-person teams that won other categories.
- **What "Best Hardware Hack" means:** no definition is published in 11.0, 12.0 or 13.0. It is just a prize line ("1 winner, 3D Printers" in 12.0, "Raspberry Pi 5s" in 11.0). The general Cal Hacks criteria apply: Application, Functionality/Quality, Creativity and Technical Complexity in 11.0 and 12.0, plus Intentionality in 13.0.

## 1. Winners examined (all claims cite the project page)

Team size counts the members listed on the Devpost page. Video length comes from the YouTube watch page (`lengthSeconds`) or Vimeo oEmbed. Shot types come from YouTube storyboard thumbnails, one frame every 1 to 2 s. I could not get audio or captions (YouTube returned empty caption files), so voiceover content is **not verified**.

### Best Hardware Hack

| | HCP, "Hardware Context Protocol (MCP for Hardware)" | PulseWalk |
|---|---|---|
| URL | https://devpost.com/software/hardware-context-protocol | https://devpost.com/software/pulsewalk |
| Event / prizes | 12.0: Best Hardware Hack + Crater Play-Do Prize | 11.0: Best Hardware Hack |
| Team | 4 | 3 |
| What it is | An SDK/protocol that lets an LLM discover and control hardware nodes over a custom TCP pub/sub layer. The demo is a voice/prompt-driven SO-ARM101 robot-arm "chef" that uses AprilTag camera localization. | A cardboard prototype shoe with IR distance sensors (front and bottom), 5 servos and vibration motors driven by an ESP32 for visually impaired walkers, plus a phone app. |
| Tagline / hook | "Giving AI a physical embodiment for the world. Control multiple hardware platforms cooperatively via LLM prompts." The hook rides a concept judges already know: "MCP revolutionized … Missing from this was meaningful capability to interact with the real world." | A personal story: "my grandmother … tripped on an uneven curb". |
| Story arc | Problem (LLMs can't touch the world) → protocol → arm demo → engineering details (their own single-chain IK solver with elbow-up/down solutions, a simulator to validate the IK, custom TCP stack) → roadmap. | Person → problem → three detection zones (0–6 in, 6–12 in, 12+ in) → build (ESP32, 2×5000 mAh) → candid failures ("we had trouble integrating Deepgram … Skylo"). |
| Video | "HCP Calhacks Demo", **4:08**. Raw handheld phone footage at their hackathon table (table number visible). It cuts between a laptop terminal where prompts are typed and the arm picking objects inside a cardboard backdrop, several prompt → action cycles, a teammate smiling at the end. No title cards, slides or face-to-camera pitch visible. | **No video embedded.** |
| Live physical demo | Yes, it is the entire video. | Yes, a wearable prototype. |
| Wow | You type a sentence and a real arm moves. | You walk toward a curb and the shoe buzzes harder. |
| Claims | Engineering-specific but **no numbers** (no latency or success rate). | Concrete thresholds and components. Honest about what didn't work. |

**Takeaway:** neither hardware winner was polished. One won with a 4-minute unedited phone video and the other with no video and a cardboard shoe. What they share is a **physical cause → effect a judge sees in seconds** and **real engineering detail in the write-up**.

### Overall winners (context)

| Project | Prize | Team | One line | Hardware | Video (length, style) | Claims |
|---|---|---|---|---|---|---|
| FaceTimeOS https://devpost.com/software/facetime-macos-ai-agent | 12.0 1st Overall | 3 | FaceTime/iMessage your Mac and an AI agent controls it | none (phone + Mac) | 2:02. Opens with **face-to-camera skit** of the three teammates with burned-in subtitles ("What if you could control your computer from anywhere in the world?"), then phone-in-hand demo, screen capture, "Task Done!", and closes in a car ("on the go") | Relatable hook ("forgotten to train your Hugging Face model?"), vibes, no numbers |
| Ted.AI https://devpost.com/software/ted-ai-x8537t | 12.0 2nd Overall + LiveKit Most Creative | 4 | Teddy bear with ESP32, pulse/touch/IMU sensors and an LLM companion for kids' mental wellness | ESP32, HW-502 pulse, MPU6050 | 2:09. Black **"EVERY DAY" title card**, stock city B-roll, teddy close-up, face-to-camera with the bear, red **"DEMO" badge** on live interaction, line-art explainer, dashboard screen capture | Emotional; "millions of children"; few hard numbers |
| Project No Cap https://devpost.com/software/project-no-cap | 12.0 3rd Overall | 4 | Real-time misinformation checking | none | 0:50 | Vibes |
| Duet https://devpost.com/software/duet-0tbkxe | 11.0 1st Overall | 4 | Live EEG (Emotiv) → emotion → generated music | Emotiv EEG | 3:41. **Title slide with webcam picture-in-picture narration**, live footage of a person wearing the EEG, waveform visual, a long (~85 s) "How Duet Works" **architecture slide** with picture-in-picture, end card "Come find your creative self with Duet". The on-screen footnote says the background music is generated from brainwaves, so the product scores its own video. | "approximately 1 in 10" children statistic; personal teaching experience |
| GhostWriter https://devpost.com/software/ghostwriter-491kuy | 11.0 2nd Overall | 2 | Office-hours speech → auto-updated course notes | none | 2:37 | Personal TA story |
| SnackSnap https://devpost.com/software/snacksnap-lw4b9q | 11.0 3rd Overall | 2 | Recycle in real life to feed a digital pet | none | 1:12 | Light |

### Other hardware-flavoured winners (12.0 unless noted)

| Project | Prize | Team | Hardware | Video | Notable framing |
|---|---|---|---|---|---|
| Bob https://devpost.com/software/bob-vj43mq | Cal Hacks Most Creative Hack | 3 | Snap Spectacles + Gemini Live | 1:59. **Demo-first cold open** (wearer wiring an LED while talking), **stat cards** ("642,000 technicians needed", US map), talking head, **POV through the glasses** with overlay text, **architecture diagram with each block highlighted in turn**, title card "We asked 20 people what they would do with our glasses…", vox-pop interviews | Numbers up front: "shortage of 70,000 electricians and 642,000 mechanics" |
| Iris https://devpost.com/software/iris-ojrmnv | Crater Play-Do Prize | 4 | Webcam eye tracking | 1:35, "Iris - CalHacks '25 (Winner)". Cinematic: extreme close-up of eyes opening, on-screen **research citations with statistics** overlaid on B-roll, laptop demo with face picture-in-picture | "sub-100ms latency", Kalman/KDE filtering, a family-member story |
| MIRAI https://devpost.com/software/boxing-robot | BitRobot Best Robotics Hack, 1st | 4 | Unitree G1 humanoid | 1:33. **Vertical phone footage at the venue**, caption label per mode ("Following Mode", "Motion Imitation Mode", "Voice Recognition Mode"); the person moves and the robot mirrors | Technical stack (MediaPipe, Pinocchio IK, SDK2) |
| MARC https://devpost.com/software/marc-marker-actuated-robotic-controller | BitRobot Best Robotics Hack, 2nd | 4 | SO-ARM robot arm drawing | 1:59. Team pitching at the table to a low camera with a laptop slide behind; the arm only appears in the last ~8 s | "sub-2mm positioning accuracy", "$10,000" arm cost, "285 million people with visual impairments" |
| Wingman https://devpost.com/software/wingman-9xok8b | Snap Spectacles + BitRobot Robotics 3rd | 4 | Spectacles → ground robot | 1:22. A **single continuous vertical phone shot**: the user gestures and the robot drives | Disaster-response story |
| BallAR https://devpost.com/software/ballar | Snap Spectacles | 3 | Spectacles + ESP32 electro-stimulation | 0:51 | Humorous ("less Greek Life and more Geek Life") |
| MemARy https://devpost.com/software/memary | Snap Spectacles | 4 | Spectacles | 1:17 | Statistic-first hook ("Over 7 million Americans over 65 …") |
| Physical Digital Darts https://devpost.com/software/digital-physical-darts-wii-darts | Cal Hacks Best Beginner Hack | 4 | Accelerometer dart, custom circuit | 2:05 | Gallery includes circuit schematic, board photos, CAD |
| PulseWalk (11.0) | above | | | | |

### One-person winners (no Best Solo Hack existed in 11.0 or 12.0)

| Project | Prize | Write-up | Video | Lesson |
|---|---|---|---|---|
| MafiAI https://devpost.com/software/mafiai | 11.0 Cal Hacks "Hack on AI" | **29 words** ("Mafia with LLMs!") | 0:51 pure screen capture of the game | The live table demo carried it. The write-up barely mattered. |
| Refrigerator Ramsay https://devpost.com/software/refrigerator-ramsay | 11.0 Google Most Useful App | 611 words, standard template | 2:04 "Working Demo" | Clear everyday problem |
| Duck Duck Goose https://devpost.com/software/duck-duck-goose-jelxr9 | 12.0 AppLovin Query Planner Challenge ($10,000) | **83 words**, number-first: "~1300x speedup compared to the baseline implementation (~45s to ~30ms) on an M3 Pro with 18 GB RAM. Preprocessing takes 300.5 seconds" | 0:22 | **Closest analogue to SplitSys:** a solo systems/performance project that won with one measured number, its baseline, its hardware and its cost disclosed |
| Brydge https://devpost.com/software/brydge | 12.0 YC "Build an Iconic YC Company" 1st | 906 words, first person ("During my internship at NVIDIA…") | 2:29 polished screen recording with zoom-ins; **labels sped-up parts "2X SPEED"**; "Thanks for watching" | Honest speed labelling |
| Cryptoagent-CFO https://devpost.com/software/cryptoagent-cfo, PitchLab https://devpost.com/software/pitchlab-clxhuj, Poke SDR https://devpost.com/software/poke-sdr, Shard https://devpost.com/software/shard-2kvsch | Postman / Rox / Lava / Sui sponsor prizes (12.0) | 218–1455 words | 3:00 / 2:05 / 3:39 (Vimeo) / 3:16 | Sponsor-track wins; numbers used as hooks ("cuts costs 80%", "$10/day") |

Two more one-person 12.0 winners came from sponsor tracks: XRPower https://devpost.com/software/xrpower (Ripple XRPL) and MonitorMachine https://devpost.com/software/monitormachine (Regeneron Honorable Mention, 9:01 video).

### Aggregate numbers
These cover all 96 Cal Hacks 12.0 gallery winners (pages 1–4, every project page fetched) plus the 12 Cal Hacks 11.0 page-1 winners I opened, for 108 winning projects in total.
- **Team size, 12.0 winners:** 8 solo, 16 of two, 25 of three, 45 of four, 2 of five. **Solo is about 8% of winners**, and none of the solo winners was a hardware project.
- **Video length, all 108 winners:** 99 had an embedded video. **Median 125 s** (IQR 95–191 s), minimum 21 s, maximum 782 s. Distribution: ≤60 s: 8, 61–120 s: 34, 121–180 s: 29, >180 s: 28.
- **Video length, Cal Hacks-track (organizer) prize winners only:** 15 with video, **median 129 s**.
- **Write-up length, all 108:** median about 680 words, range 29–1959.

## 2. Patterns that separate hardware and solo winners

Caveat: I could only examine **winners**. Devpost served gallery pages 1–4 (all winners) for 12.0 and page 1 for 11.0, then returned a bot-challenge page. I did not systematically sample non-winners, so read "vs non-winners" as "what all the winners share", not as a controlled comparison.

1. **A physical cause → effect visible within about 10 seconds.** Every hardware-category winner shows an input and an immediate physical response. HCP: type a prompt and the arm grabs food (https://devpost.com/software/hardware-context-protocol). MIRAI: a person raises an arm and the humanoid mirrors it (https://devpost.com/software/boxing-robot). Wingman: point and the robot drives (https://devpost.com/software/wingman-9xok8b). PulseWalk: approach a drop-off and the vibration intensifies (https://devpost.com/software/pulsewalk). *For an FPGA, which has no moving parts, the LEDs/7-seg changing and text appearing on the monitor are the "motion". Make that cause → effect explicit.*
2. **Reality beats polish for hardware.** The 12.0 Best Hardware Hack video is a 4-minute unedited phone clip at the table, and the 11.0 winner had no video and a cardboard shoe. Judges reward "it really works here". Polished production shows up in overall/creative winners (FaceTimeOS, Ted.AI, Iris, Bob), not in the hardware winners.
3. **The hook is a person-sized problem or an analogy to something judges already know.** HCP: "MCP for Hardware". FaceTimeOS: "FaceTime your Mac". PulseWalk/Elda: grandmother. Iris: paralysis. Bob: a 642,000-technician shortage. Technical winners still lead with a one-line outcome a layperson gets.
4. **Numbers are credibility, and the best ones carry their conditions.** Duck Duck Goose states the baseline, the result, the machine and the preprocessing cost (https://devpost.com/software/duck-duck-goose-jelxr9). Iris states "sub-100ms" and MARC "sub-2mm". Bob and Iris put **stat cards and citations on screen** in the video. Vibes-only projects win in AI/consumer categories, but technical winners anchor on numbers.
5. **Engineering depth shows up as named components and problems solved, not as buzzwords.** HCP explains its own IK solver, null-space rejection and custom TCP pub/sub. PulseWalk lists exact zones and components. Darts shows the schematic, the board front and back, and the CAD.
6. **Candour doesn't hurt.** PulseWalk openly lists integrations that failed and still won Best Hardware. Brydge labels "2X SPEED". This supports keeping SplitSys's limits visible.
7. **Solo winners keep scope tight and the demo instant.** MafiAI had 29 words and a 51-second screen capture. Duck Duck Goose had 83 words, one number and a 22-second video. Solo wins came from one crisp thing done fully, not from breadth.
8. **The architecture diagram appears in many winning videos, but animated and narrated.** Bob highlights each block in turn and Duet narrates over its diagram for about 85 s. The diagram works when it is walked through, not flashed.

## 3. Framing recommendations for SplitSys

### One-sentence hook (spoken, first line of the pitch and the video)
> "When a chatbot writes one word at a time, most of the multipliers in an AI chip sit idle, so I built a chip on this FPGA that rewires itself to put twice as many of them to work, and it's running a real language model right now."

A shorter alternative for the table: "This FPGA is a tiny AI chip I built solo. It reshapes itself to double how much of it is working while an LLM writes word by word."

### Devpost tagline (≤ 200 chars; both checked by character count)
- **A (178 chars):** "I built a tiny AI chip on an FPGA that rewires itself to keep twice as many multipliers busy while a language model writes one word at a time - and it is running a real LLM live."
- **B (175 chars):** "A one-person AI chip on an FPGA that reshapes itself mid-run to double how much of it works while an LLM writes word by word. Running a real model live, every number measured."

Before using them, make sure "twice as many multipliers busy" matches what was measured: a 2.02–2.07× utilization ratio at M=1. If the denominator is not "busy multipliers", say "doubles its utilization" instead.

### Devpost first two lines (above the template sections)
> SplitSys is an 8×8 int8 systolic-array accelerator I designed and built solo on a Spartan-7 FPGA. In LLM decode (one token at a time, M=1), a square array wastes most of its rows, so SplitSys rewires into a 4×16 "STRIP" shape and measures **2.02–2.07× higher utilization**. A cycle model I wrote predicted **all 12 hardware counters exactly on 773 GEMMs** run on the board. It runs the public stories260K model with **every linear layer on the FPGA**, live, at about 2 s/token.

(The "wastes most of its rows" and "idle rows" wording here and in the video script is my reading of M=1 on a square array. Check it against the actual SplitSys dataflow and the FULL-mode utilization number before publishing.)

### First 30 seconds (video and live pitch): show these three things
1. **Cold start:** power switch on, the board boots from flash, LEDs and 7-seg come alive with no laptop programming it. This proves it's standalone hardware (the "it's real" beat HCP and MIRAI get from motion).
2. **The LLM generating live:** dashboard text appearing token by token while board LEDs and counters tick in sync. Say "every matrix multiply is on this chip."
3. **The one idea, as a before/after:** FULL 8×8 → STRIP 4×16 on the dashboard (scene with the array grid), with the measured utilization ratio jumping to about 2×. One number, big, on screen.

### De-emphasise
- The int8 requantization details, BRAM layouts, UART packet format and Rust/cocotb tooling. Keep them for Q&A and the README.
- Phase/process story (gates, agents, verification flow), except one line: "verified against a bit-exact golden model."
- Any comparison with CPU/GPU throughput, and any "fast" language. Say "efficient use of the silicon", not "fast".
- The model's parameter count as a headline. "Tiny" is fine and honest, so don't hide it, but the story is the architecture, not the model.

### Honest limitations to keep visible (on Devpost and on one video card)
- "Linear layers run on the FPGA. Attention, softmax and normalisation run on the laptop."
- "About 2 s per token. This is a 64-MAC int8 array on a hobby FPGA; the result is a utilization gain inside the array, not a speed claim against a CPU or GPU."
- "stories260K is a tiny public model."
- "The 2.02–2.07× is measured at M=1 (decode). In prefill-like shapes (large M), FULL mode is the right shape, which is why it's switchable." Check this wording against your own data before using it.
- Model-match scope: "773 hardware GEMMs, 12 counters each, exact match", stated with its scope.

### Category-specific angle
- **Best Hardware Hack:** lead with the board (cold boot, LEDs, a real model running on silicon you configured). Physical presence first, numbers second.
- **Best Solo Hack:** say "solo" early and once ("designed, verified and brought up by one person"), then let the breadth speak through one image: RTL → cycle model → board → live LLM, with no hand-waving at any layer. Solo winners won on one crisp thing; here the crisp thing is the 2× mode switch.
- **Most Technically Complex Hack (13.0):** the "model predicted every counter exactly on 773 GEMMs" result is the strongest signal of "command of the infrastructure". Put it in the pitch, not only in the write-up.
- **13.0 "Application" criterion:** give one sentence on who benefits, e.g. "Edge and small-batch LLM inference is decode-bound; reshaping the array is a way to stop paying for idle multipliers." Avoid market-size claims you can't source.

### Live table demo (13.0: 3-minute pitch + 2-minute Q&A, pre-recorded or live demo allowed)
- Keep the dashboard attract loop running and the board visible at eye level, with LEDs facing the judges.
- If any physical input can trigger the FULL/STRIP switch or a cold restart, let the judge press it. HCP- and MIRAI-style "judge causes, hardware responds" is the strongest hardware moment. Only do this if the input already exists and is reliable.
- Keep the recorded video ready on the laptop as a fallback (12.0 explicitly required a locally stored recorded video).

## 4. Video guidance

### What the winners did
- **Length:** winners cluster around **2 minutes** (median 125 s across 99 winner videos, median 129 s for Cal Hacks-track prizes). The two Best Hardware data points are extremes: 4:08 raw (HCP) and none at all (PulseWalk). Aim for about 2:00 and keep a 60 s cut.
- **Structures seen:**
  - Raw demo only: HCP 4:08, MIRAI 1:33, Wingman 1:22, MafiAI 0:51.
  - Skit/talking-head hook, then demo, then close: FaceTimeOS 2:02.
  - Title card → B-roll → face-to-camera with product → demo badge → screen capture: Ted.AI 2:09.
  - Demo-first cold open → stat cards → talking head → POV demo → animated architecture → vox pops: Bob 1:59.
  - Slides + webcam picture-in-picture → live footage → long architecture slide → end card: Duet 3:41.
  - Cinematic close-ups + citation overlays + screen demo: Iris 1:35.
- **Shot types that recur:** board/device close-ups at the venue, laptop-screen capture, picture-in-picture of the presenter, on-screen labels for each demo mode (MIRAI), stat cards (Bob), animated architecture walkthrough (Bob, Duet), an honest speed label (Brydge "2X SPEED"), end card with name/URL (Duet, Brydge).
- **Not verified:** music and voiceover. I could not fetch audio or captions. Subtitles are visible in the FaceTimeOS frames, so at least one winner used **burned-in captions**. Captions are recommended anyway: judges watch async, often muted.

### SplitSys ~2:00 video: shot list + script outline (real material only)

| Time | Shot | On-screen text | Voiceover (draft) |
|---|---|---|---|
| 0:00–0:08 | **Board close-up**, hand flips power; LEDs and 7-seg come up (cold start from flash). Real time, no cuts. | "SplitSys — built solo" | "This FPGA is an AI accelerator I designed and built myself. It just booted from its own flash." |
| 0:08–0:22 | **Dashboard live LLM generation** (the generation scene), text appearing; cut-in of board LEDs ticking. | "Live: stories260K — every linear layer on the FPGA" | "It's running a real, public language model. Every matrix multiply in every linear layer is computed on this chip, at about two seconds per token. Slow, but real." |
| 0:22–0:40 | **Dashboard scene showing the 8×8 array at M=1**, idle rows visible. | "Decode = one token at a time → most of a square array idles" | "When a model writes one word at a time, a square matrix engine has almost nothing to fill most of its rows." |
| 0:40–1:00 | **Architecture diagram, FULL 8×8 ↔ STRIP 4×16**, then the dashboard mode switch with the utilization number. | Big: "2.02–2.07× utilization (measured, M=1)" | "So the array rewires itself into a long, thin strip: four by sixteen. Same multipliers, twice as many doing useful work. Measured on the board, not simulated." |
| 1:00–1:25 | **Hardware counters vs cycle model** scene; zoom on the "match" column. | "773 GEMMs on hardware · 12 counters each · model predicted all exactly" | "I also wrote a cycle-level model of the chip. Across 773 matrix multiplies run on the board, it predicted every one of twelve hardware counters exactly, so I know where every cycle goes." |
| 1:25–1:45 | **Architecture diagram walk-through**, highlighting one block at a time (Bob-style): host → UART → buffers → array → requant → counters. | Block labels | "Under the hood: int8 inputs, int32 accumulation, on-chip buffers, a sequencer that chooses the shape, and counters that make it all measurable." |
| 1:45–2:00 | **Honest-limits card**, then the board with the LLM still generating; end card. | "Linear layers on FPGA · attention/softmax/norms on laptop · ~2 s/token · no CPU/GPU speed claims" → "SplitSys · [name] · [GitHub]" | "What's on the chip is the linear layers; the rest runs on the laptop. The point isn't raw speed. It's making every multiplier count." |

Production notes:
- Record the board and dashboard in **real time**. If any segment is sped up, label it "2× speed" (Brydge precedent).
- Burn in captions.
- A face-to-camera intro is optional. If you do one, keep it to 5 s after the cold open. Solo builders benefit from being seen once ("built by one person").
- Avoid stock B-roll. Every frame should be your hardware or your dashboard.

### 60-second cut

| Time | Shot | Text / VO |
|---|---|---|
| 0:00–0:06 | Power-on cold boot, LEDs/7-seg | "An AI chip I built solo on this FPGA. It boots from flash." |
| 0:06–0:18 | Live LLM generation + LEDs | "It's running a real language model. Every linear layer on the chip, ~2 s/token." |
| 0:18–0:38 | FULL → STRIP diagram + dashboard switch, big "2.02–2.07×" | "One word at a time leaves a square array idle, so it rewires to 4×16 and doubles utilization, measured." |
| 0:38–0:50 | Counters vs model: "773 GEMMs · 12 counters · exact" | "A cycle model predicted every hardware counter exactly." |
| 0:50–1:00 | Limits line + end card | "Attention and norms on the laptop; no speed claims. SplitSys." |

## 5. Data gaps / what I could not verify
- Devpost blocks automated fetches (WebFetch HTTP 403; curl allowed for about 5 gallery pages, then a bot-challenge page). I got 12.0 gallery pages 1–4 (96 winners, all swept for team size) and 11.0 page 1 (24 winners listed, of which I opened the first 12, covering all the Cal Hacks-track prizes on that page including Best Hardware Hack and 1st–3rd Overall). 11.0 winners beyond those were not examined.
- I did not systematically examine any non-winners.
- No captions or audio. Voiceover and music are unverified, and shot descriptions come from storyboard thumbnails.
- https://cal-hacks-13-0.devpost.com/rules contains only the Code of Conduct. It has nothing on team size, solo eligibility or prior work. The main page has the "Team required: 2 to 4 members" and "All work must be done during the hackathon" lines.
- No judging rubric beyond the published criteria. No published definition of "Best Hardware Hack" or "Best Solo Hack".
- PulseWalk, Elda AI, Guardian Angel and Talk Tuah embed no YouTube/Vimeo video on their pages.
