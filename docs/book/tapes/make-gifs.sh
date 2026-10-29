#!/usr/bin/env bash
# Make the book's GIFs from the VHS tapes in this directory (#479).
#
#   make-gifs.sh                  render every tape
#   make-gifs.sh a.tape b.tape    render only these
#   make-gifs.sh --check [...]    validate the tapes, render nothing
#   make-gifs.sh --captions x.tape  print the tape's captions as ASS events
#
# MAKE_GIFS_NO_BUILD=1 uses the recon on PATH and does not build one.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)

# The tapes cd to DEMO, and recon reads its config from DEMO_CONFIG. Fixed
# short paths, so a GIF shows the same path on each computer and never the
# path of this checkout. lib/common.tape has the same two paths.
DEMO=/tmp/recon-demo
DEMO_CONFIG=/tmp/recon-demo-config

# Print a tape with each Source line replaced by the tape it names. A
# relative Source path is relative to the repo root, as VHS reads it there.
expand() {
  local line path
  while IFS= read -r line || [ -n "$line" ]; do
    if [[ $line =~ ^Source[[:space:]]+\"?([^\"]+)\"?$ ]]; then
      path=${BASH_REMATCH[1]}
      [[ $path = /* ]] || path="$ROOT/$path"
      expand "$path"
    else
      printf '%s\n' "$line"
    fi
  done < "$1"
}

# Print the captions of a tape as ASS events. A caption is a `#> text` line,
# and it shows from the command after it to the next caption, or to the end.
# The times come from the tape: each Sleep, and each key at the typing speed
# (or at the speed after `@`). A hidden part is not in the GIF, so it adds
# no time. SCALE (default 1) multiplies each time, to fit the real GIF.
captions() {
  expand "$1" | awk -v scale="${2:-1}" '
    function ms(d) { return d ~ /ms$/ ? d + 0 : d * 1000 }
    function stamp(t) {
      t = int(t * scale / 10 + 0.5)
      return sprintf("%d:%02d:%02d.%02d", t / 360000, t / 6000 % 60, t / 100 % 60, t % 100)
    }
    function close_caption() {
      if (text != "") printf "Dialogue: 0,%s,%s,Default,,0,0,0,,%s\n", stamp(start), stamp(now), text
    }
    function add(t) { if (!hidden) now += t }
    BEGIN { speed = 50 }
    /^#> / {
      close_caption(); start = now
      text = substr($0, 4); gsub(/[{}]/, "\\\\&", text)
      next
    }
    /^#/ || NF == 0 { next }
    $1 == "Hide" { hidden = 1; next }
    $1 == "Show" { hidden = 0; next }
    $1 == "Set" && $2 == "TypingSpeed" { speed = ms($3); next }
    $1 == "Sleep" { add(ms($2)); next }
    $1 ~ /^(Output|Set|Env|Require|Screenshot|Copy|Paste|Wait)/ { next }
    {
      i = index($1, "@"); at = i ? ms(substr($1, i + 1)) : speed
      if ($1 ~ /^Type/) {
        line = $0; sub(/^Type[^"]*"/, "", line); sub(/"$/, "", line)
        add(length(line) * at)
      } else {
        add((NF > 1 ? $2 : 1) * at)
      }
    }
    END { close_caption() }
  '
}

if [ "${1:-}" = "--captions" ]; then captions "$2"; exit; fi

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
  # A Wait takes a time that the tape does not say, so the captions after it
  # would be at the wrong time.
  if expand "$tape" | grep -q '^#> ' && expand "$tape" | grep -q '^Wait'; then
    echo "$tape: a tape with captions cannot use Wait" >&2
    status=1
  fi
done
[ "$status" -eq 0 ] || exit "$status"
$check && exit 0

if [ -z "${MAKE_GIFS_NO_BUILD:-}" ]; then
  # With no git, the build stamp is the version only. The keymap overlay
  # shows the stamp, and with git it has the path of this checkout. A target
  # directory of its own keeps the normal build's stamp as it is.
  GIT_DIR=/nonexistent cargo build --release --quiet --target-dir "$ROOT/target/gifs"
  PATH="$ROOT/target/gifs/release:$PATH"
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

# Write the captions of TAPE into a band under the terminal in GIF, in the
# terminal's own background color. The ASS file has the GIF's size as its
# play resolution, so the font size and margin are in pixels. Simple shaping
# gives a monospace font no ligatures ("fi"). The times are scaled to the
# real length of the GIF, which VHS makes a little shorter than the tape.
# The palette is made again, so that the GIF keeps its colors.
BAND=44
add_captions() {
  local tape=$1 gif=$2 ass="$work/captions.ass" bg w h end real
  bg=$(ffmpeg -v error -i "$gif" -frames:v 1 -vf crop=1:1:0:0 -f rawvideo -pix_fmt rgb24 - | xxd -p)
  IFS=x read -r w h < <(ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of csv=p=0:s=x "$gif")
  h=$((h + BAND))
  end=$(captions "$tape" | tail -1 | cut -d, -f3)
  real=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$gif")
  {
    printf '[Script Info]\nScriptType: v4.00+\nPlayResX: %d\nPlayResY: %d\n\n' "$w" "$h"
    printf '[V4+ Styles]\n'
    printf 'Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n'
    printf 'Style: Default,Menlo,22,&H00E8E8E8,&H00E8E8E8,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,2,10,10,%d,1\n\n' $(( (BAND - 22) / 2 ))
    printf '[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n'
    captions "$tape" "$(awk -v e="$end" -v r="$real" 'BEGIN { split(e, t, ":"); print r / (t[1] * 3600 + t[2] * 60 + t[3]) }')"
  } > "$ass"
  ffmpeg -v error -y -i "$gif" -filter_complex "\
[0:v]pad=iw:ih+$BAND:0:0:color=0x$bg,ass=$ass:shaping=simple,\
split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=none" "$work/out.gif"
  mv "$work/out.gif" "$gif"
}

# The ffmpeg in Homebrew core has no ass filter (libass).
need_libass() {
  # Not grep -q: it stops early, ffmpeg gets SIGPIPE, and pipefail fails.
  ffmpeg -hide_banner -filters 2>/dev/null | grep ' ass ' > /dev/null && return
  echo "make-gifs.sh: this ffmpeg has no ass filter (libass). Install one:" >&2
  echo "  brew uninstall --ignore-dependencies ffmpeg" >&2
  echo "  brew tap homebrew-ffmpeg/ffmpeg && brew install homebrew-ffmpeg/ffmpeg/ffmpeg" >&2
  exit 1
}
if grep -q '^#> ' "${tapes[@]}"; then need_libass; fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

for tape in "${tapes[@]}"; do
  "${clean[@]}" vhs "$tape"
  out=$(awk '$1 == "Output" { gsub(/"/, "", $2); print $2 }' "$tape")
  [ -n "$out" ] || { echo "$tape: no Output line" >&2; exit 1; }
  if grep -q '^#> ' "$tape"; then add_captions "$tape" "$out"; fi
  size=$(wc -c < "$out")
  if [ "$size" -gt "$max" ]; then
    echo "$out: $size bytes is over the limit of $max" >&2
    status=1
  fi
done
exit "$status"
