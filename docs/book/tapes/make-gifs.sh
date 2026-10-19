#!/usr/bin/env bash
# Make the book's GIFs from the VHS tapes in this directory (#479).
#
#   make-gifs.sh                  render every tape
#   make-gifs.sh a.tape b.tape    render only these
#   make-gifs.sh --check [...]    validate the tapes, render nothing
#
# MAKE_GIFS_NO_BUILD=1 uses the recon on PATH and does not build one.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)

# The tapes cd to DEMO, and recon reads its config from DEMO_CONFIG. Fixed
# short paths, so a GIF shows the same path on each computer and never the
# path of this checkout. common.tape has the same two paths.
DEMO=/tmp/recon-demo
DEMO_CONFIG=/tmp/recon-demo-config

check=false
if [ "${1:-}" = "--check" ]; then check=true; shift; fi
if [ $# -eq 0 ]; then set -- "$HERE"/*.tape; fi
tapes=()
for tape in "$@"; do tapes+=("$(cd "$(dirname "$tape")" && pwd)/$(basename "$tape")"); done

# Output and Source paths in a tape are relative to the repo root.
cd "$ROOT"

status=0
for tape in "${tapes[@]}"; do
  vhs validate "$tape" >/dev/null || status=1
done
[ "$status" -eq 0 ] || exit "$status"
$check && exit 0

if [ -z "${MAKE_GIFS_NO_BUILD:-}" ]; then
  cargo build --release --quiet
  PATH="$ROOT/target/release:$PATH"
fi

rm -rf "$DEMO" "$DEMO_CONFIG"
cp -R "$HERE/data" "$DEMO"
cp -R "$HERE/config" "$DEMO_CONFIG"
# A fixed date, because the file view's title shows it.
find "$DEMO" -exec touch -t 202610160900 {} +

# Only these variables reach recon. Anything else from the user's shell
# (RECON_FILTER_PATH, EDITOR, TERM_PROGRAM, ...) would change the GIF.
clean=(env -i HOME="$HOME" PATH="$PATH" XDG_CONFIG_HOME="$DEMO_CONFIG")

# A demo config that recon refuses would make a GIF of an error message.
"${clean[@]}" recon -b -q --emit files --set triage "$DEMO/logs" \
  > /dev/null < /dev/null

# GitHub refuses a GIF over 10 MB in an issue, and a large GIF makes each
# clone bigger, so the limit is half that.
max=${GIF_MAX_BYTES:-5242880}

for tape in "${tapes[@]}"; do
  "${clean[@]}" vhs "$tape"
  out=$(awk '$1 == "Output" { gsub(/"/, "", $2); print $2 }' "$tape")
  size=$(wc -c < "$out")
  if [ "$size" -gt "$max" ]; then
    echo "$out: $size bytes is over the limit of $max" >&2
    status=1
  fi
done
exit "$status"
