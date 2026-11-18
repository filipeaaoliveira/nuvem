//! `DetailRequest::check`: a detail load that answers with another resource's
//! document settles as that load's failure instead of being dropped as stale.
//!
//! Each case is one detail kind, built from a capture whose resource is known,
//! and asked about by the resource the capture describes, which must pass
//! through untouched, and then once per compared field by a request that
//! differs in that field alone, which must become the failure for what was
//! asked. Without the second half, the
//! reducer drops the answer and the pane says "Loading detail for ..." forever.

use nuvem::app::effect::DetailRequest;
use nuvem::app::{AppError, EffectResult};
use nuvem::services::resources::{cloud_run, cloudsql, compute, gke, secrets, storage};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};

fn context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

/// The error text a failed result carries, or `None` for any other result.
fn failure(result: &EffectResult) -> Option<(String, String)> {
    let (asked, error): (String, &AppError) = match result {
        EffectResult::ComputeInstanceDetailLoadFailed {
            instance_name,
            instance_zone,
            error,
            ..
        } => (format!("{instance_name}@{instance_zone}"), error),
        EffectResult::GkeClusterDetailLoadFailed {
            cluster_name,
            cluster_location,
            error,
            ..
        } => (format!("{cluster_name}@{cluster_location}"), error),
        EffectResult::CloudSqlInstanceDetailLoadFailed {
            instance_name,
            instance_region,
            error,
            ..
        } => (format!("{instance_name}@{instance_region}"), error),
        EffectResult::StorageBucketDetailLoadFailed {
            bucket_name, error, ..
        } => (bucket_name.clone(), error),
        EffectResult::StorageObjectDetailLoadFailed {
            object_key, error, ..
        } => (object_key.clone(), error),
        EffectResult::SecretDetailLoadFailed {
            secret_name, error, ..
        } => (secret_name.clone(), error),
        EffectResult::CloudRunServiceDetailLoadFailed {
            service_name,
            service_region,
            error,
            ..
        } => (format!("{service_name}@{service_region}"), error),
        EffectResult::CloudRunRevisionDetailLoadFailed {
            revision_name,
            region,
            error,
            ..
        } => (format!("{revision_name}@{region}"), error),
        _ => return None,
    };
    Some((asked, error.to_string()))
}

/// One case: the result a load produced, the request that names its own
/// resource, a request for another one, and how that other one is spelled in
/// the failure.
struct Case {
    loaded: EffectResult,
    matching: DetailRequest,
    /// One request per compared field, each differing from `matching` in
    /// that field alone, with how the failure spells it.
    others: Vec<(DetailRequest, &'static str)>,
}

fn cases() -> Vec<Case> {
    let compute =
        compute::parse_instance_detail_json(include_str!("fixtures/gce_instance_detail.json"))
            .expect("parses");
    let gke = gke::parse_cluster_detail_json(include_str!("fixtures/gke_cluster_detail.json"))
        .expect("parses");
    let sql = cloudsql::parse_instance_detail_json(include_str!(
        "fixtures/cloudsql_instance_detail.json"
    ))
    .expect("parses");
    let bucket = storage::parse_bucket_detail_json(include_str!("fixtures/gcs_bucket_detail.json"))
        .expect("parses");
    let object = storage::parse_object_detail_json(include_str!("fixtures/gcs_object_detail.json"))
        .expect("parses");
    let secret = secrets::parse_secret_detail_json(include_str!("fixtures/secret_detail.json"))
        .expect("parses");
    let service = cloud_run::parse_service_detail_json(include_str!(
        "fixtures/cloud_run_service_detail.json"
    ))
    .expect("parses");
    let revision = cloud_run::parse_revision_detail_json(include_str!(
        "fixtures/cloud_run_revision_detail.json"
    ))
    .expect("parses");

    vec![
        Case {
            matching: DetailRequest::ComputeInstance {
                name: compute.name.clone(),
                zone: compute.zone.clone(),
            },
            others: vec![
                (
                    DetailRequest::ComputeInstance {
                        name: "other".into(),
                        zone: compute.zone.clone(),
                    },
                    "other@",
                ),
                (
                    DetailRequest::ComputeInstance {
                        name: compute.name.clone(),
                        zone: "elsewhere".into(),
                    },
                    "@elsewhere",
                ),
            ],
            loaded: EffectResult::ComputeInstanceDetailLoaded {
                context: context(),
                detail: compute,
            },
        },
        Case {
            matching: DetailRequest::GkeCluster {
                name: gke.name.clone(),
                location: gke.location.clone(),
            },
            others: vec![
                (
                    DetailRequest::GkeCluster {
                        name: gke.name.clone(),
                        location: "elsewhere".into(),
                    },
                    "@elsewhere",
                ),
                (
                    DetailRequest::GkeCluster {
                        name: "other".into(),
                        location: gke.location.clone(),
                    },
                    "other@",
                ),
            ],
            loaded: EffectResult::GkeClusterDetailLoaded {
                context: context(),
                detail: gke,
            },
        },
        Case {
            matching: DetailRequest::CloudSqlInstance {
                name: sql.name.clone(),
                region: sql.region.clone(),
            },
            others: vec![
                (
                    DetailRequest::CloudSqlInstance {
                        name: "other".into(),
                        region: sql.region.clone(),
                    },
                    "other@",
                ),
                (
                    DetailRequest::CloudSqlInstance {
                        name: sql.name.clone(),
                        region: "elsewhere".into(),
                    },
                    "@elsewhere",
                ),
            ],
            loaded: EffectResult::CloudSqlInstanceDetailLoaded {
                context: context(),
                detail: Box::new(sql),
            },
        },
        Case {
            matching: DetailRequest::StorageBucket {
                name: bucket.name.clone(),
            },
            others: vec![(
                DetailRequest::StorageBucket {
                    name: "other".into(),
                },
                "other",
            )],
            loaded: EffectResult::StorageBucketDetailLoaded {
                context: context(),
                detail: bucket,
            },
        },
        Case {
            matching: DetailRequest::StorageObject {
                key: object.key.clone(),
            },
            others: vec![(
                DetailRequest::StorageObject {
                    key: "other".into(),
                },
                "other",
            )],
            loaded: EffectResult::StorageObjectDetailLoaded {
                context: context(),
                detail: object,
            },
        },
        Case {
            matching: DetailRequest::Secret {
                name: secret.name.clone(),
            },
            others: vec![(
                DetailRequest::Secret {
                    name: "other".into(),
                },
                "other",
            )],
            loaded: EffectResult::SecretDetailLoaded {
                context: context(),
                detail: secret,
            },
        },
        Case {
            matching: DetailRequest::CloudRunService {
                name: service.name.clone(),
                region: service.region.clone(),
            },
            others: vec![
                (
                    DetailRequest::CloudRunService {
                        name: service.name.clone(),
                        region: "elsewhere".into(),
                    },
                    "@elsewhere",
                ),
                (
                    DetailRequest::CloudRunService {
                        name: "other".into(),
                        region: service.region.clone(),
                    },
                    "other@",
                ),
            ],
            loaded: EffectResult::CloudRunServiceDetailLoaded {
                context: context(),
                detail: service,
            },
        },
        Case {
            matching: DetailRequest::CloudRunRevision {
                name: revision.name.clone(),
                region: revision.region.clone(),
            },
            others: vec![(
                DetailRequest::CloudRunRevision {
                    name: "other".into(),
                    region: revision.region.clone(),
                },
                "other@",
            )],
            loaded: EffectResult::CloudRunRevisionDetailLoaded {
                context: context(),
                detail: revision,
            },
        },
    ]
}

#[test]
fn a_detail_answered_for_the_resource_asked_about_passes_through() {
    for case in cases() {
        let checked = case.matching.clone().check(case.loaded.clone());
        assert_eq!(
            checked, case.loaded,
            "{:?}: a detail for the resource it asked about must reach the pane",
            case.matching
        );
    }
}

#[test]
fn a_detail_answered_for_another_resource_becomes_the_requests_failure() {
    for case in cases() {
        for (other, spelled) in case.others {
            let checked = other.clone().check(case.loaded.clone());
            let (asked, message) = failure(&checked).unwrap_or_else(|| {
                panic!(
                    "{other:?}: a detail about another resource must become this \
                     load's failure, got {checked:?}"
                )
            });
            assert!(
                asked.contains(spelled),
                "{other:?}: the failure must name the resource that was asked for, \
                 so the pane showing it accepts it; it names {asked}"
            );
            assert!(
                message.contains("asked for") && message.contains("describes"),
                "{other:?}: the failure says what was asked and what came back: {message}"
            );
        }
    }
}

/// Every effect that is not a detail load is not held to anything.
#[test]
fn only_detail_loads_are_checked() {
    let effect = nuvem::app::Effect::RefreshComputeInstances {
        context: context(),
        filter: None,
    };
    assert_eq!(DetailRequest::of(&effect), None);
}
