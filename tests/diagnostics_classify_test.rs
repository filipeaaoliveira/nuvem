use nuvem_domain::gcp::diagnostics::{
    GcloudErrorKind, Retryability, classify_gcloud_stderr, stderr_indicates_requester_pays,
};

#[test]
fn classifies_permission_denied() {
    assert_eq!(
        classify_gcloud_stderr(
            "ERROR: (gcloud.compute.instances.list) Some requests did not succeed: PERMISSION_DENIED: The caller does not have permission"
        ),
        GcloudErrorKind::PermissionDenied
    );
}

#[test]
fn classifies_api_disabled() {
    assert_eq!(
        classify_gcloud_stderr(
            "Compute Engine API has not been used in project 123 before or it is disabled. Enable it by visiting ..."
        ),
        GcloudErrorKind::ApiDisabled
    );
}

#[test]
fn classifies_quota() {
    assert_eq!(
        classify_gcloud_stderr("Quota exceeded: RESOURCE_EXHAUSTED for quota metric 'Queries'"),
        GcloudErrorKind::Quota
    );
}

#[test]
fn classifies_network() {
    assert_eq!(
        classify_gcloud_stderr(
            "ERROR: Could not reach the server: connection timed out (getaddrinfo failed)"
        ),
        GcloudErrorKind::Network
    );
}

#[test]
fn classifies_missing_project() {
    assert_eq!(
        classify_gcloud_stderr("ERROR: The required property [project] is not currently set."),
        GcloudErrorKind::MissingProject
    );
}

#[test]
fn unknown_falls_through() {
    assert_eq!(
        classify_gcloud_stderr("something totally unexpected"),
        GcloudErrorKind::Unknown
    );
}

#[test]
fn detects_requester_pays_stderr() {
    assert!(stderr_indicates_requester_pays(
        "ERROR: (gcloud.storage.ls) Bucket is requester pays bucket but no user project provided."
    ));
    assert!(stderr_indicates_requester_pays(
        "userProjectMissing: Bucket is a requester pays bucket but no user project was provided."
    ));
    assert!(!stderr_indicates_requester_pays(
        "PERMISSION_DENIED: The caller does not have permission"
    ));
}

#[test]
fn label_is_human_readable() {
    assert_eq!(
        GcloudErrorKind::PermissionDenied.label(),
        "permission denied"
    );
    assert_eq!(GcloudErrorKind::ApiDisabled.label(), "API disabled");
}

#[test]
fn classifies_invalid_filter_query() {
    use nuvem_domain::gcp::diagnostics::{GcloudErrorKind, classify_gcloud_stderr};
    assert_eq!(
        classify_gcloud_stderr("ERROR: (gcloud.logging.read) INVALID_ARGUMENT: unparseable filter"),
        GcloudErrorKind::InvalidQuery
    );
    assert_eq!(
        classify_gcloud_stderr("ERROR: (400) Invalid filter argument"),
        GcloudErrorKind::InvalidQuery
    );
    assert_eq!(GcloudErrorKind::InvalidQuery.label(), "invalid query");
}

#[test]
fn classifies_unauthenticated_stderr() {
    for text in [
        "ERROR: (gcloud.compute.instances.list) There was a problem refreshing your current auth tokens: invalid_grant",
        "You do not currently have an active account selected. Please run: gcloud auth login",
        "Reauthentication required. Please run: gcloud auth login",
        "ERROR: (gcloud) Your credentials are invalid. (401)",
    ] {
        assert_eq!(
            classify_gcloud_stderr(text),
            GcloudErrorKind::Unauthenticated,
            "{text}"
        );
    }
}

#[test]
fn classifies_invalid_input_stderr() {
    for text in [
        "ERROR: (gcloud.compute.instances.describe) argument INSTANCE_NAME: Invalid value 'my bad name'",
        "ERROR: (gcloud) unrecognized arguments: --nope",
        "ERROR: Invalid argument: zone (400)",
    ] {
        assert_eq!(
            classify_gcloud_stderr(text),
            GcloudErrorKind::InvalidInput,
            "{text}"
        );
    }
}

#[test]
fn permission_and_api_still_win_over_new_categories() {
    assert_eq!(
        classify_gcloud_stderr("PERMISSION_DENIED: forbidden (403)"),
        GcloudErrorKind::PermissionDenied
    );
    assert_eq!(
        classify_gcloud_stderr(
            "Compute Engine API has not been used in project 5 before or it is disabled"
        ),
        GcloudErrorKind::ApiDisabled
    );
}

#[test]
fn unknown_when_no_pattern_matches() {
    assert_eq!(
        classify_gcloud_stderr("something entirely unexpected"),
        GcloudErrorKind::Unknown
    );
}

#[test]
fn retryability_gates_persistent_vs_transient() {
    assert_eq!(
        GcloudErrorKind::Network.retryability(),
        Retryability::Transient
    );
    assert_eq!(
        GcloudErrorKind::Quota.retryability(),
        Retryability::Transient
    );
    assert_eq!(
        GcloudErrorKind::Unknown.retryability(),
        Retryability::Transient
    );
    for kind in [
        GcloudErrorKind::PermissionDenied,
        GcloudErrorKind::ApiDisabled,
        GcloudErrorKind::MissingProject,
        GcloudErrorKind::Unauthenticated,
        GcloudErrorKind::InvalidInput,
        GcloudErrorKind::Parse,
        GcloudErrorKind::MissingExecutable,
        GcloudErrorKind::Cancelled,
    ] {
        assert_eq!(kind.retryability(), Retryability::Persistent, "{kind:?}");
    }
}

#[test]
fn user_action_is_present_and_nonempty_for_every_kind() {
    for kind in [
        GcloudErrorKind::PermissionDenied,
        GcloudErrorKind::ApiDisabled,
        GcloudErrorKind::Quota,
        GcloudErrorKind::Network,
        GcloudErrorKind::Parse,
        GcloudErrorKind::MissingProject,
        GcloudErrorKind::Unknown,
        GcloudErrorKind::MissingExecutable,
        GcloudErrorKind::Unauthenticated,
        GcloudErrorKind::InvalidInput,
        GcloudErrorKind::Cancelled,
    ] {
        assert!(!kind.user_action().is_empty(), "{kind:?}");
    }
}
