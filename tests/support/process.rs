//! Shared real-process helpers for the test binaries that spawn children and
//! then have to make a claim about what the kernel still holds.
//!
//! `tests/gcloud_runner_test.rs` and `tests/ci_reliability_evidence.rs`
//! both pull this in with `#[path = "support/process.rs"]`, so it lives in a
//! subdirectory: a plain `tests/*.rs` file would become a test target of its
//! own.
//!
//! `has_unreaped_child` is here rather than copied into each binary because it
//! encodes an argument about pid recycling that nobody re-derives on the way
//! past — see its own comment. Two copies of it are two chances for one of them
//! to stop meaning what its comment says.
//!
//! Both helpers are called by both binaries, so — unlike the other support
//! modules here — this one carries no `allow(dead_code)`. It should not acquire
//! one: a dead-code warning from this file means a helper has lost its last
//! caller, which is exactly the thing worth being told about.
#![cfg(unix)]

use std::time::{Duration, Instant};

/// Whether this process still has an un-reaped child with this pid.
///
/// `waitpid(WNOHANG)` failing with `ECHILD` proves there is none. The obvious
/// probe — asserting `kill(pid, 0) == -1` — would instead flake, because it goes
/// true again the moment the OS recycles the pid onto an unrelated process.
/// `waitpid` does not: an unrelated process is not a child of ours, so a
/// recycled id still answers `ECHILD`.
///
/// That is narrower than immunity, and the difference is worth stating rather
/// than rounding off. The one recycling this cannot see through is the id being
/// handed to a NEW CHILD OF THIS SAME TEST BINARY — a sibling test spawning
/// while we ask. Then the pid really is our child again, the answer flips to
/// true, and on a child that has already exited we would also consume the
/// sibling's zombie out from under it. So ask promptly, about a pid whose run
/// has just returned; do not keep a pid around and re-ask later.
///
/// It reaps: on a pid that IS still an un-reaped child this consumes the zombie.
/// Callers that need the child to stay waitable must peek with `WNOWAIT`
/// instead.
pub(crate) fn has_unreaped_child(pid: libc::pid_t) -> bool {
    let mut status = 0;
    // SAFETY: the only pointer is `&mut status`, a live `c_int` on this frame
    // that `waitpid` may write once. `WNOHANG` means the call returns without
    // blocking whatever `pid` names, and the doc above bounds which pids callers
    // may pass — the reaping this performs is a caller obligation, not a memory
    // one.
    let observed = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
    !(observed == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD))
}

/// Poll `condition` until it holds or `deadline` elapses.
///
/// A test waiting for the OS to catch up gets to fail on its own assertion
/// message instead of racing, and a condition that never comes true fails that
/// test rather than hanging the job until CI's global timeout. The deadline
/// belongs to the call site, which is the only place that knows how long the
/// thing being waited for may legitimately take.
pub(crate) fn wait_until(deadline: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    condition()
}
