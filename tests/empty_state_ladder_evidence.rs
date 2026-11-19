//! Evidence for the empty-state ladder each resource list renders, and for the
//! zero-area guard the Logging view opens with.
//!
//! Both came out of the weekly mutation sweep (shard 12, 2026-09-01) rather
//! than out of a diff — eleven survivors in code no change had touched, which
//! is exactly what that job exists to find.
//!
//! The ladder is the interesting one. "No GKE clusters found for dev-project"
//! and "Press r to load GKE clusters." are different sentences about different
//! situations — one says the project is empty, the other says nothing has been
//! fetched yet — and they are told apart by a single `&& status == Ready`.
//! Every existing render test loads rows, so the whole ladder below the first
//! rung was unasserted: a mutant could swap the two messages and the suite
//! stayed green while the operator was told their cluster list was empty when
//! it had simply never loaded.

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction, WorkspaceAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    CloudRunService, CloudRunServiceStatus, EndpointVisibility, GkeCluster, GkeClusterStatus,
    ResourceServiceKind,
};
use ratatui::layout::Rect;
use ratatui::{Terminal, backend::TestBackend};

fn terminal_lines(terminal: &Terminal<TestBackend>) -> Vec<String> {
    let buffer = terminal.backend().buffer();
    let width = buffer.area.width as usize;
    buffer
        .content()
        .chunks(width)
        .map(|line| line.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect()
}

/// Built through the reducer with the returned effects DISCARDED, which is
/// what keeps each rung reachable. Driving this through `RuntimeSession`
/// instead would run the load those effects ask for, and every case below
/// would arrive as `Loading` or `Failed` — the two rungs that were already
/// covered.
fn app_on(service: ResourceServiceKind) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: None,
                zone: None,
            }],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(service)),
    );
    app
}

fn context_of(app: &AppState) -> nuvem_domain::gcp::configuration::GcloudContext {
    app.root.current_context().expect("an active context")
}

fn rendered_at(app: AppState, width: u16) -> String {
    let session = RuntimeSession::new_for_test(app);
    let mut terminal = Terminal::new(TestBackend::new(width, 20)).expect("terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render");
    terminal_lines(&terminal).join("\n")
}

/// One cluster, then a filter that hides it — the only state in which the
/// empty-state path renders AND the tools line has something to say
/// (`list_tools_line` returns `None` while `total_rows` is 0).
fn gke_with_everything_filtered_out() -> AppState {
    let mut app = app_on(ResourceServiceKind::GkeClusters);
    one_gke_cluster(&mut app);
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for ch in "zzz-nothing".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));
    app
}

fn one_gke_cluster(app: &mut AppState) {
    let context = context_of(app);
    reduce(
        app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context,
            clusters: vec![GkeCluster {
                name: "prod-cluster".to_string(),
                location: "europe-west1".to_string(),
                status: GkeClusterStatus::Running,
                current_master_version: None,
                current_node_count: Some(3),
                endpoint_visibility: EndpointVisibility::Public,
            }],
        }),
    );
}

fn rendered(app: AppState) -> String {
    let session = RuntimeSession::new_for_test(app);
    let mut terminal = Terminal::new(TestBackend::new(120, 20)).expect("terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render");
    terminal_lines(&terminal).join("\n")
}

// --- The ladder: "none exist" is not "none loaded" ---

/// A load that returned zero clusters means the project has none, and the
/// message names the project so the operator can see which one was asked.
#[test]
fn a_ready_but_empty_gke_list_says_the_project_has_none() {
    let mut app = app_on(ResourceServiceKind::GkeClusters);
    let context = context_of(&app);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context,
            clusters: Vec::new(),
        }),
    );

    let screen = rendered(app);

    assert!(
        screen.contains("No GKE clusters found for dev"),
        "an empty result must name the project it was empty for:\n{screen}"
    );
    assert!(
        !screen.contains("Press r to load"),
        "a completed load must not read as one that never ran:\n{screen}"
    );
}

/// Nothing fetched yet is the other rung, and it must invite the fetch rather
/// than assert an emptiness no load has established.
#[test]
fn a_gke_list_that_never_loaded_invites_the_load() {
    let screen = rendered(app_on(ResourceServiceKind::GkeClusters));

    assert!(
        screen.contains("Press r to load GKE clusters."),
        "an unloaded list must invite the load:\n{screen}"
    );
    assert!(
        !screen.contains("No GKE clusters found"),
        "nothing loaded is not the same as nothing there:\n{screen}"
    );
}

/// The same two rungs on Cloud Run, whose renderer repeats the ladder
/// verbatim — the sweep found the identical four survivors in both.
#[test]
fn a_ready_but_empty_cloud_run_list_says_the_project_has_none() {
    let mut app = app_on(ResourceServiceKind::CloudRunServices);
    let context = context_of(&app);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServicesLoaded {
            context,
            services: Vec::new(),
        }),
    );

    let screen = rendered(app);

    assert!(
        screen.contains("No Cloud Run services found for dev"),
        "{screen}"
    );
    assert!(!screen.contains("Press r to load"), "{screen}");
}

#[test]
fn a_cloud_run_list_that_never_loaded_invites_the_load() {
    let screen = rendered(app_on(ResourceServiceKind::CloudRunServices));

    assert!(
        screen.contains("Press r to load Cloud Run services."),
        "{screen}"
    );
    assert!(!screen.contains("No Cloud Run services found"), "{screen}");
}

/// The third rung: rows exist but the filter hides them all. Distinct from
/// both of the above, because the remedy is to change the filter rather than
/// to load or to look elsewhere.
#[test]
fn a_filter_that_matches_nothing_says_so_and_quotes_itself() {
    let mut app = app_on(ResourceServiceKind::GkeClusters);
    let context = context_of(&app);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context,
            clusters: vec![GkeCluster {
                name: "prod-cluster".to_string(),
                location: "europe-west1".to_string(),
                status: GkeClusterStatus::Running,
                current_master_version: None,
                current_node_count: Some(3),
                endpoint_visibility: EndpointVisibility::Public,
            }],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for ch in "zzz-nothing".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    let screen = rendered(app);

    assert!(
        screen.contains("match filter") && screen.contains("zzz-nothing"),
        "a filter that hides every row must say so and quote itself:\n{screen}"
    );
    assert!(
        !screen.contains("No GKE clusters found for"),
        "a hidden row is not an absent one:\n{screen}"
    );
}

// --- The zero-area guard ---

/// A degenerate area must produce no output and no panic.
///
/// Read what this does NOT do. `ui::logging::render` opens with
/// `if area.width == 0 || area.height == 0`, and the sweep reported that
/// `||`→`&&` survives here. It survives this test too, deliberately: measured
/// with the mutation applied — with an empty view and with entries loaded —
/// ratatui clips a zero-extent rect rather than panicking or writing, so the
/// guard saves work and changes nothing observable. There is no assertion that
/// could tell the two apart, which makes it an equivalent mutant rather than a
/// coverage gap, and writing a test that appeared to pin it would be worse
/// than leaving it unpinned.
///
/// What this is worth keeping for is the other direction: if a future change
/// makes zero-extent rendering panic, the guard becomes load-bearing and this
/// notices.
#[test]
fn the_logging_view_draws_nothing_into_a_degenerate_area() {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Logging;
    let session = RuntimeSession::new_for_test(app);

    for area in [
        Rect::new(0, 0, 0, 20),
        Rect::new(0, 0, 120, 0),
        Rect::new(0, 0, 0, 0),
    ] {
        let mut terminal = Terminal::new(TestBackend::new(120, 20)).expect("terminal");
        terminal
            .draw(|frame| nuvem::ui::logging::render(frame, area, &session))
            .unwrap_or_else(|error| panic!("rendering into {area:?} must not fail: {error}"));

        let screen = terminal_lines(&terminal).join("");
        assert!(
            screen.trim().is_empty(),
            "nothing may be drawn for {area:?}:\n{screen}"
        );
    }
}

/// The Cloud Run filter rung, which the GKE case above does not cover: the
/// two renderers repeat the ladder verbatim, and the sweep found the same
/// survivor in each, so each needs its own assertion.
/// The Cloud Run twin of [`gke_with_everything_filtered_out`].
fn cloud_run_with_everything_filtered_out() -> AppState {
    let mut app = app_on(ResourceServiceKind::CloudRunServices);
    let context = context_of(&app);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServicesLoaded {
            context,
            services: vec![CloudRunService {
                name: "checkout".to_string(),
                region: "europe-west1".to_string(),
                url: None,
                latest_ready_revision: None,
                latest_created_revision: None,
                traffic_summary: "100% latest".to_string(),
                status: CloudRunServiceStatus::Ready,
                ready_message: None,
            }],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for ch in "zzz-nothing".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));
    app
}

#[test]
fn a_cloud_run_filter_that_matches_nothing_says_so() {
    let screen = rendered(cloud_run_with_everything_filtered_out());

    assert!(
        screen.contains("match filter") && screen.contains("zzz-nothing"),
        "{screen}"
    );
    assert!(
        !screen.contains("No Cloud Run services found for"),
        "{screen}"
    );
}

/// The same width gate on Cloud Run. Both renderers carry their own copy of
/// the line, so a test on one says nothing about the other — the sweep found
/// the survivor in each.
#[test]
fn the_cloud_run_tools_line_is_spent_only_on_a_wide_enough_terminal() {
    assert!(
        has_a_tools_row(&rendered_at(cloud_run_with_everything_filtered_out(), 120)),
        "a wide terminal shows the tools line"
    );
    assert!(
        !has_a_tools_row(&rendered_at(cloud_run_with_everything_filtered_out(), 70)),
        "a narrow one spends those columns on the message instead"
    );
}

/// Whether the screen spends a row of its own on the tools line, rather than
/// carrying it in the list's border title. The narrow layout keeps the line
/// in the title, row count first, so the count alone cannot tell the two
/// layouts apart.
fn has_a_tools_row(screen: &str) -> bool {
    screen
        .lines()
        .any(|line| line.contains("Filter: zzz-nothing") && !line.contains("┌Resources /"))
}

/// The tools line — `Filter: … Rows: n/n Sort: …` — is spent only where there
/// are columns for it. `(!compact)` is what decides that, and compact is the
/// narrow layout the sidebar collapses into at 80 columns or fewer.
///
/// The state matters as much as the widths: this is the *empty-state* path, so
/// it needs rows that exist but are all filtered away. With rows visible the
/// table renderer runs instead and never reaches this line, and with no rows
/// at all `list_tools_line` returns `None` and the flag decides nothing.
///
/// Asserted from both sides, because one width cannot tell `!compact` from
/// `compact`: at 120 the line is part of how an operator reads the list, and
/// at 70 the columns are needed for the rows themselves.
#[test]
fn the_list_tools_line_is_spent_only_on_a_wide_enough_terminal() {
    assert!(
        has_a_tools_row(&rendered_at(gke_with_everything_filtered_out(), 120)),
        "a wide terminal shows the tools line"
    );
    let narrow = rendered_at(gke_with_everything_filtered_out(), 70);
    assert!(
        !has_a_tools_row(&narrow),
        "a narrow one spends those columns on the message instead"
    );
    assert!(
        narrow.contains("Rows: 0/1"),
        "and its border title still carries the whole count:\n{narrow}"
    );
}
