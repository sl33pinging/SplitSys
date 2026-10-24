# Kickoff prompt — paste this as your first message in Claude Code

Start Claude Code in the repo root in auto mode (`claude --permission-mode auto`), then paste:

```
You are the lead for SplitSys, our Cal Hacks 13.0 hardware project. Read CLAUDE.md, docs/SPEC.md,
docs/PLAN.md, and docs/DEMO.md in full before acting.

We are working SIMULATION-FIRST. Build and verify everything with testbenches, system
simulation, and host<->RTL co-simulation, and build every bitstream timing-clean, BEFORE any
hardware. Do not ask me for the board until red-team-reviewer passes gate S4. Then we do one
guided hardware session using docs/HW_CHECKLIST.md, one physical step at a time.

Start now, and work autonomously without waiting for my confirmation between tasks:
1. Delegate to toolchain-engineer: explore WSL (and read-only the Windows drive with bounded
   searches) for existing tools, the Vivado wrapper, and an Urbana master XDC; install everything
   missing (user-level first); write docs/ENV.md and scripts/check_env.sh. If anything needs sudo,
   give me ONE combined command to run. Do not use code from my old projects.
2. In parallel: spec-steward (spec.toml + codegen), golden-modeler (golden + vectors),
   perf-modeler (cycle model). Then the RTL/verification lanes per docs/PLAN.md.
3. Create docs/STATUS.md and keep it current: gate, item, state, evidence command.
4. Run red-team-reviewer at every S gate. Apply the cut rules if a gate slips.
5. Only interrupt me for: the single sudo command, a missing XDC you cannot find on disk, or a
   decision that is genuinely mine. Otherwise keep going, and give me one line per completed
   handoff.

Rules that override everything: no fabricated numbers, no weakened tests, the spec wins,
protect fpga/known_good/, Devpost with table number before Sun 09:00.
```

## Follow-up prompts

- **Progress check:** `Summarize docs/STATUS.md: which S gate are we on, what passed (with evidence), what's blocked, and the ETA to S4.`
- **Gate check:** `Run red-team-reviewer on gate <S1|S2|S3|S4|G4|G5|G6>. Re-run every evidence command. Report pass/fail per item.`
- **Start the hardware session (after S4 passes):** `Begin Phase H. Walk me through docs/HW_CHECKLIST.md one step at a time: one physical action per message, the command you'll run, and what I should see. On any mismatch, stop and reproduce it in simulation before touching the board again.`
- **Bug triage:** `Hardware result mismatches golden for <case>. Have verification-engineer reproduce it in sim with the exact descriptor and seed from the hardware log, then rtl-designer fixes, then full regression, then rebuild.`
- **Demo polish:** `Have host-dashboard run the cold-start drill and the scripted demo on the monitor, list every visual issue, and fix them in priority order.`
- **Audience prep:** `Have demo-storyteller produce the 2-minute pitch for MatX and for Annapurna, then quiz me with the Q&A bank.`
