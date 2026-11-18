//! Tests for the `ResourceService` trait that unifies the generic resource
//! collections (Compute, GKE, Cloud SQL, Cloud Run, and the Cloud Storage
//! bucket list) behind a single interface. Written first (TDD RED) before
//! the trait/impls exist.

use nuvem::app::projection::table::SortDirection;
use nuvem::app::state::resources::{
    CloudRun, CloudRunSortField, CloudSql, CloudSqlInstanceSortField, Compute,
    ComputeInstanceSortField, Gke, GkeSortField, ResourceService, ResourcesState, Storage,
    StorageSortField,
};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::filter::{CompiledFilter, compile_filter};
use nuvem_domain::gcp::resource::{
    CloudRunService, CloudRunServiceStatus, CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus,
    ComputeInstance, EndpointVisibility, GkeCluster, GkeClusterStatus, ResourceServiceKind,
    ResourceStatus, StorageBucket,
};

fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".into()),
        region: None,
        zone: None,
    }
}

fn instance(name: &str) -> ComputeInstance {
    ComputeInstance {
        id: format!("{name}-id"),
        name: name.into(),
        zone: "europe-west1-b".into(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".into(),
        internal_ip: None,
        external_ip: None,
    }
}

fn cluster(name: &str) -> GkeCluster {
    GkeCluster {
        name: name.into(),
        location: "europe-west1".into(),
        status: GkeClusterStatus::Running,
        current_master_version: Some("1.30".into()),
        current_node_count: Some(3),
        endpoint_visibility: EndpointVisibility::Public,
    }
}

fn sql_instance(name: &str) -> CloudSqlInstance {
    CloudSqlInstance {
        name: name.into(),
        database_version: "POSTGRES_15".into(),
        engine: "PostgreSQL".into(),
        region: "europe-west1".into(),
        tier: "db-custom-1-3840".into(),
        availability: CloudSqlAvailability::Zonal,
        status: CloudSqlStatus::Runnable,
        connectivity: EndpointVisibility::Public,
    }
}

fn bucket(name: &str) -> StorageBucket {
    StorageBucket {
        name: name.into(),
        location: "europe-west1".into(),
        location_type: None,
        storage_class: "STANDARD".into(),
        versioning_enabled: false,
        uniform_bucket_level_access: true,
        public_access_prevention: None,
        time_created: None,
        updated: None,
    }
}

fn run_service(name: &str) -> CloudRunService {
    CloudRunService {
        name: name.into(),
        region: "europe-west1".into(),
        url: Some(format!("https://{name}.example.com")),
        latest_ready_revision: Some(format!("{name}-00001")),
        latest_created_revision: Some(format!("{name}-00001")),
        traffic_summary: "100% latest".into(),
        status: CloudRunServiceStatus::Ready,
        ready_message: None,
    }
}

/// Compiles a filter input the way the `visible_*` accessors do. Panics on an
/// input the test itself got wrong.
fn compiled(input: &str) -> CompiledFilter {
    compile_filter(input).expect("test filter input must compile")
}

/// Types `input` into the bucket list's filter input and applies it, so
/// `visible_buckets` reflects it.
fn apply_bucket_filter(state: &mut ResourcesState, input: &str) {
    let storage = state.storage_mut();
    storage.start_filter();
    for ch in input.chars() {
        storage.append_filter_char(ch);
    }
    storage.apply_filter();
}

#[test]
fn compute_kind_is_compute_instances() {
    assert_eq!(Compute::KIND, ResourceServiceKind::ComputeInstances);
}

#[test]
fn gke_kind_is_gke_clusters() {
    assert_eq!(Gke::KIND, ResourceServiceKind::GkeClusters);
}

#[test]
fn cloud_sql_kind_is_cloud_sql() {
    assert_eq!(CloudSql::KIND, ResourceServiceKind::CloudSql);
}

#[test]
fn cloud_run_kind_is_cloud_run_services() {
    assert_eq!(CloudRun::KIND, ResourceServiceKind::CloudRunServices);
}

#[test]
fn storage_kind_is_cloud_storage() {
    assert_eq!(Storage::KIND, ResourceServiceKind::CloudStorage);
}

#[test]
fn compute_collection_rows_match_public_accessor() {
    let mut state = ResourcesState::default();
    state.set_instances(ctx(), vec![instance("a"), instance("b")]);
    let via_trait: Vec<&ComputeInstance> = Compute::collection(&state).rows().iter().collect();
    let via_public: Vec<&ComputeInstance> = state.instances().iter().collect();
    assert_eq!(via_trait, via_public);
}

#[test]
fn gke_collection_rows_match_public_accessor() {
    let mut state = ResourcesState::default();
    state.set_gke_clusters(ctx(), vec![cluster("a"), cluster("b")]);
    let via_trait: Vec<&GkeCluster> = Gke::collection(&state).rows().iter().collect();
    let via_public: Vec<&GkeCluster> = state.gke_clusters().iter().collect();
    assert_eq!(via_trait, via_public);
}

#[test]
fn cloud_sql_collection_rows_match_public_accessor() {
    let mut state = ResourcesState::default();
    state.set_cloud_sql_instances(ctx(), vec![sql_instance("a"), sql_instance("b")]);
    let via_trait: Vec<&CloudSqlInstance> = CloudSql::collection(&state).rows().iter().collect();
    let via_public: Vec<&CloudSqlInstance> = state.cloud_sql_instances().iter().collect();
    assert_eq!(via_trait, via_public);
}

#[test]
fn cloud_run_collection_rows_match_public_accessor() {
    let mut state = ResourcesState::default();
    state.set_cloud_run_services(ctx(), vec![run_service("a"), run_service("b")]);
    let via_trait: Vec<&CloudRunService> = CloudRun::collection(&state).rows().iter().collect();
    let via_public: Vec<&CloudRunService> = state.cloud_run_services().iter().collect();
    assert_eq!(via_trait, via_public);
}

#[test]
fn storage_collection_rows_match_public_accessor() {
    let mut state = ResourcesState::default();
    state
        .storage_mut()
        .set_buckets(ctx(), vec![bucket("a"), bucket("b")]);
    let via_trait: Vec<&StorageBucket> = Storage::collection(&state).rows().iter().collect();
    let via_public: Vec<&StorageBucket> = state.storage().buckets().iter().collect();
    assert_eq!(via_trait, via_public);
}

#[test]
fn compute_matches_filter_agrees_with_observable_filtering() {
    let mut state = ResourcesState::default();
    state.set_instances(ctx(), vec![instance("web-server"), instance("db-server")]);
    let web = &state.instances()[state
        .instances()
        .iter()
        .position(|i| i.name == "web-server")
        .unwrap()];
    assert!(Compute::matches_filter(web, &compiled("web")));
    assert!(!Compute::matches_filter(
        web,
        &compiled("nonexistent-needle")
    ));
}

// --- the four filter modes at the trait boundary ------------------------
//
// The grammar itself is covered by `tests/filter_grammar_test.rs`; these pin
// that `matches_filter` evaluates a *compiled* filter over the row's
// searchable fields rather than a plain substring.

#[test]
fn compute_matches_filter_regex_narrows_partially_and_case_insensitively() {
    let row = instance("web-server");
    assert!(Compute::matches_filter(&row, &compiled("WEB")));
    assert!(Compute::matches_filter(&row, &compiled("w.b-ser")));
    assert!(
        !Compute::matches_filter(&row, &compiled("^server")),
        "an anchored pattern must not match mid-field"
    );
}

#[test]
fn compute_matches_filter_fuzzy_keeps_a_row_a_substring_filter_would_drop() {
    let row = instance("web-server");
    assert!(
        Compute::matches_filter(&row, &compiled("-f wsrv")),
        "fuzzy mode matches a non-adjacent subsequence"
    );
    assert!(
        !Compute::matches_filter(&row, &compiled("wsrv")),
        "the same needle must not match as a regex/substring"
    );
}

#[test]
fn compute_matches_filter_inverse_is_the_complement_of_regex() {
    for row in [instance("web-server"), instance("db-server")] {
        assert_eq!(
            Compute::matches_filter(&row, &compiled("!web")),
            !Compute::matches_filter(&row, &compiled("web")),
            "inverse must keep exactly the rows regex mode drops"
        );
    }
}

#[test]
fn compute_matches_filter_label_keeps_every_row() {
    // Label mode is server-authoritative: list rows carry no labels, so the
    // client-side predicate cannot drop a row the server returned.
    assert!(Compute::matches_filter(
        &instance("web-server"),
        &compiled("-l env=prod")
    ));
}

#[test]
fn gke_matches_filter_agrees_with_observable_filtering() {
    let mut state = ResourcesState::default();
    state.set_gke_clusters(ctx(), vec![cluster("prod-cluster")]);
    let row = &state.gke_clusters()[0];
    assert!(Gke::matches_filter(row, &compiled("prod")));
    assert!(!Gke::matches_filter(row, &compiled("nonexistent-needle")));
}

#[test]
fn cloud_sql_matches_filter_agrees_with_observable_filtering() {
    let mut state = ResourcesState::default();
    state.set_cloud_sql_instances(ctx(), vec![sql_instance("billing-db")]);
    let row = &state.cloud_sql_instances()[0];
    assert!(CloudSql::matches_filter(row, &compiled("billing")));
    assert!(!CloudSql::matches_filter(
        row,
        &compiled("nonexistent-needle")
    ));
}

#[test]
fn cloud_run_matches_filter_agrees_with_observable_filtering() {
    let mut state = ResourcesState::default();
    state.set_cloud_run_services(ctx(), vec![run_service("checkout-api")]);
    let row = &state.cloud_run_services()[0];
    assert!(CloudRun::matches_filter(row, &compiled("checkout")));
    assert!(!CloudRun::matches_filter(
        row,
        &compiled("nonexistent-needle")
    ));
}

/// Unlike the other four services, `Storage::matches_filter` does *not* agree
/// with Cloud Storage's observable filtering: the trait impl is grammar-aware
/// (it must be, the signature takes a `CompiledFilter`) while
/// `StorageState::visible_buckets` deliberately stays a literal lowercased
/// substring match, because `is_safe_remote_prefix` admits `-f api` and `!api`
/// verbatim as remote object prefixes. Nothing in `src/` routes the bucket
/// list through the trait method today; the second half of this test pins the
/// divergence so a future generic caller finds it here rather than in a
/// dropped row.
#[test]
fn storage_matches_filter_searches_the_bucket_fields_but_diverges_from_observable_filtering() {
    let mut state = ResourcesState::default();
    state
        .storage_mut()
        .set_buckets(ctx(), vec![bucket("assets-bucket")]);
    let row = &state.storage().buckets()[0];
    // Name match. `CompiledFilter` owns case-insensitivity, so callers no
    // longer lowercase the needle first.
    assert!(Storage::matches_filter(row, &compiled("assets")));
    // Location match (bucket() fixture sets location to "europe-west1").
    assert!(Storage::matches_filter(row, &compiled("europe-west1")));
    // Storage-class match (bucket() fixture sets storage_class to "STANDARD").
    assert!(Storage::matches_filter(row, &compiled("standard")));
    assert!(!Storage::matches_filter(
        row,
        &compiled("nonexistent-needle")
    ));

    // The split, on one input: `-f asts` is a fuzzy subsequence of
    // "assets-bucket" but not a substring of any of its fields.
    assert!(
        Storage::matches_filter(row, &compiled("-f asts")),
        "the trait impl reads the prefix grammar"
    );
    apply_bucket_filter(&mut state, "-f asts");
    assert!(
        state.storage().visible_buckets().is_empty(),
        "the bucket list reads `-f asts` as a literal substring and matches nothing"
    );
    assert_eq!(state.storage().visible_bucket_count(), 0);
}

// --- `compare` coverage -----------------------------------------------
//
// The default sort field/direction for every collection is Name/Ascending
// (see `ResourcesState::default`), so the public `visible_*` accessors
// already reflect the alphabetical order independent of calling `compare`
// directly. Each test below: (1) inserts rows out of alphabetical order,
// (2) checks the publicly observable sorted view against a hand-derived
// alphabetical expectation, (3) checks `compare` directly against
// hand-derived `Ordering`s for both directions, and (4) cross-checks that
// sorting the raw rows with `compare` reproduces the same order the public
// accessor exposes.

#[test]
fn compute_compare_by_name_matches_visible_instances_sort_order() {
    let mut state = ResourcesState::default();
    state.set_instances(
        ctx(),
        vec![instance("charlie"), instance("alpha"), instance("bravo")],
    );

    let visible: Vec<&str> = state
        .visible_instances()
        .iter()
        .map(|i| i.name.as_str())
        .collect();
    assert_eq!(visible, vec!["alpha", "bravo", "charlie"]);

    let alpha = &state.instances()[state
        .instances()
        .iter()
        .position(|i| i.name == "alpha")
        .unwrap()];
    let bravo = &state.instances()[state
        .instances()
        .iter()
        .position(|i| i.name == "bravo")
        .unwrap()];

    assert_eq!(
        Compute::compare(
            alpha,
            bravo,
            ComputeInstanceSortField::Name,
            SortDirection::Ascending
        ),
        std::cmp::Ordering::Less
    );
    assert_eq!(
        Compute::compare(
            alpha,
            bravo,
            ComputeInstanceSortField::Name,
            SortDirection::Descending
        ),
        std::cmp::Ordering::Greater
    );

    let mut rows: Vec<_> = state.instances().iter().collect();
    rows.sort_by(|a, b| {
        Compute::compare(
            a,
            b,
            ComputeInstanceSortField::Name,
            SortDirection::Ascending,
        )
    });
    let sorted_names: Vec<&str> = rows.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(sorted_names, visible);
}

#[test]
fn gke_compare_by_name_matches_visible_clusters_sort_order() {
    let mut state = ResourcesState::default();
    state.set_gke_clusters(
        ctx(),
        vec![cluster("charlie"), cluster("alpha"), cluster("bravo")],
    );

    let visible: Vec<&str> = state
        .visible_gke_clusters()
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(visible, vec!["alpha", "bravo", "charlie"]);

    let alpha = &state.gke_clusters()[state
        .gke_clusters()
        .iter()
        .position(|c| c.name == "alpha")
        .unwrap()];
    let bravo = &state.gke_clusters()[state
        .gke_clusters()
        .iter()
        .position(|c| c.name == "bravo")
        .unwrap()];

    assert_eq!(
        Gke::compare(alpha, bravo, GkeSortField::Name, SortDirection::Ascending),
        std::cmp::Ordering::Less
    );
    assert_eq!(
        Gke::compare(alpha, bravo, GkeSortField::Name, SortDirection::Descending),
        std::cmp::Ordering::Greater
    );

    let mut rows: Vec<_> = state.gke_clusters().iter().collect();
    rows.sort_by(|a, b| Gke::compare(a, b, GkeSortField::Name, SortDirection::Ascending));
    let sorted_names: Vec<&str> = rows.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(sorted_names, visible);
}

#[test]
fn cloud_sql_compare_by_name_matches_visible_instances_sort_order() {
    let mut state = ResourcesState::default();
    state.set_cloud_sql_instances(
        ctx(),
        vec![
            sql_instance("charlie"),
            sql_instance("alpha"),
            sql_instance("bravo"),
        ],
    );

    let visible: Vec<&str> = state
        .visible_cloud_sql_instances()
        .iter()
        .map(|i| i.name.as_str())
        .collect();
    assert_eq!(visible, vec!["alpha", "bravo", "charlie"]);

    let alpha = &state.cloud_sql_instances()[state
        .cloud_sql_instances()
        .iter()
        .position(|i| i.name == "alpha")
        .unwrap()];
    let bravo = &state.cloud_sql_instances()[state
        .cloud_sql_instances()
        .iter()
        .position(|i| i.name == "bravo")
        .unwrap()];

    assert_eq!(
        CloudSql::compare(
            alpha,
            bravo,
            CloudSqlInstanceSortField::Name,
            SortDirection::Ascending
        ),
        std::cmp::Ordering::Less
    );
    assert_eq!(
        CloudSql::compare(
            alpha,
            bravo,
            CloudSqlInstanceSortField::Name,
            SortDirection::Descending
        ),
        std::cmp::Ordering::Greater
    );

    let mut rows: Vec<_> = state.cloud_sql_instances().iter().collect();
    rows.sort_by(|a, b| {
        CloudSql::compare(
            a,
            b,
            CloudSqlInstanceSortField::Name,
            SortDirection::Ascending,
        )
    });
    let sorted_names: Vec<&str> = rows.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(sorted_names, visible);
}

#[test]
fn cloud_run_compare_by_name_matches_visible_services_sort_order() {
    let mut state = ResourcesState::default();
    state.set_cloud_run_services(
        ctx(),
        vec![
            run_service("charlie"),
            run_service("alpha"),
            run_service("bravo"),
        ],
    );

    let visible: Vec<&str> = state
        .visible_cloud_run_services()
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(visible, vec!["alpha", "bravo", "charlie"]);

    let alpha = &state.cloud_run_services()[state
        .cloud_run_services()
        .iter()
        .position(|s| s.name == "alpha")
        .unwrap()];
    let bravo = &state.cloud_run_services()[state
        .cloud_run_services()
        .iter()
        .position(|s| s.name == "bravo")
        .unwrap()];

    assert_eq!(
        CloudRun::compare(
            alpha,
            bravo,
            CloudRunSortField::Name,
            SortDirection::Ascending
        ),
        std::cmp::Ordering::Less
    );
    assert_eq!(
        CloudRun::compare(
            alpha,
            bravo,
            CloudRunSortField::Name,
            SortDirection::Descending
        ),
        std::cmp::Ordering::Greater
    );

    let mut rows: Vec<_> = state.cloud_run_services().iter().collect();
    rows.sort_by(|a, b| CloudRun::compare(a, b, CloudRunSortField::Name, SortDirection::Ascending));
    let sorted_names: Vec<&str> = rows.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(sorted_names, visible);
}

#[test]
fn storage_compare_by_name_matches_visible_buckets_sort_order() {
    let mut state = ResourcesState::default();
    state.storage_mut().set_buckets(
        ctx(),
        vec![bucket("charlie"), bucket("alpha"), bucket("bravo")],
    );

    let visible: Vec<&str> = state
        .storage()
        .visible_buckets()
        .iter()
        .map(|b| b.name.as_str())
        .collect();
    assert_eq!(visible, vec!["alpha", "bravo", "charlie"]);

    let alpha = &state.storage().buckets()[state
        .storage()
        .buckets()
        .iter()
        .position(|b| b.name == "alpha")
        .unwrap()];
    let bravo = &state.storage().buckets()[state
        .storage()
        .buckets()
        .iter()
        .position(|b| b.name == "bravo")
        .unwrap()];

    assert_eq!(
        Storage::compare(
            alpha,
            bravo,
            StorageSortField::Name,
            SortDirection::Ascending
        ),
        std::cmp::Ordering::Less
    );
    assert_eq!(
        Storage::compare(
            alpha,
            bravo,
            StorageSortField::Name,
            SortDirection::Descending
        ),
        std::cmp::Ordering::Greater
    );

    let mut rows: Vec<_> = state.storage().buckets().iter().collect();
    rows.sort_by(|a, b| Storage::compare(a, b, StorageSortField::Name, SortDirection::Ascending));
    let sorted_names: Vec<&str> = rows.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(sorted_names, visible);
}

#[test]
fn storage_compare_by_location_ties_broken_by_name() {
    // Two buckets sharing the same location: `compare` on the Location field
    // must fall back to the name as a tiebreaker, matching the pattern used
    // by every other service's `compare` (see `compare_buckets`).
    let mut same_location_a = bucket("zeta");
    same_location_a.location = "us-central1".into();
    let mut same_location_b = bucket("alpha");
    same_location_b.location = "us-central1".into();

    assert_eq!(
        Storage::compare(
            &same_location_a,
            &same_location_b,
            StorageSortField::Location,
            SortDirection::Ascending,
        ),
        std::cmp::Ordering::Greater,
        "equal locations must tie-break on name (alpha < zeta)"
    );
    assert_eq!(
        Storage::compare(
            &same_location_a,
            &same_location_b,
            StorageSortField::Location,
            SortDirection::Descending,
        ),
        std::cmp::Ordering::Less
    );
}

// --- `collection_mut` coverage -----------------------------------------

#[test]
fn compute_collection_mut_refers_to_same_collection_as_collection() {
    let mut state = ResourcesState::default();
    state.set_instances(ctx(), vec![instance("a"), instance("b"), instance("c")]);

    let via_collection: Vec<ComputeInstance> = Compute::collection(&state).rows().to_vec();
    let via_collection_mut: Vec<ComputeInstance> =
        Compute::collection_mut(&mut state).rows().to_vec();
    assert_eq!(via_collection, via_collection_mut);
    assert_eq!(via_collection_mut.len(), 3);
}

#[test]
fn gke_collection_mut_refers_to_same_collection_as_collection() {
    let mut state = ResourcesState::default();
    state.set_gke_clusters(ctx(), vec![cluster("a"), cluster("b"), cluster("c")]);

    let via_collection: Vec<GkeCluster> = Gke::collection(&state).rows().to_vec();
    let via_collection_mut: Vec<GkeCluster> = Gke::collection_mut(&mut state).rows().to_vec();
    assert_eq!(via_collection, via_collection_mut);
    assert_eq!(via_collection_mut.len(), 3);
}

#[test]
fn cloud_sql_collection_mut_refers_to_same_collection_as_collection() {
    let mut state = ResourcesState::default();
    state.set_cloud_sql_instances(
        ctx(),
        vec![sql_instance("a"), sql_instance("b"), sql_instance("c")],
    );

    let via_collection: Vec<CloudSqlInstance> = CloudSql::collection(&state).rows().to_vec();
    let via_collection_mut: Vec<CloudSqlInstance> =
        CloudSql::collection_mut(&mut state).rows().to_vec();
    assert_eq!(via_collection, via_collection_mut);
    assert_eq!(via_collection_mut.len(), 3);
}

#[test]
fn cloud_run_collection_mut_refers_to_same_collection_as_collection() {
    let mut state = ResourcesState::default();
    state.set_cloud_run_services(
        ctx(),
        vec![run_service("a"), run_service("b"), run_service("c")],
    );

    let via_collection: Vec<CloudRunService> = CloudRun::collection(&state).rows().to_vec();
    let via_collection_mut: Vec<CloudRunService> =
        CloudRun::collection_mut(&mut state).rows().to_vec();
    assert_eq!(via_collection, via_collection_mut);
    assert_eq!(via_collection_mut.len(), 3);
}

#[test]
fn storage_collection_mut_refers_to_same_collection_as_collection() {
    let mut state = ResourcesState::default();
    state
        .storage_mut()
        .set_buckets(ctx(), vec![bucket("a"), bucket("b"), bucket("c")]);

    let via_collection: Vec<StorageBucket> = Storage::collection(&state).rows().to_vec();
    let via_collection_mut: Vec<StorageBucket> =
        Storage::collection_mut(&mut state).rows().to_vec();
    assert_eq!(via_collection, via_collection_mut);
    assert_eq!(via_collection_mut.len(), 3);
}
