//! Domain types for nuvem: the stable vocabulary of Google Cloud concepts that
//! the reducers, services, UI projections, and tests all share.
//!
//! Everything here is pure data and pure functions — identifiers, summaries,
//! status and health enums, filter grammars, money and text helpers. A type
//! belongs in this crate when more than one layer above needs it, and putting
//! it here is what lets those layers agree on what a value means without
//! importing from one another.
//!
//! This crate must not depend on `ratatui`, `crossterm`, `tokio`, or `clap`,
//! and must not touch the filesystem, the network, or `gcloud`. A type that
//! mentions terminal layout, subprocesses, HTTP, credentials, paths, or CLI
//! flags belongs in the `nuvem` binary crate instead.
//!
//! - [`gcp`] — the GCP concept vocabulary, one module per concern.
//!
//! `missing_docs` is deliberately not enforced here. Turned on to survey the
//! crate it reported 652 items, and 533 of them were struct fields and enum
//! variants on plain data types, where the only doc that could be written
//! restates the name — `/// The zone.` on `zone`. Leaving it on would demand
//! that tautology of every future field, and a crate where most doc comments
//! say nothing is a crate where readers stop reading them. Modules, types and
//! methods are documented because there is something to say about them; a
//! field is documented when its meaning is not its name.

pub mod gcp;
