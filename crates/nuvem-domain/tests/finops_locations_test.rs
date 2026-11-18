use nuvem_domain::gcp::finops::{RecommenderId, is_zone, query_locations, region_of};

/// A zone has three hyphen-separated segments and a region has two. The
/// discriminator matters because PersistentDisk::location holds either one:
/// a regional disk carries its region there, and treating that as a zone
/// would query `europe-west1` against a zonal recommender and get an empty
/// list back — indistinguishable from "nothing to save".
#[test]
fn a_zone_is_told_from_a_region_by_its_segment_count() {
    assert!(is_zone("europe-west1-b"));
    assert!(!is_zone("europe-west1"));
    assert!(!is_zone(""));
    assert!(!is_zone("global"));
}

#[test]
fn a_region_is_the_zone_without_its_trailing_letter() {
    assert_eq!(
        region_of("europe-west1-b"),
        Some("europe-west1".to_string())
    );
    assert_eq!(region_of("us-central1-a"), Some("us-central1".to_string()));
    assert_eq!(region_of("europe-west1"), None);
}

/// The zonal recommenders get the zones seen, deduplicated, and nothing else.
#[test]
fn a_zonal_recommender_queries_only_the_zones_that_were_seen() {
    let seen = vec![
        "europe-west1-b".to_string(),
        "europe-west1-b".to_string(),
        "us-central1-a".to_string(),
    ];
    assert_eq!(
        query_locations(RecommenderId::IdleVm, &seen),
        vec!["europe-west1-b".to_string(), "us-central1-a".to_string()]
    );
}

/// The address recommender is regional, so the zones collapse to their
/// regions — two zones in one region are one query, not two.
#[test]
fn a_regional_recommender_collapses_zones_to_their_regions() {
    let seen = vec![
        "europe-west1-b".to_string(),
        "europe-west1-c".to_string(),
        "us-central1-a".to_string(),
    ];
    assert_eq!(
        query_locations(RecommenderId::UnusedAddress, &seen),
        vec!["europe-west1".to_string(), "us-central1".to_string()]
    );
}

/// A region already in the seen set — a regional persistent disk — counts for
/// the regional recommender and is skipped by the zonal ones.
#[test]
fn a_region_in_the_seen_set_reaches_only_the_regional_recommender() {
    let seen = vec!["europe-west4".to_string()];
    assert!(query_locations(RecommenderId::IdleVm, &seen).is_empty());
    assert_eq!(
        query_locations(RecommenderId::UnusedAddress, &seen),
        vec!["europe-west4".to_string()]
    );
}

/// Nothing loaded means nothing to query. An empty query set is what makes
/// the card render "not probed" instead of a $0 total.
#[test]
fn no_seen_locations_produce_no_queries() {
    assert!(query_locations(RecommenderId::IdleVm, &[]).is_empty());
    assert!(query_locations(RecommenderId::UnusedAddress, &[]).is_empty());
}

/// Order is the seen order, deduplicated, so a refresh issues its subprocesses
/// in a stable sequence and the coverage line reads the same way twice.
#[test]
fn query_order_is_stable_and_deduplicated() {
    let seen = vec![
        "us-central1-a".to_string(),
        "europe-west1-b".to_string(),
        "us-central1-a".to_string(),
    ];
    assert_eq!(
        query_locations(RecommenderId::MachineType, &seen),
        vec!["us-central1-a".to_string(), "europe-west1-b".to_string()]
    );
}
