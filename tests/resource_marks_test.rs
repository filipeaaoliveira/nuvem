use nuvem::app::state::resources::{
    CloudRun, CloudSql, Compute, Gke, ResourceRowId, ResourceService, ResourcesState, Storage,
};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};

fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".into()),
        region: None,
        zone: None,
    }
}

fn inst(name: &str, status: ResourceStatus) -> ComputeInstance {
    inst_in(name, "europe-west1-b", status)
}

fn inst_in(name: &str, zone: &str, status: ResourceStatus) -> ComputeInstance {
    ComputeInstance {
        id: format!("{name}-{zone}-id"),
        name: name.into(),
        zone: zone.into(),
        status,
        machine_type: "e2-medium".into(),
        internal_ip: None,
        external_ip: None,
    }
}

fn loaded(names: &[(&str, ResourceStatus)]) -> ResourcesState {
    let mut state = ResourcesState::default();
    let rows = names.iter().map(|(n, s)| inst(n, s.clone())).collect();
    state.set_instances(ctx(), rows);
    state
}

#[test]
fn space_toggles_mark_on_selected_and_counts() {
    let mut state = loaded(&[
        ("a", ResourceStatus::Running),
        ("b", ResourceStatus::Running),
    ]);
    assert_eq!(state.marked_count(), 0);
    state.toggle_mark(); // selects visible[0] = "a" (sorted by name)
    assert!(state.is_instance_marked("a", "europe-west1-b"));
    assert_eq!(state.marked_count(), 1);
    state.toggle_mark();
    assert_eq!(state.marked_count(), 0);
}

#[test]
fn marks_survive_refresh_but_prune_deleted_rows() {
    let mut state = loaded(&[
        ("a", ResourceStatus::Running),
        ("b", ResourceStatus::Running),
    ]);
    state.toggle_mark(); // marks "a"
    state.select_next();
    state.toggle_mark(); // marks "b"
    assert_eq!(state.marked_count(), 2);
    // "b" disappears on refresh; "a" remains.
    state.set_instances(ctx(), vec![inst("a", ResourceStatus::Running)]);
    assert_eq!(state.marked_count(), 1);
    assert!(state.is_instance_marked("a", "europe-west1-b"));
    assert!(!state.is_instance_marked("b", "europe-west1-b"));
}

#[test]
fn hidden_marks_counted_but_not_visible_marked() {
    let mut state = loaded(&[
        ("api", ResourceStatus::Running),
        ("web", ResourceStatus::Running),
    ]);
    state.toggle_mark(); // marks "api"
    state.select_next();
    state.toggle_mark(); // marks "web"
    state.start_filter();
    for c in "web".chars() {
        state.append_filter_char(c);
    }
    state.apply_filter(); // only "web" visible; "api" hidden mark
    assert_eq!(state.marked_count(), 2);
    assert_eq!(state.visible_marked_count(), 1);
}

#[test]
fn range_mark_marks_inclusive_span_in_visible_order() {
    let mut state = loaded(&[
        ("a", ResourceStatus::Running),
        ("b", ResourceStatus::Running),
        ("c", ResourceStatus::Running),
    ]);
    state.toggle_mark(); // anchor + mark "a" (index 0)
    state.select_next();
    state.select_next(); // selection at "c" (index 2)
    state.range_mark(); // marks a,b,c
    assert_eq!(state.marked_count(), 3);
}

#[test]
fn clear_marks_and_unmark_visible() {
    let mut state = loaded(&[
        ("api", ResourceStatus::Running),
        ("web", ResourceStatus::Running),
    ]);
    state.toggle_mark();
    state.select_next();
    state.toggle_mark();
    state.start_filter();
    for c in "web".chars() {
        state.append_filter_char(c);
    }
    state.apply_filter(); // "api" hidden mark, "web" visible mark
    state.unmark_visible();
    assert_eq!(state.marked_count(), 1); // only hidden "api" remains
    state.clear_marks();
    assert_eq!(state.marked_count(), 0);
}

#[test]
fn marks_cleared_on_context_change() {
    let mut state = loaded(&[("a", ResourceStatus::Running)]);
    state.toggle_mark();
    assert_eq!(state.marked_count(), 1);
    state.mark_all_stale_and_clear();
    assert_eq!(state.marked_count(), 0);
}

#[test]
fn compute_row_id_is_name_and_zone() {
    let instance = inst_in("web", "us-central1-a", ResourceStatus::Running);
    assert_eq!(
        Compute::row_id(&instance),
        ResourceRowId {
            name: "web".to_string(),
            location: "us-central1-a".to_string(),
        }
    );
}

#[test]
fn row_id_is_total_across_services() {
    use nuvem_domain::gcp::resource::{
        CloudRunService, CloudRunServiceStatus, CloudSqlAvailability, CloudSqlInstance,
        CloudSqlStatus, EndpointVisibility, GkeCluster, GkeClusterStatus, StorageBucket,
    };

    let sql = CloudSqlInstance {
        name: "orders".into(),
        database_version: "POSTGRES_15".into(),
        engine: "PostgreSQL".into(),
        region: "europe-west1".into(),
        tier: "db-custom-1-3840".into(),
        availability: CloudSqlAvailability::Zonal,
        status: CloudSqlStatus::Runnable,
        connectivity: EndpointVisibility::Private,
    };
    assert_eq!(
        CloudSql::row_id(&sql),
        ResourceRowId {
            name: "orders".to_string(),
            location: "europe-west1".to_string(),
        }
    );

    let cluster = GkeCluster {
        name: "prod".into(),
        location: "europe-west1".into(),
        status: GkeClusterStatus::Running,
        current_master_version: None,
        current_node_count: None,
        endpoint_visibility: EndpointVisibility::Private,
    };
    assert_eq!(
        Gke::row_id(&cluster),
        ResourceRowId {
            name: "prod".to_string(),
            location: "europe-west1".to_string(),
        }
    );

    let service = CloudRunService {
        name: "api".into(),
        region: "europe-west1".into(),
        url: None,
        latest_ready_revision: None,
        latest_created_revision: None,
        traffic_summary: String::new(),
        status: CloudRunServiceStatus::Ready,
        ready_message: None,
    };
    assert_eq!(
        CloudRun::row_id(&service),
        ResourceRowId {
            name: "api".to_string(),
            location: "europe-west1".to_string(),
        }
    );

    let bucket = StorageBucket {
        name: "assets".into(),
        location: "EU".into(),
        location_type: None,
        storage_class: "STANDARD".into(),
        versioning_enabled: false,
        uniform_bucket_level_access: true,
        public_access_prevention: None,
        time_created: None,
        updated: None,
    };
    assert_eq!(
        Storage::row_id(&bucket),
        ResourceRowId {
            name: "assets".to_string(),
            location: "EU".to_string(),
        }
    );
}

#[test]
fn same_name_in_two_zones_is_marked_independently() {
    let mut state = ResourcesState::default();
    state.set_instances(
        ctx(),
        vec![
            inst_in("web", "us-central1-a", ResourceStatus::Running),
            inst_in("web", "us-central1-b", ResourceStatus::Running),
        ],
    );
    state.toggle_mark(); // marks the first visible "web" row
    assert_eq!(state.marked_count(), 1);
    assert!(state.is_instance_marked("web", "us-central1-a"));
    assert!(!state.is_instance_marked("web", "us-central1-b"));

    state.select_next();
    state.toggle_mark();
    assert_eq!(state.marked_count(), 2);
    assert!(state.is_instance_marked("web", "us-central1-b"));

    let marked = state.marked_instances();
    assert_eq!(
        marked
            .iter()
            .map(|instance| (instance.name.clone(), instance.zone.clone()))
            .collect::<Vec<_>>(),
        vec![
            ("web".to_string(), "us-central1-a".to_string()),
            ("web".to_string(), "us-central1-b".to_string()),
        ]
    );
}

#[test]
fn mark_keys_are_inert_when_gke_is_active() {
    let mut state = loaded(&[("a", ResourceStatus::Running)]);
    state.select_service(nuvem_domain::gcp::resource::ResourceServiceKind::GkeClusters);
    state.toggle_mark();
    state.range_mark();
    assert_eq!(state.marked_count(), 0);
    // switching back keeps compute marks usable
    state.select_service(nuvem_domain::gcp::resource::ResourceServiceKind::ComputeInstances);
    state.toggle_mark();
    assert_eq!(state.marked_count(), 1);
}

fn sql(name: &str, region: &str) -> nuvem_domain::gcp::resource::CloudSqlInstance {
    use nuvem_domain::gcp::resource::{
        CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, EndpointVisibility,
    };
    CloudSqlInstance {
        name: name.into(),
        database_version: "POSTGRES_15".into(),
        engine: "PostgreSQL".into(),
        region: region.into(),
        tier: "db-custom-1-3840".into(),
        availability: CloudSqlAvailability::Zonal,
        status: CloudSqlStatus::Runnable,
        connectivity: EndpointVisibility::Private,
    }
}

fn cloud_sql_loaded(names: &[&str]) -> ResourcesState {
    let mut state = ResourcesState::default();
    state.select_service(nuvem_domain::gcp::resource::ResourceServiceKind::CloudSql);
    state.set_cloud_sql_instances(
        ctx(),
        names.iter().map(|n| sql(n, "europe-west1")).collect(),
    );
    state
}

#[test]
fn cloud_sql_rows_are_markable_and_counted() {
    let mut state = cloud_sql_loaded(&["archive", "orders"]);
    assert_eq!(state.marked_count(), 0);
    state.toggle_mark(); // visible[0] = "archive"
    assert!(state.is_cloud_sql_instance_marked("archive", "europe-west1"));
    assert_eq!(state.marked_count(), 1);
    assert_eq!(state.visible_marked_count(), 1);
    state.select_next();
    state.range_mark(); // anchor "archive" .. selection "orders"
    assert_eq!(state.marked_count(), 2);
    state.unmark_visible();
    assert_eq!(state.marked_count(), 0);
    state.toggle_mark();
    state.clear_marks();
    assert_eq!(state.marked_count(), 0);
}

#[test]
fn cloud_sql_marks_are_pruned_on_refresh_and_cleared_on_context_change() {
    let mut state = cloud_sql_loaded(&["archive", "orders"]);
    state.toggle_mark();
    state.select_next();
    state.toggle_mark();
    assert_eq!(state.marked_count(), 2);

    // "orders" disappears on refresh; "archive" keeps its mark.
    state.set_cloud_sql_instances(ctx(), vec![sql("archive", "europe-west1")]);
    assert_eq!(state.marked_count(), 1);
    assert!(state.is_cloud_sql_instance_marked("archive", "europe-west1"));
    assert!(!state.is_cloud_sql_instance_marked("orders", "europe-west1"));

    state.mark_all_stale_and_clear();
    assert_eq!(state.marked_count(), 0);
}

/// Marks are per-service: switching the active service must not disturb the
/// marks the other service already holds.
#[test]
fn switching_services_leaves_the_other_services_marks_intact() {
    use nuvem_domain::gcp::resource::ResourceServiceKind;

    let mut state = loaded(&[("a", ResourceStatus::Running)]);
    state.toggle_mark(); // one compute mark
    assert_eq!(state.marked_count(), 1);

    state.select_service(ResourceServiceKind::CloudSql);
    state.set_cloud_sql_instances(ctx(), vec![sql("orders", "europe-west1")]);
    assert_eq!(state.marked_count(), 0, "Cloud SQL starts unmarked");
    state.toggle_mark();
    assert_eq!(state.marked_count(), 1);
    assert!(state.is_cloud_sql_instance_marked("orders", "europe-west1"));

    // Back to compute: its mark survived the round trip untouched.
    state.select_service(ResourceServiceKind::ComputeInstances);
    assert_eq!(state.marked_count(), 1);
    assert!(state.is_instance_marked("a", "europe-west1-b"));

    // And the Cloud SQL mark is still there behind it.
    state.select_service(ResourceServiceKind::CloudSql);
    assert_eq!(state.marked_count(), 1);
    assert!(state.is_cloud_sql_instance_marked("orders", "europe-west1"));
}

/// Clearing the Cloud SQL collection (its stale/clear teardown) drops its marks
/// without touching Compute's.
#[test]
fn cloud_sql_stale_and_clear_drops_only_cloud_sql_marks() {
    use nuvem_domain::gcp::resource::ResourceServiceKind;

    let mut state = loaded(&[("a", ResourceStatus::Running)]);
    state.toggle_mark();
    state.select_service(ResourceServiceKind::CloudSql);
    state.set_cloud_sql_instances(ctx(), vec![sql("orders", "europe-west1")]);
    state.toggle_mark();
    assert_eq!(state.marked_count(), 1);

    state.mark_active_stale_and_clear(ResourceServiceKind::CloudSql);

    assert_eq!(state.marked_count(), 0);
    state.select_service(ResourceServiceKind::ComputeInstances);
    assert_eq!(state.marked_count(), 1, "compute marks are untouched");
}

/// Cloud Storage, GKE and Cloud Run have no batch actions, so their mark keys
/// stay inert: a mark there would dangle with nothing to apply it to.
#[test]
fn mark_keys_stay_inert_for_services_without_batch_actions() {
    use nuvem_domain::gcp::resource::ResourceServiceKind;

    for service in [
        ResourceServiceKind::GkeClusters,
        ResourceServiceKind::CloudRunServices,
        ResourceServiceKind::CloudStorage,
    ] {
        let mut state = loaded(&[("a", ResourceStatus::Running)]);
        state.select_service(service);
        state.toggle_mark();
        state.range_mark();
        assert_eq!(state.marked_count(), 0, "{service:?} must not take marks");
        assert_eq!(state.visible_marked_count(), 0);
    }
}
