# shellcheck shell=bash
# Shared by render-demo.sh and capture-frame.sh; sourced, not run.
#
# Both scripts start `nuvem --demo` through a launcher written by
# write_demo_launcher, which runs the app with a scrubbed environment so that
# nothing on the machine doing the recording reaches a frame.

# Find the nuvem binary and print its absolute path. NUVEM_BIN wins, and may
# be a path or a command name on PATH; otherwise target/release/nuvem, then
# target/debug/nuvem under the repository root given as $1.
resolve_nuvem_bin() {
  local root=$1 bin=${NUVEM_BIN:-} candidate
  if [[ -n $bin ]]; then
    if [[ $bin != */* ]]; then
      candidate=$(type -P -- "$bin") ||
        die "NUVEM_BIN=$bin is not a path and not a command on PATH"
      bin=$candidate
    fi
    [[ -f $bin && -x $bin ]] || die "NUVEM_BIN is not an executable file: $bin"
  else
    for candidate in "$root/target/release/nuvem" "$root/target/debug/nuvem"; do
      if [[ -f $candidate && -x $candidate ]]; then
        bin=$candidate
        break
      fi
    done
    [[ -n $bin ]] || die "no nuvem binary: run cargo build --release, or set NUVEM_BIN"
  fi
  echo "$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")"
}

# Write an executable at $1 that runs the binary $2 as `nuvem --demo` with
# every per-user directory inside the scratch directory $3, and stderr saved
# to $3/nuvem.stderr.
#
# nuvem finds its config, plugins, state, log and cache through the platform
# directories: on Linux these honour the XDG_* variables, falling back to
# HOME; on macOS they derive from HOME alone. `env -i` drops everything else,
# including NUVEM_*, CLOUDSDK_* and GOOGLE_APPLICATION_CREDENTIALS. Only PATH,
# TERM and the locale pass through, and TERM is read when the launcher runs,
# so it is the recording terminal's own.
write_demo_launcher() {
  local launcher=$1 bin=$2 scratch=$3 var
  local -a fixed=(
    "HOME=$scratch/home"
    "XDG_CONFIG_HOME=$scratch/xdg/config"
    "XDG_CACHE_HOME=$scratch/xdg/cache"
    "XDG_STATE_HOME=$scratch/xdg/state"
    "XDG_DATA_HOME=$scratch/xdg/data"
    "PATH=$PATH"
  )
  for var in LANG LC_ALL LC_CTYPE; do
    if [[ -n ${!var:-} ]]; then
      fixed+=("$var=${!var}")
    fi
  done
  mkdir -p "$scratch/home" "$scratch/xdg/config" "$scratch/xdg/cache" \
    "$scratch/xdg/state" "$scratch/xdg/data"
  {
    echo '#!/usr/bin/env bash'
    printf 'exec env -i'
    printf ' %q' "${fixed[@]}"
    # shellcheck disable=SC2016 # expanded by the launcher, not here
    printf ' TERM="${TERM:-xterm-256color}"'
    printf ' %q --demo 2>%q\n' "$bin" "$scratch/nuvem.stderr"
  } >"$launcher"
  chmod +x "$launcher"
}
