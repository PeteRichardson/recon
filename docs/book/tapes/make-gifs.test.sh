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

# expect_captions NAME TAPE: --captions on TAPE must print stdin exactly.
expect_captions() {
  cat > "$TMP/expected"
  expect "$1" bash -c 'bash "$0" --captions "$1" | diff - "$2"' "$MAKE" "$2" "$TMP/expected"
}

# Caption times come from the tape. Worked by hand, at 100ms a key:
#   Sleep 1s                      0.0 -> 1.0   "first" starts at 1.0
#   Type "ab"                     1.0 -> 1.2
#   Sleep 500ms                   1.2 -> 1.7   "second" starts at 1.7
#   Enter                         1.7 -> 1.8
#   Sleep 1s                      1.8 -> 2.8   end of the GIF
cat > "$TMP/timed.tape" <<'TAPE'
Output timed.gif
Set Width 600
Set Height 200
Set TypingSpeed 100ms
Sleep 1s
#> first
Type "ab"
Sleep 500ms
#> second
Enter
Sleep 1s
TAPE
expect_captions "--captions prints each caption from its step to the next" "$TMP/timed.tape" <<'ASS'
Dialogue: 0,0:00:01.00,0:00:01.70,Default,,0,0,0,,first
Dialogue: 0,0:00:01.70,0:00:02.80,Default,,0,0,0,,second
ASS

# The same rules through Source, with a hidden part that the GIF does not
# show and so adds no time. At 100ms a key, from common.tape:
#   Type@300ms "jj"               0.0 -> 0.6
#   Down 2                        0.6 -> 0.8   "two" starts at 0.8
#   Enter@500ms                   0.8 -> 1.3
#   Sleep 1s                      1.3 -> 2.3   end of the GIF
cat > "$TMP/common.tape" <<'TAPE'
Set TypingSpeed 100ms
Hide
Type "cd /tmp"
Enter
Sleep 5s
Show
TAPE
cat > "$TMP/sourced.tape" <<TAPE
Output sourced.gif
Source $TMP/common.tape
#> one
Type@300ms "jj"
Down 2
#> two
Enter@500ms
Sleep 1s
TAPE
expect_captions "--captions follows Source, Type@, repeats, and skips hidden time" "$TMP/sourced.tape" <<'ASS'
Dialogue: 0,0:00:00.00,0:00:00.80,Default,,0,0,0,,one
Dialogue: 0,0:00:00.80,0:00:02.30,Default,,0,0,0,,two
ASS

# < and > are plain text in a caption. { and } start an ASS override, so
# they are escaped.
printf 'Output x.gif\n#> recon <file> {x}\nSleep 1s\n' > "$TMP/braces.tape"
expect_captions "--captions keeps < and >, and escapes { and }" "$TMP/braces.tape" <<'ASS'
Dialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,recon <file> \{x\}
ASS

# A Wait takes a time that the tape does not say, so the captions after it
# would be at the wrong time.
printf 'Output x.gif\n#> wait\nWait /\\$/\n' > "$TMP/wait.tape"
expect "--check refuses Wait in a tape with captions" \
  bash -c '! bash "$0" --check "$1"' "$MAKE" "$TMP/wait.tape"

# The band for the captions is added only to a tape that has captions.
# valid.gif is still there from the size-limit test, with no captions.
height() { ffprobe -v error -select_streams v:0 -show_entries stream=height -of csv=p=0 "$1"; }
expect "a render adds no band to a tape without captions" \
  test "$(height "$TMP/valid.gif")" -eq 200
cat > "$TMP/captioned.tape" <<TAPE
Output "$TMP/captioned.gif"
Set Width 600
Set Height 200
#> say hi
Type "echo hi"
Enter
Sleep 300ms
TAPE
bash "$MAKE" "$TMP/captioned.tape" > "$TMP/render.log" 2>&1 || tail -5 "$TMP/render.log"
expect "a render adds a caption band to a tape with captions" \
  test "$(height "$TMP/captioned.gif")" -gt 200

echo "$PASSES passed, $FAILURES failed"
[ "$FAILURES" -eq 0 ]
