# System / co-sim issues found by verification (owner: verification-engineer, instance 2)

Same format as `tb/BUGS.md`. Each entry: failing command, minimal reproduction, expected vs got,
classification, suspected location. Status: OPEN | FIXED-pending-reverify | CLOSED (re-verified, hash).

Re-run any system case alone: `scripts/wsl 'make -C tb/system SUITES=<suite> ONLY=<job name> WORKERS=1 [WAVES=1]'`
(waves land in `~/.cache/splitsys/system/seed<SEED>/waves/<board>.fst`).

_No open issues._
