//! The bridge that lets a synchronous effect run an asynchronous client call.
//!
//! `services::sdk::block_on` is reached from two kinds of thread and must work
//! on both. Listings are dispatched through `dispatch_background_refresh`,
//! which hops to a `spawn_blocking` thread; detail panes fall to the run
//! loop's catch-all and are awaited inline on a runtime worker. The first
//! implementation used `Handle::block_on`, which refuses to run on a worker --
//! "Cannot start a runtime from within a runtime" -- so every detail pane
//! crashed the process while every listing worked.
//!
//! No test caught it because the session harnesses hold no clients: with
//! `sdk_clients: None` the native branch is never taken, so the bridge was
//! only ever exercised from the blocking side. These call it from both.

use std::time::Duration;

use nuvem::services::sdk::{block_on, block_on_cancellable};
use tokio_util::sync::CancellationToken;

/// The case that panicked: a runtime worker, which is where the run loop
/// awaits an effect that opens a detail pane.
#[test]
fn the_bridge_runs_a_call_from_a_runtime_worker() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a multi-threaded runtime");

    let answer = runtime.block_on(async {
        // Not the outermost `block_on` above: this is *inside* the runtime, on
        // a worker driving it, which is the situation `dispatch_effect` is in.
        tokio::spawn(async { block_on(async { "loaded" }) })
            .await
            .expect("the task does not panic")
    });

    assert_eq!(answer, "loaded");
}

/// The case that always worked, kept so a future change cannot fix the worker
/// by breaking the blocking thread.
#[test]
fn the_bridge_runs_a_call_from_a_blocking_thread() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a multi-threaded runtime");

    let answer = runtime.block_on(async {
        tokio::task::spawn_blocking(|| block_on(async { "loaded" }))
            .await
            .expect("the blocking task does not panic")
    });

    assert_eq!(answer, "loaded");
}

/// The runtime keeps running while the bridge blocks.
///
/// This is what `block_in_place` buys over simply blocking the worker: the
/// thread hands its queued work to a sibling first. A timer scheduled before
/// the bridge starts must still fire while it is blocked, or the poll, the
/// key reader and the frame clock would all stop for the length of every
/// request.
#[test]
fn the_runtime_keeps_running_while_the_bridge_blocks() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a multi-threaded runtime");

    runtime.block_on(async {
        let ticked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&ticked);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        tokio::spawn(async move {
            block_on(async {
                std::thread::sleep(Duration::from_millis(200));
            });
        })
        .await
        .expect("the blocking task does not panic");

        assert!(
            ticked.load(std::sync::atomic::Ordering::SeqCst),
            "a timer scheduled before the bridge blocked must still have fired: \
             the runtime is not allowed to stop for the length of a request"
        );
    });
}

/// The cancellable twin takes the same bridge, so it is reachable from the
/// same two places.
#[test]
fn the_cancellable_bridge_runs_from_a_runtime_worker_and_honours_the_token() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a multi-threaded runtime");

    runtime.block_on(async {
        let live = CancellationToken::new();
        let finished = tokio::spawn({
            let live = live.clone();
            async move { block_on_cancellable(async { "loaded" }, &live) }
        })
        .await
        .expect("the task does not panic");
        assert_eq!(
            finished,
            Some("loaded"),
            "an uncancelled call returns its value"
        );

        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let abandoned = tokio::spawn({
            let cancelled = cancelled.clone();
            async move {
                block_on_cancellable(
                    async {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        "loaded"
                    },
                    &cancelled,
                )
            }
        })
        .await
        .expect("the task does not panic");
        assert_eq!(
            abandoned, None,
            "a cancelled call abandons the request rather than waiting it out"
        );
    });
}
