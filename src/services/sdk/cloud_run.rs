//! Cloud Run services through the native client.
//!
//! The CLI reads the Knative v1 shape (`metadata`/`spec`/`status`) that
//! `gcloud run services list` returns; this reads the v2 resource, whose
//! fields map onto the row more directly. The one place the two genuinely
//! differ is readiness: Knative reports a `Ready` condition whose status is the
//! string `"True"`/`"False"`, while v2 reports a `terminal_condition` carrying
//! a `State` enum. Rather than pretend the v2 value is a Knative one, the
//! vocabulary is translated explicitly below.

use google_cloud_auth::credentials::Credentials;
use google_cloud_run_v2::client::{Revisions, Services};
use nuvem_domain::gcp::resource::{
    CloudRunCondition, CloudRunRevision, CloudRunRevisionDetail, CloudRunService,
    CloudRunServiceDetail, CloudRunServiceStatus, CloudRunTrafficTarget, ComputeKeyValue,
};

use crate::services::resources::cloud_run::traffic_summary_from;
use crate::services::resources::list_filter::{RESOURCE_LIST_LIMIT, rows_are_capped};
use crate::services::sdk::{ApiError, detail_json, non_empty, timestamp_to_string};

pub struct SdkCloudRunService {
    services: Services,
    revisions: Revisions,
}

impl SdkCloudRunService {
    pub async fn connect(credentials: Credentials) -> Result<Self, ApiError> {
        let services = Services::builder()
            .with_credentials(credentials.clone())
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        let revisions = Revisions::builder()
            .with_credentials(credentials)
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        Ok(Self {
            services,
            revisions,
        })
    }

    /// The same service, built from already-constructed clients. See
    /// `SdkGkeService::from_clusters_for_test`.
    pub fn from_clients_for_test(services: Services, revisions: Revisions) -> Self {
        Self {
            services,
            revisions,
        }
    }

    /// The same service, answering from the demo fixtures.
    ///
    /// Required by `sdk_client_cache!`: the macro's `Fixtures` arm calls this
    /// on every service in its list, so a fifteenth service cannot join that
    /// list without one. It takes no credentials, cannot fail and cannot
    /// block -- a client built by `from_stub` holds a stub where a
    /// transport would be.
    pub fn from_fixtures() -> Self {
        let (services, revisions) = crate::services::demo::sdk::cloud_run::clients();
        Self {
            services,
            revisions,
        }
    }

    /// The same service over the demo showcase, which is what `nuvem --demo`
    /// lists. See [`crate::services::demo::Corpus`].
    pub fn from_showcase() -> Self {
        let (services, revisions) = crate::services::demo::sdk::cloud_run::clients_of(
            crate::services::demo::Corpus::Showcase,
        );
        Self {
            services,
            revisions,
        }
    }

    /// Services in every region: `-` is the API's own location wildcard, which
    /// is what `gcloud run services list` asks for unqualified.
    pub async fn load_services(&self, project: &str) -> Result<Vec<CloudRunService>, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;
        let mut rows = Vec::new();
        let mut items = self
            .services
            .list_services()
            .set_parent(format!("projects/{project}/locations/-"))
            .set_page_size(RESOURCE_LIST_LIMIT as i32)
            .by_item();
        while let Some(service) = items.next().await {
            rows.push(map_service(&service?));
            if rows_are_capped(rows.len(), RESOURCE_LIST_LIMIT) {
                break;
            }
        }
        Ok(rows)
    }

    /// The revisions of one service. `active_percent` stays unset here exactly
    /// as it does on the CLI path: the traffic split belongs to the service,
    /// and the revision list is refreshed independently of it.
    pub async fn load_revisions(
        &self,
        project: &str,
        region: &str,
        service: &str,
    ) -> Result<Vec<CloudRunRevision>, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;
        let mut rows = Vec::new();
        let mut items = self
            .revisions
            .list_revisions()
            .set_parent(format!(
                "projects/{project}/locations/{region}/services/{service}"
            ))
            .by_item();
        while let Some(revision) = items.next().await {
            rows.push(map_revision(&revision?));
        }
        Ok(rows)
    }

    /// One service described in full, for the detail pane.
    ///
    /// `revisions` is derived from the traffic split, as it is on the CLI
    /// path, and is replaced by the pane's own revisions fetch when that
    /// lands. See `revisions_from_traffic`.
    pub async fn load_service_detail(
        &self,
        project: &str,
        region: &str,
        service: &str,
    ) -> Result<CloudRunServiceDetail, ApiError> {
        let service = self
            .services
            .get_service()
            .set_name(format!(
                "projects/{project}/locations/{region}/services/{service}"
            ))
            .send()
            .await?;
        Ok(map_service_detail(&service))
    }

    /// One revision described in full, fetched on its own rather than through
    /// its service.
    pub async fn load_revision_detail(
        &self,
        project: &str,
        region: &str,
        revision: &str,
    ) -> Result<CloudRunRevisionDetail, ApiError> {
        let revision = self
            .revisions
            .get_revision()
            .set_name(format!(
                "projects/{project}/locations/{region}/revisions/{revision}"
            ))
            .send()
            .await?;
        Ok(map_revision_detail(&revision))
    }
}

/// A revision reports readiness in its condition list rather than in a single
/// terminal condition, so the `Ready` entry is the one that carries the status
/// the row shows.
pub fn map_revision(revision: &google_cloud_run_v2::model::Revision) -> CloudRunRevision {
    let ready = revision
        .conditions
        .iter()
        .find(|condition| condition.r#type == "Ready");

    CloudRunRevision {
        name: name_and_region(&revision.name).0,
        ready: status_from_state(ready),
        created: timestamp_to_string(revision.create_time.as_ref()),
        image: revision
            .containers
            .first()
            .and_then(|container| non_empty(&container.image)),
        active_percent: None,
    }
}

/// `projects/<p>/locations/<r>/services/<s>` — the row shows the last segment
/// as the name and the location as the region, because v2 carries neither as
/// its own field.
fn name_and_region(full_name: &str) -> (String, String) {
    let segments: Vec<&str> = full_name.split('/').collect();
    let name = segments.last().map(|s| s.to_string()).unwrap_or_default();
    let region = segments
        .iter()
        .position(|segment| *segment == "locations")
        .and_then(|at| segments.get(at + 1))
        .map(|region| region.to_string())
        .unwrap_or_default();
    (name, region)
}

/// The v2 condition vocabulary, mapped onto the same statuses the Knative path
/// produces. `CONDITION_PENDING` and `CONDITION_RECONCILING` are both "still
/// settling", which is what `Reconciling` means on the CLI path too.
///
/// # Why a failed condition is read twice
///
/// `CONDITION_FAILED` is two Knative states in one word. The document the CLI
/// reads says `Ready: "False"` for both, and `normalize_status` splits them on
/// the `reason` key: a deploy that failed states one, a service that has
/// simply not come up yet states none -- `Failed` against `NotReady`, Degraded
/// against Warning in the STATUS cell. v2 folded the two states into one, but
/// it kept the distinction in the `reasons` one-of, so the same split is made
/// on the same evidence rather than lost. Reading only the state made
/// `NotReady` unreachable on this path: every not-yet-ready service painted
/// "Failed" toned Degraded where the CLI painted "Not Ready" toned Warning.
///
/// The one-of is tested for presence, not for a name it can be printed under.
/// A reason that arrives as an integer this build has no word for is still a
/// stated cause, and reading it as an absent one would put the service back in
/// the wrong cell -- which is the defect this exists to close, in the one case
/// where the API has told us most.
///
/// # Why the `Unknown` payload is translated too
///
/// A state with no mapping above is still a *stated* readiness, and the CLI
/// path carries the word its own document used inside the variant. The
/// services table paints that payload through `CloudRunServiceStatus::label`,
/// so `Unknown("STATE_UNSPECIFIED")` and `Unknown("Unknown")` are two
/// different cells on screen (`State unspecified` and `Unknown`), not two
/// spellings of one. Carrying the raw v2 token there would make the STATUS cell of a
/// service read differently depending on which backend filled it, which is the
/// same thing [`condition_status`] exists to prevent one pane over; so the
/// same translation is applied, and the payload is the Knative word on both
/// paths.
///
/// # What the empty payload actually marks
///
/// Not "absent condition" -- two facts share it, and deliberately. A service
/// with no terminal condition at all and a condition whose state is an integer
/// this build has no word for both answer `Unknown(String::new())`, because
/// neither states a readiness and so neither has a Knative word to carry.
/// `normalize_status` spells the first of them the same way on the CLI path,
/// and the second has no CLI analogue: there the status is already a string.
///
/// The distinction that is real is the one against a *named* state this
/// mapping does not handle -- `STATE_UNSPECIFIED`, and whatever the API adds
/// after this build. Those did state a readiness, and carry the Knative word
/// for an indefinite one.
fn status_from_state(
    condition: Option<&google_cloud_run_v2::model::Condition>,
) -> CloudRunServiceStatus {
    let Some(condition) = condition else {
        return CloudRunServiceStatus::Unknown(String::new());
    };

    match condition.state.name() {
        Some("CONDITION_SUCCEEDED") => CloudRunServiceStatus::Ready,
        Some("CONDITION_FAILED") => {
            if condition.reasons.is_some() {
                CloudRunServiceStatus::Failed
            } else {
                CloudRunServiceStatus::NotReady
            }
        }
        Some("CONDITION_PENDING") | Some("CONDITION_RECONCILING") => {
            CloudRunServiceStatus::Reconciling
        }
        // Written as the constant rather than as `condition_status(state)`.
        // Every state that function names has already been matched above, so
        // the call could only ever return this word: it reads as a
        // translation and computes a literal.
        Some(_) => CloudRunServiceStatus::Unknown(INDEFINITE_STATUS.to_string()),
        None => CloudRunServiceStatus::Unknown(String::new()),
    }
}

pub fn map_service(service: &google_cloud_run_v2::model::Service) -> CloudRunService {
    let (name, region) = name_and_region(&service.name);
    let terminal = service.terminal_condition.as_ref();

    CloudRunService {
        name,
        region,
        url: non_empty(&service.uri),
        latest_ready_revision: non_empty(&service.latest_ready_revision)
            .map(|full| name_and_region(&full).0),
        latest_created_revision: non_empty(&service.latest_created_revision)
            .map(|full| name_and_region(&full).0),
        traffic_summary: traffic_summary_from(
            &service
                .traffic_statuses
                .iter()
                .filter(|status| !status.revision.is_empty())
                .map(|status| (status.revision.as_str(), i64::from(status.percent)))
                .collect::<Vec<_>>(),
        ),
        status: status_from_state(terminal),
        ready_message: terminal.and_then(|condition| non_empty(&condition.message)),
    }
}

/// The Knative status word for a v2 condition state.
///
/// The pane renders `status` as the `"True"`/`"False"`/`"Unknown"` vocabulary
/// the CLI path produces, so the v2 enum is translated rather than shown raw --
/// a condition row must read the same whichever backend filled it.
fn condition_status(state: Option<&str>) -> String {
    match state {
        Some("CONDITION_SUCCEEDED") => "True",
        Some("CONDITION_FAILED") => "False",
        _ => INDEFINITE_STATUS,
    }
    .to_string()
}

/// The Knative word for a readiness that is neither true nor false.
///
/// Named because two places answer it and must not drift: the Conditions pane
/// through [`condition_status`], and the services table through
/// [`status_from_state`], which paints the whole `Unknown(_)` variant with
/// `{:?}`. One word changed in one of them would be a cell that reads
/// differently depending on which pane you are looking at.
const INDEFINITE_STATUS: &str = "Unknown";

/// The reason word a v2 condition carries, whichever of the three flavours it
/// is.
///
/// v2 splits what the CLI's JSON called `reason` into a one-of: a service-level
/// reason, a revision reason and an execution reason. The pane prints one
/// field, and a failed deploy says why in exactly one of them -- the
/// `RevisionMissing` or `ContainerMissing` token an operator reads to find out
/// what went wrong. Dropping it left the Conditions pane reporting failure with
/// no cause on the native backend.
fn condition_reason(source: &google_cloud_run_v2::model::Condition) -> Option<String> {
    use google_cloud_run_v2::model::condition::Reasons;
    match source.reasons.as_ref()? {
        Reasons::Reason(reason) => reason.name(),
        Reasons::RevisionReason(reason) => reason.name(),
        Reasons::ExecutionReason(reason) => reason.name(),
        _ => None,
    }
    .map(str::to_string)
}

fn condition(source: &google_cloud_run_v2::model::Condition) -> CloudRunCondition {
    CloudRunCondition {
        type_: source.r#type.clone(),
        status: condition_status(source.state.name()),
        reason: condition_reason(source),
        message: non_empty(&source.message),
        last_transition_time: timestamp_to_string(source.last_transition_time.as_ref()),
    }
}

/// The ingress setting in the vocabulary the pane already prints.
///
/// The CLI path reads the `run.googleapis.com/ingress` annotation, whose values
/// are `all`, `internal` and `internal-and-cloud-load-balancing`; v2 names the
/// same settings `INGRESS_TRAFFIC_ALL` and friends. One cell renders both
/// backends, so the enum is translated rather than shown raw. `UNSPECIFIED`
/// means the service states no setting, which is an absent value and not the
/// word `INGRESS_TRAFFIC_UNSPECIFIED` painted into the cell.
fn ingress_word(name: Option<&str>) -> Option<String> {
    match name? {
        "INGRESS_TRAFFIC_ALL" => Some("all"),
        "INGRESS_TRAFFIC_INTERNAL_ONLY" => Some("internal"),
        "INGRESS_TRAFFIC_INTERNAL_LOAD_BALANCER" => Some("internal-and-cloud-load-balancing"),
        "INGRESS_TRAFFIC_UNSPECIFIED" => None,
        // A setting this build predates: shown as the API spells it rather than
        // dropped, because an unknown ingress is still a fact about the service.
        other => Some(other),
    }
    .map(str::to_string)
}

/// The revision rows a service document can state on its own, derived from the
/// split it reports.
///
/// The CLI pane has always filled these -- `derive_revisions_from_status` in
/// `resources::cloud_run` names each traffic target with its percentage and
/// marks the latest-ready one Ready -- and the native pane left the list empty
/// and waited for the pane's own revisions fetch. So in the instant between
/// the drill-in and that fetch the native pane showed an operator strictly
/// less than the CLI pane did: no revisions at all where the CLI named the one
/// serving and the one being replaced. This is the same derivation over the
/// same facts, read out of v2's `traffic_statuses` rather than Knative's
/// `status.traffic`.
///
/// The health here is what a split can say and no more: the target that is the
/// latest ready revision is Ready, and every other target is an empty
/// `Unknown` rather than a guess. True per-revision health arrives with the
/// revisions fetch and replaces this.
fn revisions_from_traffic(service: &google_cloud_run_v2::model::Service) -> Vec<CloudRunRevision> {
    let latest_ready =
        non_empty(&service.latest_ready_revision).map(|full| name_and_region(&full).0);
    let mut revisions: Vec<CloudRunRevision> = Vec::new();

    for status in &service.traffic_statuses {
        // A status names the revision it resolved to, so an empty one is a
        // target with nothing to show rather than the latest revision.
        let Some(name) = non_empty(&status.revision).map(|full| name_and_region(&full).0) else {
            continue;
        };
        // A tagged target and its untagged self are two statuses naming one
        // revision; the pane lists revisions, not targets.
        if revisions.iter().any(|revision| revision.name == name) {
            continue;
        }
        let ready = if latest_ready.as_deref() == Some(name.as_str()) {
            CloudRunServiceStatus::Ready
        } else {
            CloudRunServiceStatus::Unknown(String::new())
        };
        revisions.push(CloudRunRevision {
            name,
            ready,
            created: None,
            image: None,
            active_percent: u32::try_from(status.percent).ok(),
        });
    }

    revisions
}

fn container_limit(
    containers: &[google_cloud_run_v2::model::Container],
    limit: &str,
) -> Option<String> {
    containers
        .first()
        .and_then(|container| container.resources.as_ref())
        .and_then(|resources| resources.limits.get(limit).cloned())
}

pub fn map_service_detail(service: &google_cloud_run_v2::model::Service) -> CloudRunServiceDetail {
    let (name, region) = name_and_region(&service.name);
    let template = service.template.as_ref();
    let containers = template
        .map(|template| template.containers.as_slice())
        .unwrap_or_default();
    let scaling = template.and_then(|template| template.scaling.as_ref());
    let terminal = service.terminal_condition.as_ref();

    CloudRunServiceDetail {
        name,
        region,
        url: non_empty(&service.uri),
        status: status_from_state(terminal),
        latest_ready_revision: non_empty(&service.latest_ready_revision)
            .map(|full| name_and_region(&full).0),
        latest_created_revision: non_empty(&service.latest_created_revision)
            .map(|full| name_and_region(&full).0),
        // The split as the service *reports* it, not as it was requested:
        // `traffic_statuses` is what is actually serving.
        traffic: service
            .traffic_statuses
            .iter()
            .map(|status| CloudRunTrafficTarget {
                revision_name: non_empty(&status.revision).map(|full| name_and_region(&full).0),
                tag: non_empty(&status.tag),
                percent: u32::try_from(status.percent).ok(),
                // The allocation type, not an empty revision name: a status
                // always names the revision it resolved to, so testing that
                // string made `LATEST` unreachable and the split printed a
                // revision id where the CLI printed the word.
                latest: status.r#type
                    == google_cloud_run_v2::model::TrafficTargetAllocationType::Latest,
                url: non_empty(&status.uri),
            })
            .collect(),
        // Derived from the split until the pane's own revisions fetch lands,
        // as on the CLI path.
        revisions: revisions_from_traffic(service),
        // The terminal condition first, then the rest. v2 moved readiness out
        // of `conditions` into its own field, so feeding only `conditions`
        // dropped the `Ready` row the CLI path always showed -- the one line in
        // the pane that says whether the service is serving.
        conditions: terminal
            .map(condition)
            .into_iter()
            .chain(service.conditions.iter().map(condition))
            .collect(),
        image: containers
            .first()
            .and_then(|container| non_empty(&container.image)),
        cpu: container_limit(containers, "cpu"),
        memory: container_limit(containers, "memory"),
        concurrency: template
            .map(|template| template.max_instance_request_concurrency)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value != 0),
        service_account: template.and_then(|template| non_empty(&template.service_account)),
        ingress: ingress_word(service.ingress.name()),
        // A minimum of zero is a real setting -- scale to zero -- so it is
        // kept whenever the scaling block is present. A maximum of zero cannot
        // be, so there it is the unset sentinel and reads as absent.
        min_instances: scaling.and_then(|scaling| u32::try_from(scaling.min_instance_count).ok()),
        max_instances: scaling
            .and_then(|scaling| u32::try_from(scaling.max_instance_count).ok())
            .filter(|value| *value != 0),
        labels: service
            .labels
            .iter()
            .map(|(key, value)| ComputeKeyValue {
                key: key.clone(),
                value: value.clone(),
            })
            .collect(),
        raw_json: detail_json(service),
    }
}

pub fn map_revision_detail(
    revision: &google_cloud_run_v2::model::Revision,
) -> CloudRunRevisionDetail {
    let (name, region) = name_and_region(&revision.name);
    let ready = revision
        .conditions
        .iter()
        .find(|condition| condition.r#type == "Ready");

    CloudRunRevisionDetail {
        name,
        service: non_empty(&revision.service).map(|full| name_and_region(&full).0),
        region,
        ready: status_from_state(ready),
        conditions: revision.conditions.iter().map(condition).collect(),
        image: revision
            .containers
            .first()
            .and_then(|container| non_empty(&container.image)),
        cpu: container_limit(&revision.containers, "cpu"),
        memory: container_limit(&revision.containers, "memory"),
        concurrency: u32::try_from(revision.max_instance_request_concurrency)
            .ok()
            .filter(|value| *value != 0),
        min_instances: revision
            .scaling
            .as_ref()
            .and_then(|scaling| u32::try_from(scaling.min_instance_count).ok()),
        max_instances: revision
            .scaling
            .as_ref()
            .and_then(|scaling| u32::try_from(scaling.max_instance_count).ok())
            .filter(|value| *value != 0),
        service_account: non_empty(&revision.service_account),
        created: timestamp_to_string(revision.create_time.as_ref()),
        raw_json: detail_json(revision),
    }
}
