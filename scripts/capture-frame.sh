#!/usr/bin/env bash
# Capture one screen of `nuvem --demo` as coloured HTML text for the docs.
#
# Usage: scripts/capture-frame.sh <screen>... | --all
#
# Screens: home, resources, detail, palette, help, confirm, bigquery. Each is
# written to demo/frames/<screen>.html as a single <pre> fragment (see
# scripts/ansi2html.py). --all captures every screen.
#
# Needs tmux and python3. The nuvem binary comes from NUVEM_BIN (a path or a
# command on PATH), or else target/release/nuvem, or else target/debug/nuvem.
# FRAME_COLS and FRAME_ROWS set the terminal size; the default, 138x34, is the
# grid demo/tour.tape records.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
out="$root/demo/frames"
screens=(home resources detail palette help confirm bigquery)
cols=${FRAME_COLS:-138}
rows=${FRAME_ROWS:-34}

die() {
  echo "capture-frame: $*" >&2
  exit 1
}

# shellcheck source=scripts/demo-lib.sh
source "$root/scripts/demo-lib.sh"

usage() {
  echo "usage: scripts/capture-frame.sh <screen>... | --all" >&2
  echo "screens: ${screens[*]}" >&2
  exit 2
}

# The keys that lead from a fresh start to each screen. A pause follows every
# key, so each one lands after the previous screen has drawn.
keys_for() {
  case $1 in
    # Home as the app opens: every service's inventory is already loaded.
    home) echo "" ;;
    # GCE instances with the service navigator beside the list.
    resources) echo "g" ;;
    # An instance's detail, with the navigator hidden to give the detail pane
    # room.
    detail) echo "g v / w e b Enter Enter" ;;
    palette) echo "g :" ;;
    help) echo "g ?" ;;
    # Stop, from the palette, up to the confirmation that shows the API call.
    confirm) echo "g v / w e b Enter : s t o p Enter" ;;
    # A BigQuery dataset's tables, reached by the palette's bq alias, with
    # the navigator hidden so every column fits.
    bigquery) echo "g v : b q Enter Enter" ;;
    *) return 1 ;;
  esac
}

[[ $# -gt 0 ]] || usage
if [[ $1 == --all ]]; then
  set -- "${screens[@]}"
fi
for screen in "$@"; do
  keys_for "$screen" >/dev/null || usage
done

for tool in tmux python3; do
  command -v "$tool" >/dev/null || die "$tool is not on PATH"
done

bin=$(resolve_nuvem_bin "$root")

# A private tmux server whose socket lives in the scratch directory, started
# with no config file: the user's own tmux sessions and tmux.conf are never
# touched or read, and removing the directory removes the socket on every exit
# path, a failed run included.
work=$(mktemp -d)
socket="$work/tmux.sock"
cleanup() {
  tmux -S "$socket" kill-server 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT
write_demo_launcher "$work/nuvem-demo" "$bin" "$work"
mkdir -p "$out"
ctl=(tmux -S "$socket" -f /dev/null)

for screen in "$@"; do
  "${ctl[@]}" new-session -d -s frame -x "$cols" -y "$rows" \
    env TERM=xterm-256color "$work/nuvem-demo"
  "${ctl[@]}" set-option -g status off 2>/dev/null || true
  sleep 2
  if ! "${ctl[@]}" has-session -t frame 2>/dev/null; then
    echo "capture-frame: $bin exited before the $screen screen could be captured" >&2
    if [[ -s $work/nuvem.stderr ]]; then
      echo "its stderr:" >&2
      cat "$work/nuvem.stderr" >&2
    else
      echo "it wrote nothing to stderr" >&2
    fi
    exit 1
  fi
  read -ra keys <<<"$(keys_for "$screen")"
  # Home takes no keys; the guard keeps an empty array legal under set -u in
  # bash 3.2, which macOS ships.
  for key in ${keys[@]+"${keys[@]}"}; do
    "${ctl[@]}" send-keys -t frame "$key"
    sleep 0.5
  done
  sleep 1.5
  "${ctl[@]}" capture-pane -t frame -e -p |
    python3 "$root/scripts/ansi2html.py" >"$out/$screen.html"
  "${ctl[@]}" kill-session -t frame
  echo "wrote demo/frames/$screen.html"
done
