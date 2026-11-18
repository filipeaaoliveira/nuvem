//! Google Cloud concepts, modelled as plain data.
//!
//! Each submodule owns one concern — configurations and contexts, projects,
//! resources and their status, locations, filters and matchers, money,
//! diagnostics, plugin output, tunnels, argv safety — and none of them
//! performs I/O or knows how it will be rendered.
//!
//! Modules land here rather than in a layer above precisely when two layers
//! would otherwise have to import from each other: [`filter`], for instance,
//! is parsed once and read both by the client-side predicate in the app layer
//! and by the server-side `gcloud --filter` pushdown in the service layer, so
//! the two can never disagree about what an input means.

pub mod argv;
pub mod configuration;
pub mod diagnostics;
pub mod filter;
pub mod finops;
pub mod k9s;
pub mod location;
pub mod logging;
pub mod matcher;
pub mod money;
pub mod plugin_output;
pub mod project;
pub mod resource;
pub mod resource_call;
pub mod text;
pub mod text_filter;
pub mod tunnel;
