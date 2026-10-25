#!/usr/bin/env sh
# Fetch karpathy/tinyllamas stories260K (llama2.c format, MIT license) into
# models/stories260K/ and verify pinned sha256 digests.
#
# Provenance (checked 2026-10-24):
#   repo     https://huggingface.co/karpathy/tinyllamas  (license: mit)
#   revision 0bd21da7698eaf29a0d7de3992de8a46ef624add  (pinned; lastModified 2023-08-15)
#   digests  = the Hugging Face LFS oids (sha256) reported by
#              https://huggingface.co/api/models/karpathy/tinyllamas/tree/main/stories260K
#
# The downloaded files are DATA ONLY: this script never executes, sources or
# unpacks them. A file whose digest does not match is deleted.
set -eu

REV=0bd21da7698eaf29a0d7de3992de8a46ef624add
BASE="https://huggingface.co/karpathy/tinyllamas/resolve/$REV/stories260K"
HERE=$(cd "$(dirname "$0")" && pwd)
DEST="$HERE/stories260K"
mkdir -p "$DEST"

# name  size  sha256
FILES="stories260K.bin 1056540 b0a507e7ad0f626624f17112325e66691f9076d622e1d3274d103d00299f2696
tok512.bin 6227 037cb335abb25d1fa9e8ecae30ed2a3a8ace9302862ebcdc05d51a6bbb10c312"

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

echo "$FILES" | while read -r name size sum; do
    out="$DEST/$name"
    if [ -f "$out" ] && [ "$(sha256_of "$out")" = "$sum" ]; then
        echo "ok (cached)  $name"
        continue
    fi
    tmp="$out.part"
    rm -f "$tmp"
    curl -fsSL --proto '=https' --max-filesize 2000000 -o "$tmp" "$BASE/$name"
    got_size=$(wc -c <"$tmp" | tr -d ' ')
    got_sum=$(sha256_of "$tmp")
    if [ "$got_size" != "$size" ] || [ "$got_sum" != "$sum" ]; then
        rm -f "$tmp"
        echo "FAIL $name: size $got_size (want $size), sha256 $got_sum (want $sum)" >&2
        exit 1
    fi
    mv "$tmp" "$out"
    echo "ok           $name  sha256=$sum"
done
