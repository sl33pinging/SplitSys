#!/usr/bin/env bash
# tb/cosim/host_check.sh -- S3 co-sim acceptance in one command (owner: verification-engineer).
# Starts the cosim harness on an ephemeral port, runs the REAL splitsys-host against it:
#   ping, selftest, run ffn_up M=1 STRIP, run ffn_up M=1 FULL, run qkv M=8 both,
#   a random job file in each mode, bench smoke (ffn_up M in {1,64} x both modes)
# and stops the harness. Run records land in $RESULTS/sim/ (source "sim").
# usage: host_check.sh <harness Vtop> <splitsys-host> <results dir> [seed]
set -u
BIN=$1; HOST=$2; RESULTS=$3; SEED=${4:-1}
LOG=$(mktemp -d)/cosim.log
"$BIN" --port 0 --ctrl-port 0 >"$LOG" 2>&1 &
HPID=$!
trap 'kill $HPID 2>/dev/null; wait $HPID 2>/dev/null' EXIT
for _ in $(seq 100); do grep -q '^COSIM_PORT' "$LOG" && break; sleep 0.1; done
PORT=$(awk '/^COSIM_PORT/{print $2}' "$LOG")
[ -n "$PORT" ] || { echo "harness did not start"; cat "$LOG"; exit 1; }
echo "== cosim harness pid $HPID on tcp:127.0.0.1:$PORT"
H="$HOST --transport tcp:127.0.0.1:$PORT --results $RESULTS"
fail=0
step() {
  echo "== splitsys-host $*"
  local t0=$(date +%s)
  $H "$@"; local rc=$?
  echo "== rc=$rc ($(( $(date +%s) - t0 ))s)"
  [ $rc -eq 0 ] || fail=$((fail + 1))
}
step ping
step selftest
step run --name ffn_up --m 1 --mode strip --seed "$SEED"
step run --name ffn_up --m 1 --mode full --seed "$SEED"
step run --name qkv --m 8 --mode both --seed "$SEED"
step run --name attn_out --m 5 --mode both --int32 --seed "$SEED"
step bench smoke --seed "$SEED"
step ping
echo "== host_check: $fail failing step(s); harness log: $LOG"
grep -E "^cosim:|framing|glitch" "$LOG" | tail -3
exit $fail
