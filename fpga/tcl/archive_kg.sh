#!/usr/bin/env bash
# =============================================================================
# archive_kg.sh - archive a hardware-verified bitstream as a known-good build
# (owner: fpga-integrator). Run from Git Bash (or WSL) at any cwd:
#
#   bash fpga/tcl/archive_kg.sh <KGn> <tag> <log file> [<log file> ...] [--no-commit]
#   e.g. bash fpga/tcl/archive_kg.sh KG1 top h3.log h4.log h5.log
#
# Refuses (exit 1, nothing written) unless:
#   - <KGn> matches KG<number> and fpga/known_good/<KGn> does not exist yet
#     (an existing KG is never overwritten or deleted);
#   - fpga/reports/manifest.json has a row for <tag>, and the md5 of
#     fpga/build/<tag>/top.bit equals that row's md5 (i.e. the file on disk is
#     the reviewed build that was tested on the board);
#   - fpga/reports/<tag>/summary.json has pass true, git_dirty "false", and the
#     same git_hash/build_id as the manifest row;
#   - every log file exists, is non-empty, and the basenames are distinct.
# fpga/build/ is only read, never modified.
#
# Writes fpga/known_good/<KGn>/:
#   top.bit             copy of fpga/build/<tag>/top.bit (md5 re-checked)
#   reports/            copy of fpga/reports/<tag>/ (summary.json, *.rpt, build.log)
#   summary.json        copy of reports/summary.json (top-level, for scripts)
#   manifest_row.json   the manifest row for <tag>
#   git_hash.txt        the BUILD commit (summary.json git_hash = what ping
#                       prints as build_id), not the archive-time HEAD
#   archive.txt         archive time, HEAD at archive time, tag, md5, log list
#   logs/               the hardware test logs given on the command line
# then fpga/known_good/CURRENT = "fpga/known_good/<KGn>/top.bit" (the pointer
# format program.tcl follows), and commits only those two paths.
# =============================================================================
set -uo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fail() { echo "ARCHIVE_KG_FAIL: $*" >&2; exit 1; }

commit=1
args=()
for a in "$@"; do
  if [ "$a" = "--no-commit" ]; then commit=0; else args+=("$a"); fi
done
[ "${#args[@]}" -ge 3 ] || { echo "usage: archive_kg.sh <KGn> <tag> <log file> [<log file> ...] [--no-commit]" >&2; exit 2; }
kg="${args[0]}"; tag="${args[1]}"; logs=("${args[@]:2}")

# Resolve log paths before changing directory (they may be relative to the caller's cwd).
abs_logs=()
for f in "${logs[@]}"; do
  [ -f "$f" ] || fail "log file not found: $f"
  [ -s "$f" ] || fail "log file is empty: $f"
  abs_logs+=("$(cd "$(dirname "$f")" && pwd)/$(basename "$f")")
done
dups="$(for f in "${abs_logs[@]}"; do basename "$f"; done | sort | uniq -d)"
[ -z "$dups" ] || fail "log files with the same name (rename one): $dups"

cd "$root" || fail "cannot cd to $root"
[[ "$kg" =~ ^KG[0-9]+$ ]] || fail "KG name must be KG<number>, got '$kg'"
dest="fpga/known_good/$kg"
[ ! -e "$dest" ] && ! git cat-file -e "HEAD:$dest" 2>/dev/null || fail "$dest already exists; never overwrite a KG, use the next number"

bit="fpga/build/$tag/top.bit"
rep="fpga/reports/$tag"
[ -f "$bit" ] || fail "$bit missing"
[ -f "$rep/summary.json" ] || fail "$rep/summary.json missing"
[ -f fpga/reports/manifest.json ] || fail "fpga/reports/manifest.json missing"

PY="$(command -v python || command -v python3)" || fail "python not found"
# Prints line 1: <manifest md5> <git_hash> <build_id> (or "ERR <reason>"),
# then the manifest row as JSON. Writes nothing.
check="$("$PY" -I - "$tag" fpga/reports/manifest.json "$rep/summary.json" <<'EOF'
import json, sys
tag, man_path, sum_path = sys.argv[1:4]
rows = [r for r in json.load(open(man_path, encoding="utf-8")) if r.get("tag") == tag]
if len(rows) != 1:
    print(f"ERR manifest has {len(rows)} rows for tag {tag}"); sys.exit()
r = rows[0]
s = json.load(open(sum_path, encoding="utf-8"))
errs = []
if not r.get("md5"):
    errs.append("manifest row has no md5")
if s.get("pass") is not True:
    errs.append("summary.json pass is not true")
if s.get("git_dirty") != "false":
    errs.append(f"summary.json git_dirty={s.get('git_dirty')}")
if s.get("git_hash") != r.get("git_hash"):
    errs.append(f"summary git_hash {s.get('git_hash')} != manifest {r.get('git_hash')}")
if s.get("build_id") != r.get("build_id"):
    errs.append(f"summary build_id {s.get('build_id')} != manifest {r.get('build_id')}")
if errs:
    print("ERR " + "; ".join(errs)); sys.exit()
print(r["md5"], r["git_hash"], r["build_id"])
print(json.dumps(r, indent=2))
EOF
)" || fail "python check failed"
check="${check//$'\r'/}"   # Windows python prints CRLF
case "$check" in ERR*|"") fail "${check#ERR }";; esac
read -r want_md5 git_hash build_id <<< "$(head -n 1 <<< "$check")"
row_json="$(tail -n +2 <<< "$check")"

have_md5="$(md5sum "$bit" | cut -d' ' -f1)"
[ "$have_md5" = "$want_md5" ] || fail "$bit md5 $have_md5 != manifest $want_md5 (row $tag): the file on disk is not the reviewed build"
echo "archive_kg: $bit md5 $have_md5 == manifest row $tag; build $git_hash ($build_id)"

mkdir -p "$dest/reports" "$dest/logs" || fail "mkdir $dest"
cp "$bit" "$dest/top.bit" || fail "copy bit"
cp -r "$rep/." "$dest/reports/" || fail "copy reports"
cp "$rep/summary.json" "$dest/summary.json" || fail "copy summary"
printf '%s\n' "$row_json" > "$dest/manifest_row.json" || fail "write manifest_row.json"
echo "$git_hash" > "$dest/git_hash.txt"
for f in "${abs_logs[@]}"; do cp "$f" "$dest/logs/" || fail "copy log $f"; done
{
  echo "kg: $kg"
  echo "tag: $tag"
  echo "bit_md5: $have_md5"
  echo "build_git_hash: $git_hash"
  echo "build_id: $build_id"
  echo "archived_at: $(date '+%Y-%m-%d %H:%M:%S %Z')"
  echo "archive_head: $(git rev-parse HEAD)"
  echo "logs:"
  for f in "${abs_logs[@]}"; do echo "  $(basename "$f")  (from $f)"; done
} > "$dest/archive.txt"
[ "$(md5sum "$dest/top.bit" | cut -d' ' -f1)" = "$want_md5" ] || fail "copied bit md5 mismatch"

echo "$dest/top.bit" > fpga/known_good/CURRENT
echo "archive_kg: wrote $dest and fpga/known_good/CURRENT -> $dest/top.bit"

if [ "$commit" = 1 ]; then
  paths=("$dest" fpga/known_good/CURRENT)
  git add -- "${paths[@]}" || fail "git add"
  git commit -q -m "[fpga-integrator] $kg: archive $tag bit (md5 $have_md5, build $git_hash) + reports + HW test logs; CURRENT -> $kg" \
    -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>" -- "${paths[@]}" || fail "git commit"
  got="$(git show "HEAD:$dest/top.bit" | md5sum | cut -d' ' -f1)"
  [ "$got" = "$want_md5" ] || fail "committed top.bit md5 $got != $want_md5"
  echo "archive_kg: committed $(git log --oneline -1)"
fi
echo "ARCHIVE_KG_OK $kg $tag $have_md5"
