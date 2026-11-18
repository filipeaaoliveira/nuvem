//! The per-configuration client cache lets go of what the operator left.
//!
//! A client is bound to credentials, and the credentials carry a bearer
//! token; keying the cache by configuration name is what stops a switch being
//! served the wrong token, and evicting on a switch is what stops a session
//! that walked through many configurations from holding every token until it
//! exits. Building a client is offline until its first request, which is what
//! lets this run without a network.

use std::sync::Arc;

use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::sdk::SdkClients;

/// A runner nothing here may call: a client is built without a token, and the
/// token is only fetched on the first request, which these tests never make.
struct NeverRunner;

impl CommandRunner for NeverRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        panic!(
            "building a client must not invoke gcloud; got {:?}",
            command.args
        )
    }
}

fn clients() -> Arc<SdkClients> {
    Arc::new(SdkClients::new(Arc::new(NeverRunner)))
}

/// The cache methods drive a client's async construction from a blocking
/// thread, as the effect threads do; the test has to hand them the same shape.
fn on_blocking_thread<R: Send + 'static>(work: impl FnOnce() -> R + Send + 'static) -> R {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        tokio::task::spawn_blocking(work)
            .await
            .expect("blocking task")
    })
}

#[test]
fn a_switch_evicts_every_other_configurations_clients() {
    let cache = clients();
    let moved = Arc::clone(&cache);
    on_blocking_thread(move || {
        let dev = moved.storage("dev").expect("the dev client builds offline");
        let _prod = moved
            .compute("prod")
            .expect("the prod client builds offline");
        assert_eq!(moved.cached_configurations(), ["dev", "prod"]);

        moved.evict_except("prod");
        assert_eq!(
            moved.cached_configurations(),
            ["prod"],
            "everything but the configuration switched to is dropped"
        );

        let rebuilt = moved
            .storage("dev")
            .expect("a dropped client is rebuilt on demand");
        assert!(
            !Arc::ptr_eq(&dev, &rebuilt),
            "the dev client after eviction is a new one, not the old handle"
        );
    });
}

#[test]
fn a_switch_keeps_the_active_configurations_clients() {
    let cache = clients();
    let moved = Arc::clone(&cache);
    on_blocking_thread(move || {
        let before = moved.storage("dev").expect("builds offline");
        moved.evict_except("dev");
        let after = moved.storage("dev").expect("still cached");
        assert!(
            Arc::ptr_eq(&before, &after),
            "the configuration switched to keeps its client; a switch to the same \
             name must not rebuild anything"
        );
    });
}
