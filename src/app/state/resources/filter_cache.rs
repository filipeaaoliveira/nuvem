//! The resource table's applied filter, cached with the filter it compiled to.
//!
//! This is a module of its own for one reason: **privacy scope**. Rust makes a
//! module's private fields visible to its *descendants*, so a `filter_applied`
//! whose fields were private to `resources` would still be writable field by
//! field from `compute.rs`, `gke.rs`, `cloudsql.rs`, `cloudrun.rs` and
//! `storage.rs` — the exact modules that used to assign the applied filter
//! directly, and the exact hazard this change exists to close. Fields private
//! to *this* module are reachable from nowhere else at all, so the constructor
//! really is the only way to produce a value and the compiler enforces it.
//!
//! Nothing here is `pub`: the type and its accessors are `pub(super)`, visible
//! to `resources` and its descendants and no further.

use nuvem_domain::gcp::filter::{CompiledFilter, FilterParseError, compile_filter};

/// An applied table filter together with the filter it compiled to.
///
/// The compile happens once per apply instead of once per call, which matters
/// because `visible_rows` runs on every render, and the table grammar is
/// regex-by-default — every one of those calls was compiling a regex.
///
/// `regex::Regex` implements neither `PartialEq` nor `Eq`, so [`CompiledFilter`]
/// cannot derive them — and does not need to. Both fields are a pure function
/// of `source`: `compiled` is produced by [`compile_filter`] in the constructor
/// and is never mutated afterwards, and neither field is reachable outside this
/// module, so no caller can change one without the other. Two values with equal
/// sources are therefore equal in every observable respect, and equality
/// compares the source alone. That reasoning is the whole warrant for the
/// hand-written impl below, and it holds only while the fields stay private to
/// this module and the only way to build one is [`Self::new`]. Do not widen
/// either.
#[derive(Debug, Clone)]
pub(super) struct CompiledResourceFilter {
    source: String,
    compiled: Result<CompiledFilter, FilterParseError>,
}

impl PartialEq for CompiledResourceFilter {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

impl Eq for CompiledResourceFilter {}

impl Default for CompiledResourceFilter {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl CompiledResourceFilter {
    /// Compiles `source`. The only way to build one, which is what keeps the
    /// two fields from ever disagreeing.
    pub(super) fn new(source: String) -> Self {
        let compiled = compile_filter(&source);
        Self { source, compiled }
    }

    /// The pattern the user applied, verbatim.
    pub(super) fn source(&self) -> &str {
        &self.source
    }

    /// The compiled filter to match rows against.
    ///
    /// An input that will not parse or compile narrows nothing
    /// ([`CompiledFilter::None`], every loaded row visible) rather than nothing
    /// matching: an empty table behind a bad regex is indistinguishable from a
    /// correct empty result, so showing too much alongside the inline
    /// `ResourcesState::filter_error` is the safe failure.
    pub(super) fn filter(&self) -> &CompiledFilter {
        /// The fallback for an input that does not compile. A `static` rather
        /// than a value built per call, so reading the cache never allocates
        /// and callers can compare what they got by identity.
        static UNFILTERED: CompiledFilter = CompiledFilter::None;

        self.compiled.as_ref().unwrap_or(&UNFILTERED)
    }

    /// The compile error of the applied pattern, when there is one.
    pub(super) fn error_text(&self) -> Option<String> {
        self.compiled
            .as_ref()
            .err()
            .map(|error| error.message.clone())
    }
}
