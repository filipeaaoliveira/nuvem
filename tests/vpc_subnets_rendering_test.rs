//! The VPC subnets table reaches the screen (slice 2).
//!
//! Asserted on the terminal buffer through the real reducer and renderer, for
//! the reason the networks rendering test records: a service can satisfy every
//! `ResourceServiceKind` walk, hold rows, and still paint nothing.
//!
//! Two claims carry this file, and both come from one fact — a subnet's name
//! does not identify it. `REGION` must reach the screen on every row, and the
//! preview must say which region it is describing.

use nuvem::app::projection::columns::{VPC_SUBNET_COLUMNS, catalog_meta};
use nuvem::app::projection::project_resource_preview;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::command::CommandDescriptor;
use nuvem::app::state::root::Workspace;
use nuvem::app::{
    Action, AppError, AppState, CommandAction, Effect, EffectResult, ResourcesAction,
};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::resource::{
    ComputeInstance, ResourceServiceKind, ResourceStatus, VpcSubnet,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

const FRAME_WIDTH: u16 = 200;
const FRAME_HEIGHT: u16 = 40;

fn configuration() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

/// The captured shape — three rows called `default` in three regions — plus
/// one row that differs in every optional cell, so no column of this table is
/// the same on every row.
fn subnets() -> Vec<VpcSubnet> {
    vec![
        VpcSubnet {
            name: "default".to_string(),
            region: "us-central1".to_string(),
            network: "default".to_string(),
            ip_cidr_range: "10.128.0.0/20".to_string(),
            gateway_address: Some("10.128.0.1".to_string()),
            private_google_access: false,
        },
        VpcSubnet {
            name: "default".to_string(),
            region: "europe-west1".to_string(),
            network: "default".to_string(),
            ip_cidr_range: "10.132.0.0/20".to_string(),
            gateway_address: Some("10.132.0.1".to_string()),
            private_google_access: false,
        },
        // Constructed: a custom subnet with private Google access on and no
        // gateway reported. This account can reach no such subnet, so the
        // `On` and the dashed gateway rest on the published
        // `compute/v1/subnetworks` reference rather than on a capture.
        VpcSubnet {
            name: "prod-app".to_string(),
            region: "us-central1".to_string(),
            network: "prod-vpc".to_string(),
            ip_cidr_range: "192.168.0.0/24".to_string(),
            gateway_address: None,
            private_google_access: true,
        },
    ]
}

fn app_with_context() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration()],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::VpcSubnets,
        )),
    );
    app.root.workspace = Workspace::Resources;
    app
}

fn app_showing_subnets() -> AppState {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::VpcSubnetsLoaded {
            context,
            subnets: subnets(),
        }),
    );
    app
}

fn painted(app: AppState) -> String {
    let session = RuntimeSession::new_for_test(app);
    let backend = TestBackend::new(FRAME_WIDTH, FRAME_HEIGHT);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            let mut line = String::new();
            for x in 0..buffer.area.width {
                line.push_str(buffer.cell((x, y)).expect("cell").symbol());
            }
            line.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The rows of the table pane alone — the preview beside it shows the selected
/// subnet's own values, so a `contains` over the whole line would pass on a
/// table that painted nothing.
fn table_lines(painted: &str) -> Vec<&str> {
    painted
        .lines()
        .filter_map(|line| line.split("││").nth(1))
        .collect()
}

fn table_text(painted: &str) -> String {
    table_lines(painted).join("\n")
}

// --- The catalogue ---

#[test]
fn the_catalogue_is_the_six_agreed_columns_in_order() {
    let keys = VPC_SUBNET_COLUMNS
        .iter()
        .map(|column| column.spec.key)
        .collect::<Vec<_>>();

    assert_eq!(
        keys,
        vec![
            "name",
            "region",
            "network",
            "range",
            "gateway",
            "private_google_access"
        ]
    );
}

/// `REGION` is the second-most protected column, ahead of everything but the
/// identity.
///
/// It is half the identity: with every row called `default`, a table that
/// dropped `REGION` on a narrow terminal would paint rows an operator cannot
/// tell apart. Asserting the priority rather than the position is what pins
/// the *drop order*, which is the property that matters.
#[test]
fn region_outranks_every_column_but_the_name() {
    let priorities: Vec<(&str, u8)> = VPC_SUBNET_COLUMNS
        .iter()
        .map(|column| (column.spec.key, column.spec.priority))
        .collect();

    let region = priorities
        .iter()
        .find(|(key, _)| *key == "region")
        .expect("the catalogue has a region column")
        .1;
    let name = priorities
        .iter()
        .find(|(key, _)| *key == "name")
        .expect("the catalogue has a name column")
        .1;

    assert!(
        name < region,
        "the identity column is protected above all others"
    );
    for (key, priority) in &priorities {
        if *key == "name" || *key == "region" {
            continue;
        }
        assert!(
            region < *priority,
            "REGION must outrank {key:?}: it is half the row's identity, and a \
             narrow terminal that dropped it would paint rows nobody can tell \
             apart"
        );
    }
}

#[test]
fn every_column_sorts_because_there_is_no_labels_cell() {
    let unsortable = catalog_meta(ResourceServiceKind::VpcSubnets)
        .iter()
        .filter(|column| !column.sortable)
        .map(|column| column.key)
        .collect::<Vec<_>>();

    assert!(unsortable.is_empty(), "got {unsortable:?}");
}

// --- The table ---

#[test]
fn the_table_heads_the_six_columns() {
    let table = table_text(&painted(app_showing_subnets()));

    for header in ["NAME", "REGION", "NETWORK", "RANGE", "GATEWAY", "PGA"] {
        assert!(
            table.contains(header),
            "the table must head {header}, got:\n{table}"
        );
    }
}

/// Every region reaches the screen, which is the only way to tell the rows
/// apart.
#[test]
fn every_regions_row_is_painted_with_its_region() {
    let table = table_text(&painted(app_showing_subnets()));

    for region in ["us-central1", "europe-west1"] {
        assert!(
            table.contains(region),
            "the table must paint {region}, or two rows called `default` are \
             indistinguishable:\n{table}"
        );
    }
}

/// The cells are ids, never the self-link URLs the API sent.
#[test]
fn no_cell_paints_a_self_link_url() {
    let table = table_text(&painted(app_showing_subnets()));

    assert!(
        !table.contains("googleapis.com") && !table.contains("compute/v1"),
        "REGION and NETWORK paint ids, not self-links:\n{table}"
    );
}

/// `On`/`Off`, never a bool's spelling and never a dash for `false`.
#[test]
fn private_google_access_paints_a_word_and_never_a_dash() {
    let painted = painted(app_showing_subnets());
    let table = table_text(&painted);

    assert!(
        table.contains("Off"),
        "a subnet without private Google access reads Off:\n{table}"
    );
    assert!(table.contains("On"), "and one with it reads On:\n{table}");
    assert!(
        !table.contains("true") && !table.contains("false"),
        "the cell renders a word, not a bool's spelling:\n{table}"
    );
}

// --- The preview ---

/// The preview names the region in its title, because the name does not
/// identify the row it is describing.
#[test]
fn the_preview_title_carries_the_region_beside_the_name() {
    let app = app_showing_subnets();

    let preview = project_resource_preview(&app.root.resources).expect("a row is selected");

    assert_eq!(
        preview.title, "default (europe-west1)",
        "a pane headed `default` over a table of rows called `default` says \
         nothing about which one"
    );
    assert!(
        preview
            .lines
            .contains(&("Private Google access".to_string(), "Off".to_string())),
        "the pane spells the flag out where the column abbreviates it: {:?}",
        preview.lines
    );
}

/// An absent gateway dashes in the pane rather than painting an empty line.
#[test]
fn the_preview_dashes_an_absent_gateway() {
    let mut app = app_showing_subnets();
    // Ordering is (name, region): default/europe-west1, default/us-central1,
    // prod-app/us-central1. Two steps reach the constructed row.
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));

    let preview = project_resource_preview(&app.root.resources).expect("a row is selected");

    assert_eq!(preview.title, "prod-app (us-central1)");
    assert!(
        preview
            .lines
            .contains(&("Gateway".to_string(), "-".to_string())),
        "an absent gateway dashes: {:?}",
        preview.lines
    );
}

// --- Empty states ---

#[test]
fn a_project_with_no_subnets_says_so() {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::VpcSubnetsLoaded {
            context,
            subnets: Vec::new(),
        }),
    );

    let painted = painted(app);

    assert!(
        painted.contains("No VPC subnets found for"),
        "an empty project names itself:\n{painted}"
    );
}

// --- The palette guard ---

/// The log entry is offered only with a row actually under the cursor.
///
/// The guard is a conjunction — right service, context resolved, row selected
/// — and each `&&` survived being turned into `||`, because every existing
/// assertion drives the state where all three are true. An entry offered with
/// no selection resolves to nothing when pressed: it is a palette row that
/// does nothing, which is worse than one that is absent.
///
/// The `L` keybinding takes the same route, so this covers both.
#[test]
fn the_log_entry_is_not_offered_without_a_row_under_the_cursor() {
    let with_rows = {
        let mut app = app_showing_subnets();
        reduce(&mut app, &Action::Command(CommandAction::Open));
        app.root.command.visible_commands(&app.root).contains(
            &CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::VpcSubnets),
        )
    };
    assert!(
        with_rows,
        "precondition: with a row selected the entry is on offer"
    );

    // The same service, the same context, an empty table.
    let mut empty = app_with_context();
    let context = empty.root.current_context().expect("active context");
    reduce(
        &mut empty,
        &Action::EffectResult(EffectResult::VpcSubnetsLoaded {
            context,
            subnets: Vec::new(),
        }),
    );
    reduce(&mut empty, &Action::Command(CommandAction::Open));

    assert!(
        !empty.root.command.visible_commands(&empty.root).contains(
            &CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::VpcSubnets,)
        ),
        "with no subnet selected there is nothing to open logs for, so the \
         entry must be absent rather than inert"
    );
}

// --- Context guarding ---

/// A listing that resolved for a configuration the operator has since left is
/// dropped, on both the success and the failure path.
///
/// Both halves are asserted together because the failure half is the one that
/// regresses silently: an inverted success guard stops rows appearing and is
/// loud, while an inverted failure guard leaves a correct-looking table that
/// empties the next time some unrelated project's load is refused. This is the
/// **fourth** thin seam of the per-service shape — the other three were
/// written up front for this slice, and this one was not.
#[test]
fn a_result_for_a_context_the_operator_has_left_is_dropped_on_both_paths() {
    let other = GcloudContext {
        configuration: GcloudConfigurationName::new("prod"),
        account: Some("dev@example.com".to_string()),
        project: Some("prod-project".to_string()),
        region: None,
        zone: None,
    };

    let mut app = app_showing_subnets();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::VpcSubnetsLoaded {
            context: other.clone(),
            subnets: vec![VpcSubnet {
                name: "prod-only".to_string(),
                region: "us-west1".to_string(),
                network: "prod-vpc".to_string(),
                ip_cidr_range: "172.16.0.0/16".to_string(),
                gateway_address: None,
                private_google_access: true,
            }],
        }),
    );
    let names: Vec<&str> = app
        .root
        .resources
        .vpc_subnets()
        .iter()
        .map(|subnet| subnet.name.as_str())
        .collect();
    assert!(
        !names.contains(&"prod-only"),
        "another configuration's subnets must not land here: {names:?}"
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::VpcSubnetsLoadFailed {
            context: other,
            error: AppError::gcloud("PERMISSION_DENIED on prod-project".to_string()),
        }),
    );
    assert_eq!(
        app.root.resources.vpc_subnets().len(),
        3,
        "another configuration's refusal must not clear this table"
    );
    assert!(
        app.root.resources.vpc_subnets_problem().is_none(),
        "nor become this table's problem"
    );
}

/// The empty state keeps the tools line, which is the only place the hidden
/// rows are counted.
#[test]
fn a_filter_matching_nothing_says_so_and_still_counts_the_rows() {
    let mut app = app_showing_subnets();
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for character in "zzz-no-such-subnet".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(character)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    let painted = painted(app);

    assert!(
        painted.contains("No subnets match filter"),
        "a filter with no matches says so:\n{painted}"
    );
    assert!(
        painted.contains("Rows: 0/3"),
        "the tools line still counts what the filter hid:\n{painted}"
    );
}

// --- Filtering does not re-fetch ---

/// Applying a `/` filter emits **no effect** for this table, and the rows stay
/// where they are.
///
/// The networks table's finding, one service later, and the reason is the same
/// one: `load_subnets` ignores the filter it is handed, because REGION and
/// NETWORK are the tails of self-link URLs and PGA renders a bool as a word,
/// so nothing here is a verbatim substring of a raw field. A refresh on every
/// Enter would re-download a document already in state — and a failed refresh
/// runs `mark_active_stale_and_clear`, so a transient error on a call that was
/// never needed blanks the table the operator is reading.
///
/// **Paired with a service that does re-fetch**, because a reducer mutated to
/// return `Vec::new()` unconditionally satisfies the negative half alone and
/// would silently stop every table narrowing server-side. Compute pushes
/// `--filter=` down to gcloud, so its arm of the same match must still emit.
#[test]
fn applying_a_filter_refetches_the_pushdown_services_and_not_this_one() {
    let mut subnets = app_showing_subnets();
    reduce(
        &mut subnets,
        &Action::Resources(ResourcesAction::StartFilter),
    );
    for character in "europe".chars() {
        reduce(
            &mut subnets,
            &Action::Resources(ResourcesAction::AppendFilterChar(character)),
        );
    }
    let before = subnets.root.resources.vpc_subnets().len();
    let effects = reduce(
        &mut subnets,
        &Action::Resources(ResourcesAction::ApplyFilter),
    );

    assert!(
        effects.is_empty(),
        "a client-side narrowing must not spawn a gcloud process, and must not \
         put the visible table at the mercy of that call failing: {effects:?}"
    );
    assert!(before > 0, "precondition: there were rows to keep");
    assert_eq!(
        subnets.root.resources.vpc_subnets().len(),
        before,
        "the rows are still in state; only what is visible changed"
    );

    // The positive half.
    let mut compute = app_with_context();
    reduce(
        &mut compute,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::ComputeInstances,
        )),
    );
    let context = compute.root.current_context().expect("active context");
    reduce(
        &mut compute,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![ComputeInstance {
                id: "api".to_string(),
                name: "api".to_string(),
                zone: "europe-west1-b".to_string(),
                status: ResourceStatus::Running,
                machine_type: "e2-medium".to_string(),
                internal_ip: None,
                external_ip: None,
            }],
        }),
    );
    reduce(
        &mut compute,
        &Action::Resources(ResourcesAction::StartFilter),
    );
    for character in "api".chars() {
        reduce(
            &mut compute,
            &Action::Resources(ResourcesAction::AppendFilterChar(character)),
        );
    }
    let compute_effects = reduce(
        &mut compute,
        &Action::Resources(ResourcesAction::ApplyFilter),
    );

    assert!(
        compute_effects
            .iter()
            .any(|effect| matches!(effect, Effect::RefreshComputeInstances { .. })),
        "a service that pushes its filter down must still re-fetch: \
         {compute_effects:?}"
    );
}

/// Pressing `r` still re-fetches, which is the whole difference between the
/// two paths.
#[test]
fn an_explicit_refresh_still_refetches_this_table() {
    let mut app = app_showing_subnets();

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));

    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::RefreshVpcSubnets { .. })),
        "`r` is an explicit ask for fresh rows and must reach gcloud: {effects:?}"
    );
}
