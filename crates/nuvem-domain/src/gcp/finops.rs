//! Cost recommendations, normalised.
//!
//! The Recommender API gives a cost *projection over its own duration*, in a
//! sign-inverted currency amount, for recommendations that may be mutually
//! exclusive alternatives and may already have been dismissed by the
//! operator. Every one of those is a way to render a confidently wrong
//! number, so all four are handled here rather than at a render site.

use serde::{Deserialize, Serialize};

use crate::gcp::money::{Money, MoneyTotal};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationKind {
    Zonal,
    Regional,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecommenderId {
    IdleVm,
    MachineType,
    IdleDisk,
    UnusedAddress,
}

impl RecommenderId {
    pub const ALL: [Self; 4] = [
        Self::IdleVm,
        Self::MachineType,
        Self::IdleDisk,
        Self::UnusedAddress,
    ];

    /// The exact string `--recommender=` takes. Not derived from the variant
    /// name: these are an external vocabulary and a rename here would be a
    /// silently empty result rather than a compile error.
    pub fn id(self) -> &'static str {
        match self {
            Self::IdleVm => "google.compute.instance.IdleResourceRecommender",
            Self::MachineType => "google.compute.instance.MachineTypeRecommender",
            Self::IdleDisk => "google.compute.disk.IdleResourceRecommender",
            Self::UnusedAddress => "google.compute.address.IdleResourceRecommender",
        }
    }

    /// What `--location=` means for this recommender. Passing a zone to a
    /// regional recommender returns an empty list, which is indistinguishable
    /// from "nothing to save".
    pub fn location_kind(self) -> LocationKind {
        match self {
            Self::IdleVm | Self::MachineType | Self::IdleDisk => LocationKind::Zonal,
            Self::UnusedAddress => LocationKind::Regional,
        }
    }

    /// The card heading for this recommender — prose for an operator, as
    /// opposed to [`RecommenderId::id`], which is the API's vocabulary.
    pub fn label(self) -> &'static str {
        match self {
            Self::IdleVm => "Idle VMs",
            Self::MachineType => "Machine-type right-sizing",
            Self::IdleDisk => "Idle disks",
            Self::UnusedAddress => "Unused addresses",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecommendationState {
    Active,
    Claimed,
    Succeeded,
    Failed,
    Dismissed,
    Unrecognized(String),
}

impl RecommendationState {
    pub fn from_api(raw: Option<&str>) -> Self {
        match raw {
            Some("ACTIVE") => Self::Active,
            Some("CLAIMED") => Self::Claimed,
            Some("SUCCEEDED") => Self::Succeeded,
            Some("FAILED") => Self::Failed,
            Some("DISMISSED") => Self::Dismissed,
            Some(other) => Self::Unrecognized(other.to_string()),
            None => Self::Unrecognized(String::new()),
        }
    }

    /// Only ACTIVE is an available saving. CLAIMED means someone is already
    /// acting on it, SUCCEEDED/FAILED describe a past attempt, DISMISSED is
    /// the operator saying the resource is deliberate, and an unrecognised
    /// state is not assumed into a total.
    pub fn counts_toward_savings(&self) -> bool {
        matches!(self, Self::Active)
    }
}

/// One cost recommendation, already normalised: `monthly` is a positive
/// monthly saving, not the sign-inverted projection over the recommendation's
/// own duration that arrived.
///
/// `state` and `xor_group` are what a total must still exclude on — a
/// recommendation that does not count toward savings, and all but one of a set
/// of mutually exclusive alternatives sharing a group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostRecommendation {
    pub name: String,
    /// The API's own free-form English, shown verbatim so the claim stays
    /// attributable to Google rather than re-worded by this app.
    pub description: String,
    pub state: RecommendationState,
    pub priority: Option<String>,
    pub xor_group: Option<String>,
    /// Already normalised to a month and already sign-corrected to a positive
    /// saving. `None` when the projection had no usable duration.
    pub monthly: Option<Money>,
    pub last_refresh: Option<String>,
    pub target_resources: Vec<String>,
}

/// `"2592000s"` -> `2592000`. Truncates a fractional part; refuses anything
/// without the trailing `s`, which is a schema change rather than a zero.
pub fn parse_duration_seconds(raw: &str) -> Option<i128> {
    let digits = raw.strip_suffix('s')?;
    let whole = digits.split_once('.').map_or(digits, |(head, _)| head);
    whole.parse().ok()
}

/// Per-currency savings, counting each xor group once at its largest member.
///
/// The group key is the xor id *and* the currency: members priced in
/// different currencies are not comparable by amount, so ranking them would
/// pick a winner by a number that means two different things. Keyed this way,
/// a mixed group contributes one alternative per currency, which is the most
/// the app can say without an exchange rate.
pub fn savings_total(recommendations: &[CostRecommendation]) -> MoneyTotal {
    let mut total = MoneyTotal::default();
    let mut groups: Vec<((String, String), Money)> = Vec::new();

    for recommendation in recommendations {
        if !recommendation.state.counts_toward_savings() {
            continue;
        }
        let Some(money) = recommendation.monthly.clone() else {
            continue;
        };
        match &recommendation.xor_group {
            None => total.add(money),
            Some(group) => {
                let key = (group.clone(), money.currency.clone());
                match groups.iter_mut().find(|(id, _)| *id == key) {
                    Some((_, best)) if best.nanos >= money.nanos => {}
                    Some((_, best)) => *best = money,
                    None => groups.push((key, money)),
                }
            }
        }
    }

    for (_, best) in groups {
        total.add(best);
    }
    total
}

/// Segment counts, not a lookup table: a zone is `<region>-<letter>` and a
/// region is `<continent>-<direction><digit>`, so three segments against two
/// tells them apart without this file having to know every Google region.
const ZONE_SEGMENTS: usize = 3;
const REGION_SEGMENTS: usize = 2;

/// Whether a location string names a zone rather than a region.
///
/// `PersistentDisk::location` holds either — a regional disk carries its
/// region where a zonal one carries its zone — so the two arrive mixed in one
/// list, and querying one against the other's recommender returns an empty
/// array that reads as "nothing to save".
pub fn is_zone(location: &str) -> bool {
    location.split('-').count() == ZONE_SEGMENTS
}

/// The region a zone belongs to: `europe-west1-b` -> `europe-west1`.
/// `None` for anything that is not a zone.
pub fn region_of(zone: &str) -> Option<String> {
    if !is_zone(zone) {
        return None;
    }
    let (region, _) = zone.rsplit_once('-')?;
    Some(region.to_string())
}

/// The `--location` values worth querying for one recommender, given the
/// locations nuvem has already listed rows in.
///
/// Deriving the fanout from loaded rows rather than enumerating every Google
/// zone is design-spec open question 1, resolved to option (c): it is both
/// cheap and complete for resources nuvem has actually seen. What it is not is
/// complete for the *project*, which is why every card renders its coverage
/// rather than presenting the figure as the whole picture.
pub fn query_locations(recommender: RecommenderId, seen: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for location in seen {
        let candidate = match recommender.location_kind() {
            LocationKind::Zonal => is_zone(location).then(|| location.clone()),
            LocationKind::Regional => {
                if is_zone(location) {
                    region_of(location)
                } else {
                    (location.split('-').count() == REGION_SEGMENTS).then(|| location.clone())
                }
            }
            LocationKind::Global => Some("global".to_string()),
        };
        if let Some(candidate) = candidate
            && !out.contains(&candidate)
        {
            out.push(candidate);
        }
    }
    out
}
