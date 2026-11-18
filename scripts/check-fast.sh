#!/usr/bin/env bash
# The local quality gates: fmt, clippy, shell lint and the test suite. CI runs
# the same checks (.github/workflows/ci.yml) plus rustdoc and cargo-deny.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

# THE TERMINAL MARKER
#
# A tier that was KILLED used to be indistinguishable from one that passed and
# from one still running: the log simply stopped, and absence of output was the
# only tell. Absence is a terrible signal - it is also what an in-progress run
# looks like, and a truncated log gets read as whatever the reader expects. So
# every exit path ends by printing one marker line, and the question "did this
# run reach its own end?" is answered by looking rather than by inferring:
#
#   EXIT=0            reached its end; every gate passed
#   EXIT=<n>          reached its end and refused; <n> is the status
#   EXIT=killed(...)  a signal ended it. NOTHING in this log is evidence, in
#                     either direction - not a pass, not a finding
#   no marker at all  the run is still going, or the log was truncated
#
# SIGKILL cannot be trapped, so the last line stays possible and absence still
# has to be read as "no evidence". What the trap removes is the far more common
# case - a reaper, a cancelled CI job, a lost terminal - being silent about it.
#
# The handler re-raises with the default disposition rather than exiting a
# number of its own, so the wait status a supervisor reads is still a real
# signal death (128+n).
tier_end_written=""
tier_end() {
    # Once only. Bash still runs the EXIT trap after the re-raise below, and a
    # second marker reading `EXIT=143` under one reading `killed(SIGTERM)` is
    # two answers to the question the marker exists to settle — measured, not
    # assumed: the fixture that signals this tier reported exactly that pair
    # before the guard was here.
    if [ -n "$tier_end_written" ]; then
        return 0
    fi
    tier_end_written=1
    printf '\n== check-fast end: EXIT=%s\n' "$1"
}
tier_signalled() {
    tier_end "killed(SIG$1)"
    trap - "$1"
    kill -s "$1" "$$"
}
# A signal that arrives while a gate is running is handled once that foreground
# child returns - which is the ordinary case rather than a gap, because a reaper
# signals the whole process group and the child dies with it.
trap 'tier_end "$?"' EXIT
trap 'tier_signalled TERM' TERM
trap 'tier_signalled INT' INT
trap 'tier_signalled HUP' HUP

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Shell lint, guarded so the linter is not a hard dependency for running the
# gates — but the guard says so out loud, because a gate that skips silently
# reads as coverage that ran. CI stays the hard enforcement. (Keep the word
# "shellcheck" off the start of comments: the linter reads a comment opening
# with its own name as a directive and errors out.)
if command -v shellcheck >/dev/null 2>&1; then
    shellcheck scripts/*.sh
else
    echo "note: shellcheck not installed; shell lint deferred to CI" >&2
fi

# nextest is process isolation as much as speed: a consolidated suite target
# shares one process under libtest, and tests that touch process-global state stop being
# isolated by construction the way per-file binaries isolated them. nextest
# runs each test in its own process. It runs no doctests, so the --doc run
# keeps those covered rather than letting them drop silently.
#
# A hard requirement rather than a preference, unlike the shellcheck guard
# above, because the two degrade differently. A missing linter
# skips a check and says so; a missing nextest ran the suite in the one
# configuration where this repo's process-global active theme
# (src/config/theme.rs) races across tests, and then reported the tier green.
# The note it printed was true and changed nothing: a tier whose degraded
# result is indistinguishable from its passing result attests to coverage it
# did not have. Refusing is what makes the note actionable.
if ! cargo nextest --version >/dev/null 2>&1; then
    echo "error: cargo-nextest is required. The integration tests are one binary" >&2
    echo "       (tests/suite.rs); under libtest they run as threads of one process" >&2
    echo "       and race the process-global active theme, so a libtest run is not" >&2
    echo "       the same check. Install it and re-run:" >&2
    echo "         cargo install cargo-nextest --locked   # or: brew install cargo-nextest" >&2
    exit 1
fi
cargo nextest run --all
cargo test --doc --all
