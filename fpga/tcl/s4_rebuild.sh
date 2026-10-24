#!/usr/bin/env bash
# =============================================================================
# s4_rebuild.sh - one-shot S4 freeze rebuild of every Phase H bitstream
# (owner: fpga-integrator). Run from Git Bash or WSL at any cwd:
#   bash fpga/tcl/s4_rebuild.sh            # build, check, report, commit
#   bash fpga/tcl/s4_rebuild.sh --no-commit
#
# 1. Refuses to start unless rtl/, fpga/bringup, fpga/xdc, fpga/tcl and
#    sw/crates/vec/out are clean, so every bitstream is from ONE commit.
# 2. Builds six tags with the canonical flow (PHYS_OPT=AggressiveFanoutOpt
#    POST_ROUTE_PHYS_OPT=1 for all, so there is one flow), three Vivado runs at a
#    time. Old fpga/build/<tag> directories are deleted first so a failed build
#    can never leave an older top.bit behind.
# 3. Checks every summary.json: pass, git_dirty false, git_hash == HEAD; and
#    that every "DIV_OVERRIDE bound to" line in every log is 0.
# 4. Copies each Vivado console log to fpga/reports/<tag>/build.log, runs
#    report_paths.tcl for top and top_nostrip, write_cost.py, manifest.py
#    (fills the Bitstream manifest in docs/HW_CHECKLIST.md).
# 5. Commits fpga/reports, results/cost.json and docs/HW_CHECKLIST.md only.
# Exit 0 only if everything above passed.
# =============================================================================
set -uo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root" || exit 1
commit=1
[ "${1:-}" = "--no-commit" ] && commit=0

OPTS="PHYS_OPT=AggressiveFanoutOpt POST_ROUTE_PHYS_OPT=1"
# tag | top | extra build options
JOBS=(
  "top|top|"
  "top_nostrip|top|ENABLE_STRIP=0"
  "heartbeat_uart_echo|heartbeat_uart_echo|"
  "heartbeat_uart_echo_uartswap|heartbeat_uart_echo|UART_SWAP=1"
  "packet_mem|packet_mem|"
  "packet_mem_uartswap|packet_mem|UART_SWAP=1"
)

fail() { echo "S4_REBUILD_FAIL: $*" >&2; exit 1; }

dirty="$(git status --porcelain -- rtl fpga/bringup fpga/xdc fpga/tcl sw/crates/vec/out)"
[ -z "$dirty" ] || fail "working tree not clean:"$'\n'"$dirty"
head="$(git rev-parse --short=8 HEAD)"
echo "S4: rebuilding ${#JOBS[@]} bitstreams at $head ($(date))"

run_job() {  # tag top extra
  local tag="$1" top="$2" extra="$3"
  rm -rf "fpga/build/$tag"
  # shellcheck disable=SC2086
  scripts/vivado -mode batch -source fpga/tcl/build.tcl -tclargs "$tag" "$top" $extra $OPTS \
    > "fpga/build/s4_$tag.txt" 2>&1
  echo "S4: $tag exit $?"
}

mkdir -p fpga/build
for wave in "0 1 2" "3 4 5"; do
  for i in $wave; do
    IFS='|' read -r tag top extra <<< "${JOBS[$i]}"
    run_job "$tag" "$top" "$extra" &
  done
  wait
done

errs=0
for j in "${JOBS[@]}"; do
  IFS='|' read -r tag top extra <<< "$j"
  s="fpga/reports/$tag/summary.json"
  log="fpga/build/s4_$tag.txt"
  if ! grep -q "^BUILD_PASS:" "$log"; then
    echo "S4: $tag FAILED: $(grep -E '^(BUILD_FAIL|BUILD_ERROR)|^ERROR' "$log" | head -3)"; errs=1; continue
  fi
  grep -q "\"git_hash\":\"$head\"" "$s" || { echo "S4: $tag summary not at $head"; errs=1; }
  grep -q '"git_dirty":"false"' "$s" || { echo "S4: $tag dirty"; errs=1; }
  grep -q '"pass":true' "$s" || { echo "S4: $tag pass != true"; errs=1; }
  if grep "DIV_OVERRIDE bound to" "$log" | grep -qv "bound to: 32'b0\{32\}"; then
    echo "S4: $tag has a nonzero DIV_OVERRIDE"; errs=1
  fi
  cp "$log" "fpga/reports/$tag/build.log"
  echo "S4: $tag $(grep -o '"wns_ns":[^,]*,"whs_ns":[^,]*' "$s")"
done
[ "$errs" = 0 ] || fail "see above; nothing committed"

for tag in top top_nostrip; do
  scripts/vivado -mode batch -source fpga/tcl/report_paths.tcl -tclargs "$tag" > "fpga/build/s4_paths_$tag.txt" 2>&1 \
    || fail "report_paths $tag"
done
PY="$(command -v python || command -v python3)"
"$PY" fpga/tcl/write_cost.py < /dev/null > /dev/null || fail "write_cost.py"
"$PY" fpga/tcl/manifest.py < /dev/null || fail "manifest.py"

if [ "$commit" = 1 ]; then
  paths=(fpga/reports results/cost.json docs/HW_CHECKLIST.md)
  git add "${paths[@]}" || fail "git add"
  git commit -q -m "[fpga-integrator] S4 freeze rebuild: 6 bitstreams at $head, cost.json, manifest — bash fpga/tcl/s4_rebuild.sh passed" \
    -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>" -- "${paths[@]}" || fail "git commit"
  echo "S4: committed $(git log --oneline -1)"
fi
echo "S4_REBUILD_OK at $head"
