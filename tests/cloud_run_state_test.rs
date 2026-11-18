use nuvem::app::state::resources::{
    CloudRunDetailTab, CloudRunRevisionTab, CloudRunSortField, ResourcesState,
};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{
    CloudRunCondition, CloudRunRevision, CloudRunService, CloudRunServiceDetail,
    CloudRunServiceStatus, CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus,
    EndpointVisibility, ResourceServiceKind,
};

fn context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("prod"),
        account: Some("dev@example.com".to_string()),
        project: Some("my-project".to_string()),
        region: None,
        zone: None,
    }
}

fn service(name: &str, region: &str) -> CloudRunService {
    CloudRunService {
        name: name.to_string(),
        region: region.to_string(),
        url: Some(format!("https://{name}.run.app")),
        latest_ready_revision: Some(format!("{name}-00001")),
        latest_created_revision: Some(format!("{name}-00001")),
        traffic_summary: "100% -> latest".to_string(),
        status: CloudRunServiceStatus::Ready,
        ready_message: None,
    }
}

fn revision(name: &str) -> CloudRunRevision {
    CloudRunRevision {
        name: name.to_string(),
        ready: CloudRunServiceStatus::Ready,
        created: None,
        image: Some("gcr.io/x/y:latest".to_string()),
        active_percent: Some(100),
    }
}

fn detail_with_revisions(
    name: &str,
    region: &str,
    revisions: Vec<CloudRunRevision>,
) -> CloudRunServiceDetail {
    CloudRunServiceDetail {
        name: name.to_string(),
        region: region.to_string(),
        url: Some(format!("https://{name}.run.app")),
        status: CloudRunServiceStatus::Ready,
        latest_ready_revision: Some(format!("{name}-00001")),
        latest_created_revision: Some(format!("{name}-00001")),
        traffic: Vec::new(),
        revisions,
        conditions: vec![CloudRunCondition {
            type_: "Ready".to_string(),
            status: "True".to_string(),
            reason: None,
            message: None,
            last_transition_time: None,
        }],
        image: None,
        cpu: None,
        memory: None,
        concurrency: None,
        service_account: None,
        ingress: None,
        min_instances: None,
        max_instances: None,
        labels: Vec::new(),
        raw_json: "{}".to_string(),
    }
}

fn cloud_sql_instance(name: &str, region: &str) -> CloudSqlInstance {
    CloudSqlInstance {
        name: name.to_string(),
        database_version: "MYSQL_8_0".to_string(),
        engine: "MySQL".to_string(),
        region: region.to_string(),
        tier: "db-n1-standard-4".to_string(),
        availability: CloudSqlAvailability::Regional,
        status: CloudSqlStatus::Runnable,
        connectivity: EndpointVisibility::Private,
    }
}

fn cloud_run_state(services: Vec<CloudRunService>) -> ResourcesState {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::CloudRunServices);
    state.set_cloud_run_services(context(), services);
    state
}

#[test]
fn set_cloud_run_services_populates_rows() {
    let state = cloud_run_state(vec![
        service("api", "us-central1"),
        service("web", "europe-west1"),
    ]);
    assert_eq!(state.cloud_run_services().len(), 2);
    assert!(state.has_loaded_context());
    assert_eq!(state.visible_cloud_run_service_count(), 2);
}

#[test]
fn set_cloud_run_services_preserves_selection_by_name_and_region() {
    let mut state = cloud_run_state(vec![
        service("api", "us-central1"),
        service("web", "europe-west1"),
    ]);
    // move selection to second row (web)
    state.select_next();
    let selected = state.selected_cloud_run_service().unwrap();
    assert_eq!(selected.name, "web");

    // refresh with reordered list; selection should follow web by identity
    state.set_cloud_run_services(
        context(),
        vec![
            service("web", "europe-west1"),
            service("api", "us-central1"),
            service("extra", "us-central1"),
        ],
    );
    let selected = state.selected_cloud_run_service().unwrap();
    assert_eq!(selected.name, "web");
    assert_eq!(selected.region, "europe-west1");
}

#[test]
fn cloud_run_name_filter_narrows_rows() {
    let mut state = cloud_run_state(vec![
        service("api", "us-central1"),
        service("web", "europe-west1"),
    ]);
    state.start_filter();
    for ch in "web".chars() {
        state.append_filter_char(ch);
    }
    state.apply_filter();
    assert_eq!(state.visible_cloud_run_service_count(), 1);
    assert_eq!(state.visible_cloud_run_services()[0].name, "web");
}

#[test]
fn cloud_run_sort_by_region_reorders() {
    let mut state = cloud_run_state(vec![
        service("api", "us-central1"),
        service("web", "asia-east1"),
    ]);
    // Region is column index 2
    assert_eq!(CloudRunSortField::Region.column(), 2);
    state.select_next_column(); // 0 -> 1
    state.select_next_column(); // 1 -> 2
    state.sort_selected_column();
    let visible = state.visible_cloud_run_services();
    assert_eq!(visible[0].region, "asia-east1");
    assert_eq!(visible[1].region, "us-central1");
}

#[test]
fn open_cloud_run_detail_sets_detail_view_and_target() {
    let mut state = cloud_run_state(vec![service("api", "us-central1")]);
    let opened = state.open_cloud_run_detail().unwrap();
    assert_eq!(opened.name, "api");
    assert!(state.is_showing_detail());
    assert_eq!(state.cloud_run_detail_target().unwrap().name, "api");
    assert_eq!(state.cloud_run_detail_target_name(), Some("api"));
    assert_eq!(
        state.active_cloud_run_detail_tab(),
        CloudRunDetailTab::Summary
    );
}

#[test]
fn open_cloud_run_revision_and_close_returns_to_service_detail() {
    let mut state = cloud_run_state(vec![service("api", "us-central1")]);
    state.open_cloud_run_detail();
    assert!(!state.is_showing_cloud_run_revision());

    state.open_cloud_run_revision(revision("api-00001"), "us-central1".to_string());
    assert!(state.is_showing_cloud_run_revision());
    assert_eq!(state.cloud_run_revision_target().unwrap().name, "api-00001");
    assert_eq!(
        state.active_cloud_run_revision_tab(),
        CloudRunRevisionTab::Summary
    );

    state.close_cloud_run_revision();
    assert!(!state.is_showing_cloud_run_revision());
    // still in service detail
    assert!(state.is_showing_detail());
}

#[test]
fn selected_cloud_run_revision_tracks_cursor_on_revisions_tab() {
    let mut state = cloud_run_state(vec![service("api", "us-central1")]);
    state.open_cloud_run_detail();
    state.set_cloud_run_detail(detail_with_revisions(
        "api",
        "us-central1",
        vec![
            revision("api-00001"),
            revision("api-00002"),
            revision("api-00003"),
        ],
    ));
    // move to Revisions tab
    while state.active_cloud_run_detail_tab() != CloudRunDetailTab::Revisions {
        state.select_next_tab();
    }
    assert_eq!(
        state.selected_cloud_run_revision().unwrap().name,
        "api-00001"
    );
    state.select_next();
    assert_eq!(
        state.selected_cloud_run_revision().unwrap().name,
        "api-00002"
    );
    state.select_previous();
    assert_eq!(
        state.selected_cloud_run_revision().unwrap().name,
        "api-00001"
    );
}

#[test]
fn cloud_run_detail_error_and_revision_detail_error_stored() {
    let mut state = cloud_run_state(vec![service("api", "us-central1")]);
    state.open_cloud_run_detail();
    state.set_cloud_run_detail_error("api".to_string(), "boom".to_string());
    assert_eq!(state.cloud_run_detail_error(), Some("boom"));
    assert!(state.is_showing_detail());

    state.open_cloud_run_revision(revision("api-00001"), "us-central1".to_string());
    state.set_cloud_run_revision_detail_error("nope".to_string());
    assert_eq!(state.cloud_run_revision_detail_error(), Some("nope"));
}

#[test]
fn cloud_run_detail_error_target_name_resolves_without_detail_target() {
    let mut state = cloud_run_state(vec![service("api", "us-central1")]);
    state.set_cloud_run_detail_error("api".to_string(), "boom".to_string());
    assert_eq!(state.cloud_run_detail_target_name(), Some("api"));
}

#[test]
fn cloud_run_detail_target_name_does_not_leak_cloud_sql_error_target() {
    // Regression test: the fallback arm of `cloud_run_detail_target_name`
    // read Cloud SQL's `detail_error_target_name`, so a stale Cloud SQL
    // detail error would surface as the Cloud Run detail target after the
    // user switched services. It must resolve from Cloud Run state only.
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::CloudSql);
    state.set_cloud_sql_instances(
        context(),
        vec![cloud_sql_instance("db-prod", "us-central1")],
    );
    state.open_cloud_sql_detail();
    state.set_cloud_sql_detail_error("db-prod".to_string(), "denied".to_string());

    state.select_service(ResourceServiceKind::CloudRunServices);
    state.set_cloud_run_services(context(), vec![service("api", "us-central1")]);

    state.set_cloud_run_detail_error("api".to_string(), "boom".to_string());
    assert_eq!(state.cloud_run_detail_target_name(), Some("api"));
}

#[test]
fn select_service_away_from_cloud_run_clears_stranded_revision_view() {
    // Regression test: switching services via the sidebar (select_service),
    // rather than backing out with Esc, previously left `cloud_run_revision`
    // (and its cursor) populated. That stale state would then bleed into
    // whatever service the user switched to next.
    let mut state = cloud_run_state(vec![service("api", "us-central1")]);
    state.open_cloud_run_detail();
    state.set_cloud_run_detail(detail_with_revisions(
        "api",
        "us-central1",
        vec![revision("api-00001"), revision("api-00002")],
    ));
    state.open_cloud_run_revision(revision("api-00001"), "us-central1".to_string());
    assert!(state.is_showing_cloud_run_revision());
    assert_eq!(state.selected_cloud_run_revision_index(), 0);

    // Switch away to a different service without closing the revision view.
    state.select_service(ResourceServiceKind::GkeClusters);

    assert!(!state.is_showing_cloud_run_revision());
    assert_eq!(state.selected_cloud_run_revision_index(), 0);
    assert_eq!(state.active_service(), ResourceServiceKind::GkeClusters);
}

#[test]
fn select_service_to_same_service_preserves_cloud_run_revision_view() {
    // select_service is a no-op guard when the service doesn't change; it
    // must not clobber an in-progress revision drill-down on re-selection of
    // the same service (e.g. repeated sidebar activation on Cloud Run).
    let mut state = cloud_run_state(vec![service("api", "us-central1")]);
    state.open_cloud_run_detail();
    state.open_cloud_run_revision(revision("api-00001"), "us-central1".to_string());
    assert!(state.is_showing_cloud_run_revision());

    state.select_service(ResourceServiceKind::CloudRunServices);

    assert!(state.is_showing_cloud_run_revision());
}

/// The Revisions tab has its own cursor, clamped by the same
/// `if len > 0 { (selected + 1).min(len - 1) }` shape as every list cursor —
/// but it hangs off the detail pane rather than off a `ResourceServiceKind`,
/// so the `ResourceServiceKind::ALL` walk cannot reach it. The three cases the
/// walk pins per service are pinned here by hand for the one cursor it misses:
/// an empty list, the last row, and the first row.
///
/// The empty case is not hypothetical padding: with `len > 0` weakened to
/// `len >= 0`, an empty revision list computes `len - 1` and underflows.
#[test]
fn the_revisions_cursor_stops_at_both_ends_of_the_list() {
    let mut state = cloud_run_state(vec![service("api", "us-central1")]);
    state.open_cloud_run_detail();
    state.set_cloud_run_detail(detail_with_revisions("api", "us-central1", Vec::new()));
    while state.active_cloud_run_detail_tab() != CloudRunDetailTab::Revisions {
        state.select_next_tab();
    }

    // A service whose revisions have not loaded, or which has none: the cursor
    // has no last index to clamp to and must not try to compute one.
    state.select_next();
    state.select_previous();
    assert_eq!(
        state.selected_cloud_run_revision_index(),
        0,
        "the cursor moved in a list with no revisions"
    );
    assert!(state.selected_cloud_run_revision().is_none());

    state.set_cloud_run_detail(detail_with_revisions(
        "api",
        "us-central1",
        vec![
            revision("api-00001"),
            revision("api-00002"),
            revision("api-00003"),
        ],
    ));

    // Walk to the end and press again: the last revision is a floor, and a
    // clamp raised above it selects a row that is not there.
    state.select_next();
    state.select_next();
    state.select_next();
    assert_eq!(
        state
            .selected_cloud_run_revision()
            .expect("the cursor must still be on a revision")
            .name,
        "api-00003",
        "the cursor walked past the last revision"
    );

    // And back to the top, where the first revision is a ceiling.
    state.select_previous();
    state.select_previous();
    state.select_previous();
    assert_eq!(
        state
            .selected_cloud_run_revision()
            .expect("the cursor must still be on a revision")
            .name,
        "api-00001",
        "the cursor walked off the top of the revision list"
    );
}
