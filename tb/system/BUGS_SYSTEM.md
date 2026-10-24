# System / co-sim issues found by verification (owner: verification-engineer, instance 2)

Same format as `tb/BUGS.md`. Each entry: failing command, minimal reproduction, expected vs got,
classification, suspected location. Status: OPEN | FIXED-pending-reverify | CLOSED (re-verified, hash).

Re-run any system case alone: `scripts/wsl 'make -C tb/system SUITES=<suite> ONLY=<job name> WORKERS=1 [WAVES=1]'`
(waves land in `~/.cache/splitsys/system/seed<SEED>/waves/<board>.fst`).

_No open RTL issues._ Closed test issues are listed below.

## TB-001 — `selftest_busy`: "POLL during a 64x256x256 job is not BUSY" (test bug; FIXED in tb, CLOSED)
- Command: `scripts/wsl 'make -C tb/system SIGNOFF=1 SEED=1 WORKERS=18'` at HEAD 34cbc27 (rtl adca4a3), run 20261024-103659-11148: 616/617, only this case failed.
- Classification: **test bug (wall-clock race), not RTL.** The harness advanced simulated time whenever LED2 (busy)
  was lit, independent of host traffic. The job lasts 78,594 cycles (about 0.16 s of simulation at ~500k cycles/s),
  and between RUN_GEMM and POLL the test makes several Python round trips (including tb/oracle subprocess calls)
  while 18 boards share the CPU, so the job could legitimately finish first. The same case passed in 4 earlier full
  runs. POLL returning DONE after the job ends is correct behaviour.
- Fix (tb/cosim/harness.cpp + system_test.py): new control commands `freerun 0|1` and `linger <n>`. The busy-window
  checks now run with `freerun 0` (time advances only with UART traffic plus a 1,024-clock linger), so the window is
  bounded by the bytes exchanged, independent of wall clock. The test also asserts the measured window
  (`elapsed < job cycles`) and reports it on failure. The BUSY/POLL/live-counter checks themselves are unchanged.
- Re-verified: `make -C tb/system SUITES=selftest` 3/3, then the full SIGNOFF run (see SIGNOFF_SYSTEM.md).
