#!/usr/bin/env bash
# Tests for make-gifs.sh, the one seam of the book's GIF tapes (#479).
# Run from anywhere: bash docs/book/tapes/make-gifs.test.sh
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
MAKE="$HERE/make-gifs.sh"
PASSES=0; FAILURES=0
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT

# No release build: these tapes do not run recon, and a build in a new
# worktree takes minutes. A stub stands in for the config check.
export MAKE_GIFS_NO_BUILD=1
mkdir "$TMP/bin"; printf '#!/bin/sh\nexit 0\n' > "$TMP/bin/recon"
chmod +x "$TMP/bin/recon"; export PATH="$TMP/bin:$PATH"

# expect NAME CMD...: CMD must succeed.
expect() {
  local name=$1; shift
  if "$@" >/dev/null 2>&1; then
    PASSES=$((PASSES + 1)); echo "ok   $name"
  else
    FAILURES=$((FAILURES + 1)); echo "FAIL $name"
  fi
}

tape() {
  cat > "$TMP/$1.tape" <<TAPE
Output "$TMP/$1.$2"
Set Width 600
Set Height 200
Type '$3'
Enter
Sleep 300ms
TAPE
}

tape valid gif "echo hi"
expect "--check accepts a valid tape" bash "$MAKE" --check "$TMP/valid.tape"

printf 'Output x.gif\nNotACommand\n' > "$TMP/invalid.tape"
expect "--check refuses a tape that VHS cannot parse" \
  bash -c '! bash "$0" --check "$1"' "$MAKE" "$TMP/invalid.tape"

# The tape must not see the caller's environment: a RECON_* variable or an
# EDITOR from the user's shell would change what the GIF shows. It must
# see the demo config.
tape env txt 'echo "FOO=$FOO XDG=$XDG_CONFIG_HOME" > '"$TMP/env.out"
FOO=leaked bash "$MAKE" "$TMP/env.tape" >/dev/null 2>&1
expect "a render does not pass the caller's environment to the tape" \
  grep -qx 'FOO= XDG=/tmp/recon-demo-config' "$TMP/env.out"

expect "a render makes the GIF that the tape names" \
  bash -c 'bash "$0" "$1" && [ -s "$2" ]' "$MAKE" "$TMP/valid.tape" "$TMP/valid.gif"

# A GIF over the limit fails the run, so a long recording is not committed
# by accident. GIF_MAX_BYTES lowers the limit for the test.
rm -f "$TMP/valid.gif"
expect "a render refuses a GIF over the size limit" \
  bash -c '! GIF_MAX_BYTES=10 bash "$0" "$1" && [ -s "$2" ]' \
  "$MAKE" "$TMP/valid.tape" "$TMP/valid.gif"

echo "$PASSES passed, $FAILURES failed"
[ "$FAILURES" -eq 0 ]
