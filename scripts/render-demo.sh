#!/usr/bin/env bash
# Render the demo recordings from demo/tour.tape into demo/out/:
#
#   nuvem-demo.gif         README recording
#   nuvem-demo.webm        site hero video (VP9, no audio)
#   nuvem-demo.mp4         site hero video (H.264, no audio)
#   nuvem-demo-poster.png  still frame for the video's poster and for
#                          reduced-motion readers
#
# The terminal's colours come from scripts/ansi2html.py, the same palette the
# docs frames use: the script prepends them to the tape as a `Set Theme` line.
#
# Needs vhs (which brings ttyd), ffmpeg and python3. The nuvem binary comes from
# NUVEM_BIN (a path or a command on PATH), or else target/release/nuvem, or
# else target/debug/nuvem; build one first. Usage: scripts/render-demo.sh
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
out="$root/demo/out"

die() {
  echo "render-demo: $*" >&2
  exit 1
}

# shellcheck source=scripts/demo-lib.sh
source "$root/scripts/demo-lib.sh"

for tool in vhs ttyd ffmpeg python3; do
  command -v "$tool" >/dev/null ||
    die "$tool is not on PATH; see demo/README.md for how to install it"
done

bin=$(resolve_nuvem_bin "$root")

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
write_demo_launcher "$work/nuvem-demo" "$bin" "$work"

theme=$(python3 "$root/scripts/ansi2html.py" --vhs-theme)
{
  printf 'Set Theme %s\n' "$theme"
  cat "$root/demo/tour.tape"
} >"$work/tour.tape"

(
  cd "$work"
  NUVEM_DEMO_LAUNCHER="$work/nuvem-demo" \
    vhs --quiet tour.tape -o tour.gif -o tour.mp4 -o tour.webm
)

# VHS already encodes both videos as yuv420p with no audio track. Remux the
# MP4 without re-encoding to move its index to the front (faststart), so a
# browser can start playing before the whole file arrives; -an makes the
# missing audio track a guarantee rather than a default.
ffmpeg -loglevel error -y -i "$work/tour.mp4" -an -c copy -movflags +faststart \
  "$work/nuvem-demo.mp4"
ffmpeg -loglevel error -y -i "$work/tour.webm" -an -c copy "$work/nuvem-demo.webm"
mv "$work/tour.gif" "$work/nuvem-demo.gif"
mv "$work/poster.png" "$work/nuvem-demo-poster.png"

# The README and the site's landing page load these on first view; keep them
# small. Lengthening the tour or raising the size or frame rate shows up here.
# Checked before anything reaches demo/out, so a failed run leaves the
# committed files alone.
check_size() {
  local file=$1 limit=$2 size
  size=$(wc -c <"$file")
  ((size <= limit)) || die "$(basename "$file") is $size bytes, over the $limit byte budget"
}
check_size "$work/nuvem-demo.gif" 3000000
check_size "$work/nuvem-demo.mp4" 1500000
check_size "$work/nuvem-demo.webm" 1500000

mkdir -p "$out"
for file in nuvem-demo.gif nuvem-demo.mp4 nuvem-demo.webm nuvem-demo-poster.png; do
  mv "$work/$file" "$out/$file"
done
ls -l "$out"
