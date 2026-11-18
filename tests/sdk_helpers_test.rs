//! The helpers every native mapper leans on: the two spellings a row must
//! agree on whichever backend filled it, and the cancellable bridge from a
//! blocking effect thread into a client future.

use nuvem::services::sdk::{block_on, block_on_cancellable, non_empty, timestamp_to_string};

#[test]
fn a_protobuf_timestamp_renders_the_way_the_cli_does() {
    let stamp =
        google_cloud_wkt::Timestamp::new(1_704_164_645, 500_000_000).expect("a valid timestamp");
    assert_eq!(
        timestamp_to_string(Some(&stamp)).as_deref(),
        Some("2024-01-02T03:04:05Z"),
        "seconds are rendered in the CLI's UTC shape and sub-second precision is dropped"
    );
    assert_eq!(
        timestamp_to_string(None),
        None,
        "no timestamp is no timestamp, not an empty one"
    );
}

#[test]
fn an_empty_proto_string_is_absent_and_a_non_empty_one_is_kept() {
    assert_eq!(non_empty(""), None);
    assert_eq!(non_empty("us-central1").as_deref(), Some("us-central1"));
    assert_eq!(
        non_empty(" ").as_deref(),
        Some(" "),
        "only the empty string is the unset sentinel"
    );
}

#[test]
fn a_cancelled_token_abandons_the_call_and_a_live_one_returns_its_output() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let live = tokio::task::spawn_blocking(|| {
            let token = tokio_util::sync::CancellationToken::new();
            block_on_cancellable(async { 42 }, &token)
        })
        .await
        .expect("blocking task");
        assert_eq!(live, Some(42));

        let cancelled = tokio::task::spawn_blocking(|| {
            let token = tokio_util::sync::CancellationToken::new();
            token.cancel();
            block_on_cancellable(std::future::pending::<i32>(), &token)
        })
        .await
        .expect("blocking task");
        assert_eq!(
            cancelled, None,
            "a cancelled token means no answer, not a hang"
        );

        let plain = tokio::task::spawn_blocking(|| block_on(async { "done" }))
            .await
            .expect("blocking task");
        assert_eq!(plain, "done");
    });
}
