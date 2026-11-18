//! Compute Engine regions and zones, as their `list` commands report them.

/// A Compute Engine region. `status` is the API's own `UP`/`DOWN` string,
/// `None` when the listing did not carry one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputeRegion {
    pub name: String,
    pub status: Option<String>,
}

/// A Compute Engine zone.
///
/// `region` is what the API stated, not a parse of the zone name: the name
/// does contain it (`us-central1-a`), but a zone whose naming does not follow
/// that shape would be silently misfiled by splitting on the last dash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputeZone {
    pub name: String,
    pub region: Option<String>,
    pub status: Option<String>,
}
