//! The side-effecting half of the app: everything that runs a subprocess,
//! reads the clock, or owns a background task.
//!
//! `gcloud/` owns auth, project context, and the typed command runner that all
//! `gcloud`/`bq` invocations go through. `resources/` builds the
//! service-specific reads and writes on top of it. `runtime/` initializes
//! tracing and the terminal, owns the render/input/effect loop, and dispatches
//! [`crate::app::Effect`] values to the layers below — it coordinates, and
//! deliberately implements no service details of its own.
//!
//! Resource business rules do not live here; they live in the reducers. Every
//! command receives its gcloud context explicitly from this layer rather than
//! reading ambient process state, so a widget or a reducer can never end up
//! naming a different configuration than the one nuvem is showing.
//!
//! `demo/` supplies scripted data for the demo example and the UI tests, so
//! frames can be rendered without a real cloud.

pub mod demo;
pub mod gcloud;
pub mod resources;
pub mod runtime;
pub mod sdk;
