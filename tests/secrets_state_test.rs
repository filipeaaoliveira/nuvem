//! Secret Manager's slice of `ResourcesState`, asserted directly on state.
//!
//! `tests/secrets_rendering_test.rs` pins what reaches the terminal buffer;
//! this file pins the state transitions behind it — load-phase resolution,
//! freshness, filter editing, selection, sorting and detail-tab movement —
//! at the exact boundary values where an off-by-one or a flipped comparison
//! would otherwise still paint a plausible-looking frame.

use nuvem::app::projection::table::SortDirection;
use nuvem::app::state::resources::{ResourceLoadPhase, ResourcesState};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{ComputeKeyValue, ResourceServiceKind, Secret};
use std::time::Instant;

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

fn other_ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("prod"),
        account: None,
        project: Some("prod-project".into()),
        region: None,
        zone: None,
    }
}

fn secret(name: &str, created: &str) -> Secret {
    Secret {
        name: name.into(),
        create_time: Some(created.into()),
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
    let rows = names
        .iter()
        .enumerate()
        .map(|(index, name)| secret(name, &format!("2026-01-0{}T00:00:00Z", index + 1)))
        .collect();
    state.set_secrets(ctx(), rows);
    state
}

fn selected_name(state: &ResourcesState) -> Option<String> {
    state.selected_secret().map(|secret| secret.name.clone())
}

// ---------------------------------------------------------------------------
// Load phase
// ---------------------------------------------------------------------------

#[test]
fn abandoning_the_secrets_load_resolves_its_phase_and_reports_that_it_did() {
    let mut state = ResourcesState::default();
    state.select_service(SECRETS);
    let ticket = state.begin_load(SECRETS, Instant::now());

    assert!(
        state.abandon_load(ticket),
        "abandoning the load that still owns the Loading phase must report true, \
         which is the answer the runtime reschedules on"
    );
    assert_eq!(
        state.load_phase(SECRETS),
        &ResourceLoadPhase::Idle,
        "abandoning a secrets load must resolve its phase back to Idle"
    );
}

// ---------------------------------------------------------------------------
// Freshness
// ---------------------------------------------------------------------------

#[test]
fn secrets_need_a_refresh_before_any_context_is_loaded() {
    let mut state = ResourcesState::default();
    state.select_service(SECRETS);

    assert!(
        state.needs_refresh(&ctx()),
        "a secrets list that has never loaded must ask for a refresh"
    );
}

#[test]
fn secrets_need_no_refresh_for_the_context_they_were_loaded_for() {
    let state = loaded(&["api-signing-key"]);

    assert!(
        !state.needs_refresh(&ctx()),
        "freshly loaded, non-stale secrets for the very same context must not \
         be refetched"
    );
}

#[test]
fn secrets_need_a_refresh_for_a_context_they_were_not_loaded_for() {
    let state = loaded(&["api-signing-key"]);

    assert!(
        state.needs_refresh(&other_ctx()),
        "secrets loaded under one context say nothing about another context, \
         so a different context must ask for a refresh even while the rows are \
         not stale"
    );
}

// ---------------------------------------------------------------------------
// Selection against a projected row count
// ---------------------------------------------------------------------------

#[test]
fn no_secret_is_selected_when_the_projection_has_zero_visible_rows() {
    let state = loaded(&["api-signing-key"]);

    assert_eq!(
        state.selected_visible_index_for_visible_rows(0),
        None,
        "with no projected rows there is nothing to highlight, so the table \
         must be handed no selected index at all"
    );
}

#[test]
fn the_selected_secret_index_survives_a_single_projected_row() {
    let state = loaded(&["api-signing-key"]);

    assert_eq!(
        state.selected_visible_index_for_visible_rows(1),
        Some(0),
        "one projected row is still a row: the selection must reach the table"
    );
}

// ---------------------------------------------------------------------------
// Sorting
// ---------------------------------------------------------------------------

#[test]
fn sorting_the_column_secrets_are_already_sorted_by_toggles_the_direction() {
    let mut state = loaded(&["api-signing-key", "db-password"]);
    assert_eq!(
        state.sort_direction(),
        SortDirection::Ascending,
        "secrets start sorted by NAME ascending"
    );

    state.sort_selected_column();

    assert_eq!(
        state.sort_direction(),
        SortDirection::Descending,
        "pressing sort on the already-sorted secrets column must reverse it, \
         not re-apply the same ascending sort"
    );
    assert_eq!(
        state.sort_column_index(),
        0,
        "toggling direction must leave the sorted column where it was"
    );
    assert_eq!(
        selected_name(&state).as_deref(),
        Some("db-password"),
        "the reversed order must reach the visible list, not just the field"
    );
}

#[test]
fn sorting_a_different_secrets_column_switches_field_and_starts_ascending() {
    let mut state = loaded(&["api-signing-key", "db-password"]);
    state.sort_selected_column(); // NAME descending
    state.select_next_column(); // CREATED

    state.sort_selected_column();

    assert_eq!(
        state.sort_column_index(),
        1,
        "sorting a newly selected secrets column must move the sort to it"
    );
    assert_eq!(
        state.sort_direction(),
        SortDirection::Ascending,
        "a new sort column starts ascending; it must not inherit the previous \
         column's direction"
    );
}

// ---------------------------------------------------------------------------
// Cursor movement
// ---------------------------------------------------------------------------

#[test]
fn selecting_the_next_secret_moves_the_cursor_down_one_row() {
    let mut state = loaded(&["api-signing-key", "db-password", "tls-cert"]);
    assert_eq!(selected_name(&state).as_deref(), Some("api-signing-key"));

    state.select_next();

    assert_eq!(
        state.selected_visible_index(),
        Some(1),
        "the secrets cursor must advance by exactly one row"
    );
    assert_eq!(
        selected_name(&state).as_deref(),
        Some("db-password"),
        "advancing the cursor must select the next secret in visible order"
    );
}

#[test]
fn selecting_the_next_secret_stops_on_the_last_visible_row() {
    let mut state = loaded(&["api-signing-key", "db-password", "tls-cert"]);

    for _ in 0..5 {
        state.select_next();
    }

    assert_eq!(
        state.selected_visible_index(),
        Some(2),
        "the secrets cursor must clamp to the last visible row rather than \
         running off the end of the list"
    );
    assert_eq!(
        selected_name(&state).as_deref(),
        Some("tls-cert"),
        "a clamped cursor must still resolve to the last secret"
    );
}

#[test]
fn selecting_the_next_secret_on_an_empty_list_selects_nothing() {
    let mut state = loaded(&[]);

    state.select_next();

    assert_eq!(
        state.selected_visible_index(),
        None,
        "with no secrets loaded there is no row to move to, and the empty \
         list must not be treated as one row"
    );
}

// ---------------------------------------------------------------------------
// Filter editing
// ---------------------------------------------------------------------------

#[test]
fn the_secrets_filter_draft_reports_exactly_what_was_typed() {
    let mut state = loaded(&["api-signing-key", "db-password"]);
    state.start_filter();
    for ch in "db".chars() {
        state.append_filter_char(ch);
    }

    assert_eq!(
        state.filter_draft(),
        "db",
        "the secrets filter draft must echo the typed characters, since it is \
         what the filter input paints"
    );
}

#[test]
fn backspacing_the_secrets_filter_drops_the_last_typed_character() {
    let mut state = loaded(&["api-signing-key", "db-password"]);
    state.start_filter();
    for ch in "db".chars() {
        state.append_filter_char(ch);
    }

    state.backspace_filter();

    assert_eq!(
        state.filter_draft(),
        "d",
        "backspace must remove one character from the secrets draft"
    );

    state.backspace_filter();

    assert_eq!(
        state.filter_draft(),
        "",
        "backspacing the last character must empty the secrets draft"
    );
}

#[test]
fn applying_a_secrets_filter_narrows_the_list_and_requests_no_reload() {
    let mut state = loaded(&["api-signing-key", "db-password"]);
    state.start_filter();
    for ch in "db".chars() {
        state.append_filter_char(ch);
    }

    assert_eq!(
        state.apply_filter(),
        None,
        "Secret Manager filters locally, so applying one must ask for no \
         server-side reload"
    );
    assert_eq!(
        state
            .visible_secrets()
            .iter()
            .map(|secret| secret.name.as_str())
            .collect::<Vec<_>>(),
        vec!["db-password"],
        "the applied secrets filter must narrow the visible list"
    );
    assert!(
        !state.is_filter_input_active(),
        "applying a secrets filter must close the filter input"
    );
}

// ---------------------------------------------------------------------------
// Detail tabs
// ---------------------------------------------------------------------------

#[test]
fn stepping_back_from_the_first_secret_detail_tab_wraps_to_the_last() {
    let mut state = loaded(&["api-signing-key"]);
    state.open_secret_detail();
    assert_eq!(state.active_detail_tab_label(), "Summary");

    state.select_previous_tab();

    assert_eq!(
        state.active_detail_tab_label(),
        "JSON",
        "stepping back from the first secret detail tab must wrap around to \
         the last one"
    );
}

#[test]
fn stepping_back_from_a_middle_secret_detail_tab_moves_one_tab_left() {
    let mut state = loaded(&["api-signing-key"]);
    state.open_secret_detail();
    state.select_next_tab(); // Versions
    state.select_next_tab(); // Labels
    assert_eq!(state.active_detail_tab_label(), "Labels");

    state.select_previous_tab();

    assert_eq!(
        state.active_detail_tab_label(),
        "Versions",
        "stepping back must move exactly one secret detail tab to the left"
    );
}

// ---------------------------------------------------------------------------
// Staleness
// ---------------------------------------------------------------------------

#[test]
fn marking_the_active_secrets_service_stale_clears_its_rows() {
    let mut state = loaded(&["api-signing-key", "db-password"]);
    assert_eq!(state.secrets().len(), 2);

    state.mark_active_stale_and_clear(SECRETS);

    assert!(
        state.secrets().is_empty(),
        "marking secrets stale must drop the rows that belonged to the old \
         context rather than leaving them on screen"
    );
    assert!(
        state.is_stale(),
        "the cleared secrets list must report itself stale so the refresh \
         machinery reloads it"
    );
    assert!(
        state.needs_refresh(&ctx()),
        "a stale, cleared secrets list must ask for a refresh of the context \
         it was just showing"
    );
}
