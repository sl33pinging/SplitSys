#!/usr/bin/env bash
# SplitSys toolchain check. Prints tool / required / found / status and exits
# non-zero if any REQUIRED item FAILs. OPTIONAL items never fail the run.
#
#   source scripts/env.sh && scripts/check_env.sh          # full (launches Vivado, ~30 s)
#   scripts/check_env.sh --quick                            # skip launching Vivado
#   scripts/check_env.sh --smoke                            # also run cocotb+Verilator smoke test
#   From Git Bash: scripts/check_env.sh re-executes itself in WSL.
# Owner: toolchain-engineer. See docs/ENV.md.

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if ! grep -qi microsoft /proc/version 2>/dev/null; then
    exec "$HERE/wsl" "scripts/check_env.sh $*"
fi
# shellcheck source=/dev/null
source "$HERE/env.sh"

QUICK=0; SMOKE=0
for a in "$@"; do
    case "$a" in
        --quick) QUICK=1 ;;
        --smoke) SMOKE=1 ;;
        *) echo "unknown option $a" >&2; exit 2 ;;
    esac
done

FAILS=0
row() { printf '%-22s %-14s %-38s %s\n' "$1" "$2" "${3:0:38}" "$4"; }
# ver_ge FOUND MIN -> true if FOUND >= MIN
ver_ge() { [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -1)" = "$2" ]; }

# check NAME REQ(min version or "any") KIND(required|optional) CMD...
# CMD prints the found version (first x.y[.z] token is used for comparison).
check() {
    local name="$1" req="$2" kind="$3"; shift 3
    local out ver status
    out="$("$@" 2>/dev/null | head -1 | tr -d '\r')"
    ver="$(printf '%s' "$out" | grep -oE '[0-9]+(\.[0-9]+)+' | head -1)"
    if [ -z "$out" ]; then
        status=FAIL; out="(missing)"
    elif [ "$req" = any ] || ver_ge "${ver:-0}" "$req"; then
        status=OK
    else
        status=FAIL
    fi
    if [ "$status" = FAIL ]; then
        if [ "$kind" = optional ]; then status="WARN(optional)"; else FAILS=$((FAILS + 1)); fi
    fi
    row "$name" "$req" "$out" "$status"
}

echo "SplitSys check_env  ($(date '+%Y-%m-%d %H:%M'))  root=$SPLITSYS_ROOT"
row TOOL REQUIRED FOUND STATUS
row ---- -------- ----- ------
# Rust
check rustc        1.80   required rustc --version
check cargo        1.80   required cargo --version
check clippy       any    required cargo clippy --version
check rustfmt      any    required rustfmt --version
# C/C++ and build basics
check make         4.0    required make --version
check gcc          11.0   required gcc --version
check g++          11.0   required g++ --version
check git          2.30   required git --version
check flex         any    required flex --version
check bison        any    required bison --version
check perl         5.0    required perl -e 'print "perl $]\n"'
# Python / cocotb
check python3      3.10   required python3 --version
check venv-python  3.10   required "$SPLITSYS_VENV/bin/python" --version
check cocotb       2.1.0  required "$SPLITSYS_VENV/bin/python" -c 'import cocotb; print("cocotb", cocotb.__version__)'
check pytest       8.0    required "$SPLITSYS_VENV/bin/python" -c 'import pytest; print("pytest", pytest.__version__)'
check cocotb-config any   required cocotb-config --version
check matplotlib   3.9.2  required "$SPLITSYS_VENV/bin/python" -c 'import matplotlib; matplotlib.use("Agg"); import matplotlib.pyplot; print("matplotlib", matplotlib.__version__)'
check numpy        2.0    required "$SPLITSYS_VENV/bin/python" -c 'import numpy; print("numpy", numpy.__version__)'
# Simulators
check verilator    5.036  required verilator --version
check iverilog     any    optional sh -c 'iverilog -V 2>&1 | head -1'
check gtkwave      any    optional sh -c 'command -v gtkwave'
# Nice-to-have (pending the one sudo apt-get command in docs/ENV.md)
check socat        any    optional sh -c 'socat -V | grep -m1 "socat version"'
check ccache       any    optional ccache --version
check mold         any    optional mold --version
check autoconf     any    optional autoconf --version
check help2man     any    optional help2man --version
# Windows side
check vivado-entry any    required sh -c '[ -x "$SPLITSYS_ROOT/scripts/vivado" ] && printf "%s\n" "scripts/vivado -> $("$SPLITSYS_ROOT/scripts/vivado" --which)"'
check vivado.bat   any    required sh -c 'b="$("$SPLITSYS_ROOT/scripts/vivado" --which | sed "s#^C:#/mnt/c#; s#\\\\#/#g")"; [ -f "$b" ] && echo "$b"'
if [ "$QUICK" = 0 ]; then
    check vivado-version "$SPLITSYS_VIVADO_VERSION" required sh -c '"$SPLITSYS_ROOT/scripts/vivado" -version | grep -m1 -i "^vivado v"'
else
    row vivado-version "$SPLITSYS_VIVADO_VERSION" "(skipped: --quick)" SKIP
fi
check edge-headless any   required sh -c 'f="/mnt/c/Program Files (x86)/Microsoft/Edge/Application/msedge.exe"; [ -x "$f" ] && echo "msedge.exe present"'
check urbana-xdc   any    required sh -c '[ -s "$SPLITSYS_ROOT/fpga/xdc/urbana_master.xdc" ] && head -1 "$SPLITSYS_ROOT/fpga/xdc/urbana_master.xdc" | sed "s/^# *//"'
check usbipd-win   any    optional sh -c '[ -x "/mnt/c/Program Files/usbipd-win/usbipd.exe" ] && echo "usbipd.exe present"'
check wsl-keepalive any   optional sh -c 'pgrep -f "^splitsys-keepalive" >/dev/null && echo "running (scripts/wsl_keepalive)"'
check win-cargo    any    optional sh -c 'cd /mnt/c && cmd.exe /d /c "where cargo" 2>/dev/null | grep -i cargo'

if [ "$SMOKE" = 1 ]; then
    if "$HERE/smoke_cocotb.sh" >/tmp/splitsys_smoke.log 2>&1; then
        row smoke-cocotb any "$(tail -1 /tmp/splitsys_smoke.log)" OK
    else
        row smoke-cocotb any "see /tmp/splitsys_smoke.log" FAIL; FAILS=$((FAILS + 1))
    fi
fi

echo
if [ "$FAILS" -eq 0 ]; then
    echo "check_env: ALL REQUIRED OK"
else
    echo "check_env: $FAILS REQUIRED ITEM(S) FAILED"
fi
exit $(( FAILS > 0 ? 1 : 0 ))
