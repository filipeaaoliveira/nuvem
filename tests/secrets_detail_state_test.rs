//! Secret Manager's detail-pane state and its filter plumbing.
//!
//! `tests/secrets_state_test.rs` pins list-side transitions; this file pins the
//! answers the detail pane and the filter are read for — which secret the pane
//! targets, what survives a reload, what a filter searches — at the exact
//! comparisons and accessors where a flipped operator or a stubbed return would
//! still leave a plausible-looking frame on screen.

use nuvem::app::state::resources::{
    ResourceService, ResourcesState, SecretDetailTab, SecretSortField, Secrets,
};
use nuvem::services::resources::secrets::parse_secret_detail_json;
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::diagnostics::{DiagnosticProblem, GcloudErrorKind};
use nuvem_domain::gcp::filter::compile_filter;
use nuvem_domain::gcp::resource::{ComputeKeyValue, ResourceServiceKind, Secret};

const SECRETS: ResourceServiceKind = ResourceServiceKind::SecretManagerSecrets;

fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".into()),
        region: None,
        zone: None,
    }
}

fn secret(name: &str) -> Secret {
    Secret {
        name: name.into(),
        create_time: Some("2026-01-02T00:00:00Z".into()),
        labels: vec![ComputeKeyValue {
            key: "env".into(),
            value: "prod".into(),
        }],
        replication: "Automatic".into(),
    }
}

/// A state with Secret Manager active and `names` delivered for [`ctx`].
fn loaded(names: &[&str]) -> ResourcesState {
    let mut state = ResourcesState::default();
    state.select_service(SECRETS);
    state.set_secrets(ctx(), names.iter().copied().map(secret).collect());
    state
}

/// Types `filter` into the secrets filter input and applies it.
fn apply_filter(state: &mut ResourcesState, filter: &str) {
    state.start_filter();
    for ch in filter.chars() {
        state.append_filter_char(ch);
    }
    state.apply_filter();
}

fn visible_names(state: &ResourcesState) -> Vec<String> {
    state
        .visible_secrets()
        .iter()
        .map(|secret| secret.name.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Filtering
// ---------------------------------------------------------------------------

#[test]
fn the_secrets_service_answers_the_filter_per_row_not_for_the_whole_list() {
    let filter = compile_filter("db").expect("`db` is a valid filter");

    assert!(
        Secrets::matches_filter(&secret("db-password"), &filter),
        "a secret whose name contains the filter text must match, or the \
         filter would hide the very row the operator typed it to find"
    );
    assert!(
        !Secrets::matches_filter(&secret("api-signing-key"), &filter),
        "a secret matching nothing in the filter must not match, or the \
         filter would narrow nothing at all"
    );
}

#[test]
fn a_secrets_filter_searches_name_replication_created_time_and_labels() {
    for (needle, field) in [
        ("api-signing", "name"),
        ("Automatic", "replication"),
        ("2026-01-02", "create time"),
        ("env=prod", "labels, spelled key=value"),
    ] {
        let mut state = loaded(&["api-signing-key"]);
        apply_filter(&mut state, needle);

        assert_eq!(
            visible_names(&state),
            vec!["api-signing-key".to_string()],
            "a secrets filter must search the {field}: `{needle}` describes \
             the loaded secret, so it must keep it visible"
        );
    }
}

#[test]
fn a_secrets_filter_matching_no_field_hides_every_row() {
    let mut state = loaded(&["api-signing-key", "db-password"]);

    apply_filter(&mut state, "no-such-secret");

    assert!(
        visible_names(&state).is_empty(),
        "a secrets filter matching none of the searched fields must hide \
         every row rather than leave the unnarrowed list on screen"
    );
}

#[test]
fn the_visible_secret_count_counts_exactly_the_rows_the_filter_keeps() {
    let mut state = loaded(&["api-key", "db-key", "tls-cert", "ssh-host"]);
    assert_eq!(state.visible_secret_count(), 4);

    apply_filter(&mut state, "key");

    assert_eq!(
        state.visible_secret_count(),
        2,
        "the visible secret count is what the header reports and what the \
         viewport clamps against: it must count the filtered rows, not zero, \
         one, or the unfiltered list"
    );
}

// ---------------------------------------------------------------------------
// Sort field and detail tab labels
// ---------------------------------------------------------------------------

#[test]
fn every_secret_sort_field_is_labelled_with_the_column_it_sorts() {
    assert_eq!(
        [
            SecretSortField::Name.label(),
            SecretSortField::Created.label(),
            SecretSortField::Replication.label(),
        ],
        ["Name", "Created", "Replication"],
        "the sort-field labels are the header text the operator sorts by; a \
         blank or wrong one leaves the table headed by nothing recognisable"
    );
}

#[test]
fn every_secret_detail_tab_is_labelled_with_the_tab_it_names() {
    assert_eq!(
        SecretDetailTab::ALL.map(SecretDetailTab::label),
        ["Summary", "Versions", "Labels", "JSON"],
        "the detail-tab labels are the tab strip the operator reads to know \
         which tab is open"
    );
}

// ---------------------------------------------------------------------------
// Reloads against an open detail pane and a live selection
// ---------------------------------------------------------------------------

#[test]
fn reloading_secrets_keeps_the_open_detail_when_its_secret_is_still_listed() {
    let mut state = loaded(&["api-signing-key", "db-password"]);
    state.open_secret_detail();
    assert_eq!(
        state
            .secret_detail_target()
            .map(|secret| secret.name.clone()),
        Some("api-signing-key".to_string())
    );

    let dropped = state.set_secrets(ctx(), vec![secret("api-signing-key")]);

    assert_eq!(
        dropped, None,
        "the secret the pane is on is still listed, so the reload must report \
         nothing dropped"
    );
    assert!(
        state.is_showing_detail(),
        "a reload must not close the detail pane out from under the operator \
         while its secret is still there"
    );
    assert_eq!(
        state
            .secret_detail_target()
            .map(|secret| secret.name.clone()),
        Some("api-signing-key".to_string()),
        "the surviving pane must still target the same secret"
    );
}

#[test]
fn reloading_secrets_closes_the_open_detail_and_names_the_secret_that_vanished() {
    let mut state = loaded(&["api-signing-key", "db-password"]);
    state.open_secret_detail();

    let dropped = state.set_secrets(ctx(), vec![secret("db-password")]);

    assert_eq!(
        dropped,
        Some("api-signing-key".to_string()),
        "a reload that no longer lists the pane's secret must report which \
         secret was dropped, since that is what the operator is told"
    );
    assert!(
        !state.is_showing_detail(),
        "the detail pane for a secret that is gone must be closed, not left \
         rendering stale metadata"
    );
    assert_eq!(
        state.secret_detail_target(),
        None,
        "a closed secret detail pane must target nothing"
    );
}

#[test]
fn reloading_secrets_keeps_the_selection_on_the_same_secret() {
    let mut state = loaded(&["api-signing-key", "db-password", "tls-cert"]);
    state.select_next();
    assert_eq!(
        state.selected_secret().map(|secret| secret.name.clone()),
        Some("db-password".to_string())
    );

    state.set_secrets(
        ctx(),
        vec![
            secret("api-signing-key"),
            secret("db-password"),
            secret("tls-cert"),
        ],
    );

    assert_eq!(
        state.selected_secret().map(|secret| secret.name.clone()),
        Some("db-password".to_string()),
        "a reload must leave the cursor on the secret it was on, not jump it \
         to some other row that happens to sit at a different index"
    );
}

// ---------------------------------------------------------------------------
// Problems
// ---------------------------------------------------------------------------

#[test]
fn a_recorded_secrets_problem_is_readable_until_the_next_successful_load() {
    let mut state = loaded(&[]);
    state.set_service_problem(
        SECRETS,
        DiagnosticProblem::new(GcloudErrorKind::PermissionDenied, "denied"),
    );

    let problem = state
        .secrets_problem()
        .expect("the recorded secrets problem must be readable");
    assert_eq!(
        problem.kind,
        GcloudErrorKind::PermissionDenied,
        "the Home overview reads the secrets problem to say why the list is \
         empty; losing it turns a permission failure into a blank panel"
    );
    assert_eq!(problem.message, "denied");

    state.set_secrets(ctx(), vec![secret("api-signing-key")]);

    assert!(
        state.secrets_problem().is_none(),
        "a successful secrets load must clear the previous problem"
    );
}

// ---------------------------------------------------------------------------
// Detail target, error, and close
// ---------------------------------------------------------------------------

#[test]
fn opening_a_secret_detail_targets_the_selected_secret_by_value_and_by_name() {
    let mut state = loaded(&["api-signing-key", "db-password"]);
    state.select_next();

    state.open_secret_detail();

    assert_eq!(
        state
            .secret_detail_target()
            .map(|secret| secret.name.clone()),
        Some("db-password".to_string()),
        "the open pane must target the selected secret, which is the row the \
         detail fetch is issued for"
    );
    assert_eq!(
        state.secret_detail_target_name(),
        Some("db-password"),
        "the pane's title and the arriving-detail match both read the target \
         name; a blank or foreign name mistargets both"
    );
}

#[test]
fn a_failed_secret_describe_is_readable_and_still_names_its_secret() {
    let mut state = loaded(&["api-signing-key"]);

    state.set_secret_detail_error("api-signing-key".to_string(), "denied".to_string());

    assert_eq!(
        state.secret_detail_error(),
        Some("denied"),
        "the failed describe's message is what the detail pane renders in \
         place of the metadata; dropping it leaves an empty pane"
    );
    assert!(
        state.is_showing_detail(),
        "a describe failure must still open the pane, which is where the \
         error is shown"
    );
    assert_eq!(
        state.secret_detail_target_name(),
        Some("api-signing-key"),
        "with no detail loaded the pane still knows which secret failed"
    );
    assert!(
        state.is_active_secret_detail_target("api-signing-key"),
        "a late reply for the failed secret must still be recognised as the \
         one the pane is waiting on"
    );
}

#[test]
fn only_the_secret_the_detail_pane_is_on_counts_as_its_active_target() {
    let mut state = loaded(&["api-signing-key", "db-password"]);
    state.open_secret_detail();

    assert!(
        state.is_active_secret_detail_target("api-signing-key"),
        "the open pane's own secret must be its active target"
    );
    assert!(
        !state.is_active_secret_detail_target("db-password"),
        "a reply for a different secret must not be accepted onto this pane, \
         or a stale fetch would paint the wrong secret's metadata"
    );

    let mut errored = loaded(&["api-signing-key", "db-password"]);
    errored.set_secret_detail_error("api-signing-key".to_string(), "denied".to_string());

    assert!(
        !errored.is_active_secret_detail_target("db-password"),
        "a pane holding one secret's describe failure must not claim another \
         secret's reply"
    );
}

#[test]
fn closing_the_secrets_detail_clears_its_target_and_its_error() {
    let mut state = loaded(&["api-signing-key"]);
    state.open_secret_detail();
    state.set_secret_detail_error("api-signing-key".to_string(), "denied".to_string());
    assert!(state.is_showing_detail());

    state.mark_secrets_stale_and_clear();

    assert!(
        !state.is_showing_detail(),
        "clearing the secrets list must close the pane it was showing"
    );
    assert_eq!(
        state.secret_detail_target(),
        None,
        "a closed pane must hold no target, or the next reply would be \
         accepted onto a pane that is gone"
    );
    assert_eq!(
        state.secret_detail_error(),
        None,
        "a closed pane must hold no error from the context just left behind"
    );
    assert_eq!(
        state.secret_detail_target_name(),
        None,
        "nothing is targeted once the pane is closed"
    );
}

// ---------------------------------------------------------------------------
// The JSON tab's payload
// ---------------------------------------------------------------------------

#[test]
fn the_secret_detail_json_tab_gets_the_describe_response_pretty_printed() {
    let stdout = r#"{"name":"projects/p/secrets/api-signing-key","replication":{"automatic":{}}}"#;

    let detail = parse_secret_detail_json(stdout).expect("the describe response parses");

    assert!(
        detail.raw_json.contains("\n  \"name\": "),
        "the JSON tab shows the describe response indented over several \
         lines; it must not be blank, invented, or the unformatted one-liner \
         gcloud returned — got {:?}",
        detail.raw_json
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&detail.raw_json)
            .expect("the pretty-printed JSON is still JSON"),
        serde_json::from_str::<serde_json::Value>(stdout).expect("the fixture is JSON"),
        "pretty-printing must preserve the response verbatim, not rewrite it"
    );
}
