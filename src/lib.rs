//! `nuvem` — a terminal UI for operating Google Cloud through the local
//! `gcloud` CLI.
//!
//! The crate is split so that the rules of the application are testable
//! without a terminal or a subprocess. [`app`] holds typed actions and pure
//! reducers over `AppState`; a reducer that needs I/O returns an `Effect`
//! instead of performing one. [`services`] executes those effects — gcloud
//! invocations, resource reads, and the runtime loop that dispatches them —
//! and feeds results back as actions. [`ui`] renders projections of state and
//! decides nothing. [`input`] answers "what did the user ask to do?", never
//! "how should state change?".
//!
//! [`run`] is the whole public entry point; `main.rs` only parses flags,
//! resolves [`Config`], and calls it.
//!
//! - [`app`] — state, actions, reducers, effects, projections
//! - [`config`] — CLI flags, layered config resolution, persisted state
//! - [`exit`] — the exit-status table a script reads
//! - [`input`] — key events to semantic actions, per mode
//! - [`services`] — gcloud/resource I/O and the runtime loop
//! - [`ui`] — the shared application frame and workspace renderers

pub mod app;
pub mod config;
pub mod exit;
pub mod input;
pub mod services;
pub mod ui;

pub use config::{Cli, Command, Config, ConfigOutcome};

pub async fn run(outcome: ConfigOutcome, cli: Cli) -> anyhow::Result<()> {
    services::runtime::run(outcome, cli).await
}
