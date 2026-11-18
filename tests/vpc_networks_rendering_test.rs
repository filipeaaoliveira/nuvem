//! The VPC networks table reaches the screen, and says the right thing on it
//! (slice 1).
//!
//! A service can be in `SERVICE_REGISTRY`, satisfy every `ResourceServiceKind`
//! walk, hold rows in state, and still never paint one — every one of those
//! tests passes and the tab is blank. So the assertions here are made on the
//! terminal buffer, through the real reducer and the real renderer, not on the
//! projection model.
//!
//! Two claims carry this file. The first is that `MODE` and `ROUTING` are the
//! *derived* values and not gcloud's synthesised `x_gcloud_*` strings — pinned
//! at the parser in `vpc_service_test`, and pinned again here on the cell an
//! operator actually reads. The second is that a network with no subnets
//! paints `0` and not a dash: zero is an answer the API gave, and a
//! custom-mode network with no subnets is a finding rather than a missing
//! value.

use nuvem::app::projection::columns::{VPC_NETWORK_COLUMNS, catalog_meta};
use nuvem::app::projection::table::{Cell, CellAlign, CellKind};
use nuvem::app::projection::{project_resource_preview, project_resources};
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, AppError, AppState, Effect, EffectResult, ResourcesAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::resource::{
    ComputeInstance, ResourceServiceKind, ResourceStatus, VpcNetwork, VpcRoutingMode, VpcSubnetMode,
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

/// Four networks, chosen so that no cell of this table is the same on every
/// row. A fixture whose rows all said `AUTO`/`REGIONAL` would pass a
/// projection that ignored both fields and hardcoded the strings, which is
/// exactly the shortcut `x_gcloud_subnet_mode` invites.
///
/// The `default` row is the captured one (`tests/fixtures/vpc_networks.json`),
/// down to its 43 subnets. The other three are constructed: this account can
/// reach no custom-mode network, no global-routing network and no legacy
/// network at all.
fn networks() -> Vec<VpcNetwork> {
    vec![
        VpcNetwork {
            name: "default".to_string(),
            subnet_mode: VpcSubnetMode::Auto,
            routing_mode: VpcRoutingMode::Regional,
            subnet_count: 43,
            description: Some("Default network for the project".to_string()),
        },
        VpcNetwork {
            name: "prod-vpc".to_string(),
            subnet_mode: VpcSubnetMode::Custom,
            routing_mode: VpcRoutingMode::Global,
            subnet_count: 7,
            description: None,
        },
        // The finding this table exists to make visible: a custom network
        // nobody ever added a subnet to.
        VpcNetwork {
            name: "empty-vpc".to_string(),
            subnet_mode: VpcSubnetMode::Custom,
            routing_mode: VpcRoutingMode::Regional,
            subnet_count: 0,
            description: None,
        },
        VpcNetwork {
            name: "legacy-net".to_string(),
            subnet_mode: VpcSubnetMode::Legacy,
            routing_mode: VpcRoutingMode::Unspecified,
            subnet_count: 0,
            description: None,
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
            ResourceServiceKind::VpcNetworks,
        )),
    );
    app.root.workspace = Workspace::Resources;
    app
}

fn app_showing_networks() -> AppState {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::VpcNetworksLoaded {
            context,
            networks: networks(),
        }),
    );
    app
}

/// The fixture networks with `filter` applied through the real filter actions.
fn app_with_filter(filter: &str) -> AppState {
    let mut app = app_showing_networks();
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for character in filter.chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(character)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));
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

/// The rows of the table pane alone.
///
/// The navigator, the table and the preview are painted side by side, so one
/// buffer line carries a slice of all three — and the preview shows the
/// selected network's own name and mode. A `contains` over the whole line
/// therefore passes on a table that paints nothing at all, which is the
/// failure these tests exist to catch.
fn table_lines(painted: &str) -> Vec<&str> {
    painted
        .lines()
        .filter_map(|line| line.split("││").nth(1))
        .collect()
}

fn table_text(painted: &str) -> String {
    table_lines(painted).join("\n")
}

/// Sorts by `SUBNETS`, through the real column-cursor actions rather than by
/// setting the sort field directly: `subnets` is catalogue index 3, and the
/// cursor starts on the identity column.
fn sort_by_subnets(app: &mut AppState) {
    while app.root.resources.selected_column() != 3 {
        reduce(app, &Action::Resources(ResourcesAction::SelectNextColumn));
    }
    reduce(
        app,
        &Action::Resources(ResourcesAction::SortBySelectedColumn),
    );
}

fn row_containing<'a>(table: &'a str, needle: &str) -> &'a str {
    table
        .lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("the table paints a {needle} row:\n{table}"))
}

// --- The catalogue ---

#[test]
fn the_catalogue_is_the_four_agreed_columns_in_order() {
    let keys = VPC_NETWORK_COLUMNS
        .iter()
        .map(|column| column.spec.key)
        .collect::<Vec<_>>();

    assert_eq!(keys, vec!["name", "mode", "routing", "subnets"]);
}

/// The ticket asked for a `gateway` column. There is no gateway on a network:
/// `gatewayAddress` is a subnet field. This is the pin that keeps the ticket's
/// wrong parenthetical from being "fixed" back in.
#[test]
fn the_catalogue_has_no_gateway_column() {
    let keys = VPC_NETWORK_COLUMNS
        .iter()
        .map(|column| column.spec.key)
        .collect::<Vec<_>>();

    assert!(
        !keys.iter().any(|key| key.contains("gateway")),
        "a network has no gateway address; that is a subnet field: {keys:?}"
    );
}

/// Every column sorts, which is what distinguishes this table from every other
/// one modelled on the topics table: there is no labels cell, because the
/// `compute/v1` networks resource has no `labels` field at all.
#[test]
fn every_column_sorts_because_there_is_no_labels_cell() {
    let unsortable = catalog_meta(ResourceServiceKind::VpcNetworks)
        .iter()
        .filter(|column| !column.sortable)
        .map(|column| column.key)
        .collect::<Vec<_>>();

    assert!(
        unsortable.is_empty(),
        "no networks column should be unsortable, got {unsortable:?}"
    );
}

/// The count is the one right-aligned cell: digits in a column read as a
/// column of numbers only when their units line up.
#[test]
fn only_the_subnet_count_is_right_aligned() {
    let right = VPC_NETWORK_COLUMNS
        .iter()
        .filter(|column| matches!(column.spec.align, CellAlign::Right))
        .map(|column| column.spec.key)
        .collect::<Vec<_>>();

    assert_eq!(right, vec!["subnets"]);
}

// --- The table ---

#[test]
fn the_networks_table_paints_a_row_per_network() {
    let table = table_text(&painted(app_showing_networks()));

    for name in ["default", "prod-vpc", "empty-vpc", "legacy-net"] {
        assert!(
            table.contains(name),
            "the table must paint {name}, got:\n{table}"
        );
    }
}

#[test]
fn the_table_heads_the_four_columns() {
    let table = table_text(&painted(app_showing_networks()));

    for header in ["NAME", "MODE", "ROUTING", "SUBNETS"] {
        assert!(
            table.contains(header),
            "the table must head {header}, got:\n{table}"
        );
    }
}

/// The two derived cells, on rows that disagree about both.
///
/// A projection that hardcoded either string, or that read one field for both
/// columns, paints these two rows identically and fails here.
#[test]
fn mode_and_routing_paint_the_derived_values_row_by_row() {
    let table = table_text(&painted(app_showing_networks()));

    let default = row_containing(&table, "default");
    assert!(
        default.contains("AUTO") && default.contains("REGIONAL"),
        "the captured auto/regional network must paint both:\n{default}"
    );

    let prod = row_containing(&table, "prod-vpc");
    assert!(
        prod.contains("CUSTOM") && prod.contains("GLOBAL"),
        "a custom/global network must paint both:\n{prod}"
    );

    let legacy = row_containing(&table, "legacy-net");
    assert!(
        legacy.contains("LEGACY"),
        "a network with no autoCreateSubnetworks field is LEGACY, not CUSTOM:\n{legacy}"
    );
    assert!(
        !legacy.contains("REGIONAL") && !legacy.contains("GLOBAL"),
        "an unreported routing mode must not be given the API's default:\n{legacy}"
    );
}

/// Zero subnets is an answer, not a missing value.
///
/// The API listed this network's subnets and found none. Painting a dash would
/// say "nuvem could not tell", and the custom-mode network nobody added a
/// subnet to is precisely the row an operator opens this table to find.
#[test]
fn a_network_with_no_subnets_paints_zero_and_not_a_dash() {
    let table = table_text(&painted(app_showing_networks()));

    let empty = row_containing(&table, "empty-vpc");
    assert!(
        empty.contains('0'),
        "an empty custom network must paint its 0:\n{empty}"
    );
}

#[test]
fn the_captured_networks_subnet_count_paints_its_digits() {
    let table = table_text(&painted(app_showing_networks()));

    let default = row_containing(&table, "default");
    assert!(
        default.contains("43"),
        "the count is the length of the subnetworks array:\n{default}"
    );
}

/// The dash is a *missing* cell, not the empty string.
///
/// A buffer assertion cannot tell those apart — both paint as blank space
/// between two column separators — so this reads the cell out of the table
/// model the renderer consumes. `Missing` is what carries the dash and its
/// dimmed style; an empty `Plain` cell would paint nothing at all, and the
/// column an operator is scanning for "which networks have no routing config"
/// would go silent.
#[test]
fn an_unreported_routing_mode_is_a_missing_cell_and_not_an_empty_one() {
    let app = app_showing_networks();

    let projection = project_resources(&app.root.resources);
    let model = projection.table.as_ref().expect("the table model is built");
    let routing = model
        .columns
        .iter()
        .position(|column| column.key == "routing")
        .expect("the table shows its ROUTING column");

    let cell_for = |name: &str| {
        let index = app
            .root
            .resources
            .visible_vpc_networks()
            .into_iter()
            .position(|network| network.name == name)
            .unwrap_or_else(|| panic!("{name} is visible"));
        model.rows[index].cells[routing].clone()
    };

    assert_eq!(
        cell_for("legacy-net"),
        Cell::missing(),
        "a network with no routingConfig paints the dash"
    );
    assert_eq!(
        cell_for("default").kind,
        CellKind::Plain,
        "a reported routing mode is an ordinary cell"
    );
    assert_eq!(cell_for("default").text, "REGIONAL");
}

// --- Narrowing ---

/// The mode and routing labels are searchable, which is the half of the filter
/// contract that is easy to get wrong: neither string exists in any raw field
/// the row was built from, so a searchable set assembled from the wire shape
/// rather than the cells would hide rows an operator can read.
#[test]
fn the_filter_searches_the_rendered_mode_and_routing_labels() {
    let table = table_text(&painted(app_with_filter("CUSTOM")));

    assert!(table.contains("prod-vpc"), "CUSTOM must match:\n{table}");
    assert!(table.contains("empty-vpc"), "CUSTOM must match:\n{table}");
    assert!(
        !table.contains("legacy-net"),
        "a LEGACY network is not CUSTOM:\n{table}"
    );

    let global = table_text(&painted(app_with_filter("GLOBAL")));
    assert!(global.contains("prod-vpc"), "GLOBAL must match:\n{global}");
    assert!(
        !global.contains("empty-vpc"),
        "a REGIONAL network is not GLOBAL:\n{global}"
    );
}

/// The description has no column and is searchable anyway: it is the only
/// field that says why a network exists, and searchable-without-a-column can
/// only widen what matches.
#[test]
fn the_filter_searches_the_description_that_has_no_column() {
    let table = table_text(&painted(app_with_filter("Default network for")));

    assert!(
        table.contains("default"),
        "the description must be searchable:\n{table}"
    );
    assert!(
        !table.contains("prod-vpc"),
        "only the described network matches:\n{table}"
    );
}

// --- Sorting ---

/// The count sorts numerically, not by its rendered digits.
///
/// `43` before `7` is what a text ordering produces, and it is backwards in
/// the column an operator sorted to find the widest network.
#[test]
fn the_subnet_count_sorts_numerically_and_not_as_text() {
    let mut app = app_showing_networks();
    sort_by_subnets(&mut app);

    let order = app
        .root
        .resources
        .visible_vpc_networks()
        .into_iter()
        .map(|network| network.subnet_count)
        .collect::<Vec<_>>();

    assert_eq!(
        order,
        vec![0, 0, 7, 43],
        "0, 0, 7, 43 ascending — a text sort would put 43 before 7"
    );
}

/// The name breaks every tie, so the two zero-subnet rows have one order and
/// not an arbitrary one.
#[test]
fn the_name_breaks_a_tie_in_the_sorted_column() {
    let mut app = app_showing_networks();
    sort_by_subnets(&mut app);

    let tied = app
        .root
        .resources
        .visible_vpc_networks()
        .into_iter()
        .filter(|network| network.subnet_count == 0)
        .map(|network| network.name.clone())
        .collect::<Vec<_>>();

    assert_eq!(
        tied,
        vec!["empty-vpc".to_string(), "legacy-net".to_string()]
    );
}

/// A second press reverses the order rather than re-sorting ascending.
#[test]
fn pressing_the_same_column_twice_reverses_the_order() {
    let mut app = app_showing_networks();
    sort_by_subnets(&mut app);
    sort_by_subnets(&mut app);

    let order = app
        .root
        .resources
        .visible_vpc_networks()
        .into_iter()
        .map(|network| network.subnet_count)
        .collect::<Vec<_>>();

    assert_eq!(order, vec![43, 7, 0, 0]);
}

// --- The preview ---

/// The preview pane carries the description, which is the reason it is not a
/// column: unbounded free text with the width to render it.
#[test]
fn the_preview_names_the_mode_the_routing_the_count_and_the_description() {
    let app = app_showing_networks();

    let preview = project_resource_preview(&app.root.resources).expect("a row is selected");

    assert_eq!(preview.title, "default");
    assert_eq!(
        preview.lines,
        vec![
            ("Mode".to_string(), "AUTO".to_string()),
            ("Routing".to_string(), "REGIONAL".to_string()),
            ("Subnets".to_string(), "43".to_string()),
            (
                "Description".to_string(),
                "Default network for the project".to_string()
            ),
        ]
    );
}

/// The pane spells an unreported routing mode out rather than painting the
/// table's dash: a preview has the room to say which kind of "no value" it
/// means, and "unspecified" is a different fact from a mode this build does
/// not recognise.
#[test]
fn the_preview_spells_an_unspecified_routing_mode() {
    let mut app = app_showing_networks();
    // The default ordering is by name — default, empty-vpc, legacy-net,
    // prod-vpc — so two presses land on the legacy network.
    for _ in 0..2 {
        reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    }

    let preview = project_resource_preview(&app.root.resources).expect("a row is selected");

    assert_eq!(preview.title, "legacy-net");
    assert!(
        preview
            .lines
            .contains(&("Routing".to_string(), "Unspecified".to_string())),
        "an unreported routing mode is named, not dashed: {:?}",
        preview.lines
    );
    assert!(
        preview
            .lines
            .contains(&("Description".to_string(), "-".to_string())),
        "an absent description dashes: {:?}",
        preview.lines
    );
}

// --- Empty states ---

#[test]
fn a_project_with_no_networks_says_so() {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::VpcNetworksLoaded {
            context,
            networks: Vec::new(),
        }),
    );

    let painted = painted(app);

    assert!(
        painted.contains("No VPC networks found for"),
        "an empty project names itself:\n{painted}"
    );
}

/// The empty state keeps the tools line: what was filtered, how it is sorted,
/// and how many of the loaded rows survived.
///
/// This is the state where the count matters most — the table is blank, and
/// `Rows: 0/4` is the only thing on screen saying the project *has* networks
/// and this filter hid them. The line is suppressed in compact mode, which is
/// what makes it worth asserting at full height: dropping it everywhere would
/// leave the pane saying nothing but "no matches".
#[test]
fn a_filter_matching_nothing_says_so_rather_than_painting_an_empty_table() {
    let painted = painted(app_with_filter("zzz-no-such-network"));

    assert!(
        painted.contains("No networks match filter"),
        "a filter with no matches says so:\n{painted}"
    );
    assert!(
        painted.contains("Rows: 0/4"),
        "the empty state keeps the tools line, which is the only place the \
         hidden rows are counted:\n{painted}"
    );
    assert!(
        painted.contains("Sort: Name"),
        "the tools line names the sort the blank table is still under:\n{painted}"
    );
}

// --- Context guarding ---

/// A listing that resolved for a configuration the operator has since left is
/// dropped rather than installed.
///
/// The load is asynchronous and the operator can switch configuration while it
/// is in flight. `prod-project`'s networks under a `dev-project` heading are
/// indistinguishable from the truth on screen, and a VPC table is exactly the
/// place that misreading is expensive: the operator is deciding what a subnet
/// or a firewall rule applies to.
#[test]
fn a_networks_result_for_a_context_the_operator_has_left_is_dropped() {
    let mut app = app_showing_networks();

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::VpcNetworksLoaded {
            context: GcloudContext {
                configuration: GcloudConfigurationName::new("prod"),
                account: Some("dev@example.com".to_string()),
                project: Some("prod-project".to_string()),
                region: None,
                zone: None,
            },
            networks: vec![VpcNetwork {
                name: "prod-only-network".to_string(),
                subnet_mode: VpcSubnetMode::Custom,
                routing_mode: VpcRoutingMode::Global,
                subnet_count: 1,
                description: None,
            }],
        }),
    );

    let names = app
        .root
        .resources
        .vpc_networks()
        .iter()
        .map(|network| network.name.as_str())
        .collect::<Vec<_>>();

    assert!(
        !names.contains(&"prod-only-network"),
        "another configuration's networks must not land in this table: {names:?}"
    );
    assert_eq!(
        names,
        vec!["default", "prod-vpc", "empty-vpc", "legacy-net"],
        "the rows loaded for the active context are left exactly as they were"
    );
}

/// The same guard on the failure path, which is the half that regresses
/// silently.
///
/// A refusal that arrives for a configuration the operator has since left must
/// not empty the table they are looking at, nor put another project's error on
/// their screen. The success path fails loudly when its guard inverts — rows
/// stop appearing — while an inverted failure guard leaves a correct-looking
/// table that goes blank the next time some unrelated project's load is
/// refused.
#[test]
fn a_networks_failure_for_a_context_the_operator_has_left_is_dropped() {
    let mut app = app_showing_networks();

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::VpcNetworksLoadFailed {
            context: GcloudContext {
                configuration: GcloudConfigurationName::new("prod"),
                account: Some("dev@example.com".to_string()),
                project: Some("prod-project".to_string()),
                region: None,
                zone: None,
            },
            error: AppError::gcloud("PERMISSION_DENIED on prod-project".to_string()),
        }),
    );

    assert_eq!(
        app.root.resources.vpc_networks().len(),
        4,
        "another configuration's refusal must not clear this table"
    );
    assert!(
        app.root.resources.vpc_networks_problem().is_none(),
        "another configuration's refusal must not become this table's problem"
    );

    let table = table_text(&painted(app));
    assert!(
        table.contains("default"),
        "the rows the operator loaded are still on screen:\n{table}"
    );
}

// --- Filtering does not re-fetch ---

/// Applying a `/` filter emits **no effect** for this table, and the rows stay
/// where they are.
///
/// `GcloudVpcService::load_networks` ignores the filter it is handed, because
/// MODE, ROUTING and SUBNETS are derived and appear verbatim in no raw field,
/// so a re-fetch on every Enter re-downloads a document identical to the one
/// already in state. That is waste, and it is worse than waste: a failed
/// refresh runs `mark_active_stale_and_clear`, which drops the rows and the
/// context. A transient auth blip or quota error on a call that was never
/// needed would blank a table the operator is reading, in response to them
/// typing a filter.
///
/// **Paired with a service that does re-fetch, deliberately.** A reducer
/// mutated to return `Vec::new()` unconditionally would satisfy the negative
/// assertion on its own, and would be a far worse bug than the one this fixes:
/// no table would ever narrow server-side again. Compute pushes its filter
/// into `gcloud compute instances list --filter=`, so its arm of the same
/// match must still emit. The two halves together say "this arm, and not the
/// mechanism".
#[test]
fn applying_a_filter_refetches_the_pushdown_services_and_not_this_one() {
    let mut networks = app_showing_networks();
    reduce(
        &mut networks,
        &Action::Resources(ResourcesAction::StartFilter),
    );
    for character in "prod".chars() {
        reduce(
            &mut networks,
            &Action::Resources(ResourcesAction::AppendFilterChar(character)),
        );
    }
    let before = networks.root.resources.vpc_networks().len();
    let effects = reduce(
        &mut networks,
        &Action::Resources(ResourcesAction::ApplyFilter),
    );

    assert!(
        effects.is_empty(),
        "a client-side narrowing must not spawn a gcloud process, and must not \
         put the visible table at the mercy of that call failing: {effects:?}"
    );
    assert!(before > 0, "precondition: there were rows to keep");
    assert_eq!(
        networks.root.resources.vpc_networks().len(),
        before,
        "the rows are still in state; only what is visible changed"
    );

    // The positive half. Compute pushes `--filter=` down to gcloud, so its arm
    // of the same match must still emit a refresh.
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
///
/// Without this, "no effect on a filter change" and "no effect ever" are the
/// same assertion, and a table that could never be refreshed at all would pass
/// the test above.
#[test]
fn an_explicit_refresh_still_refetches_this_table() {
    let mut app = app_showing_networks();

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));

    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::RefreshVpcNetworks { .. })),
        "`r` is an explicit ask for fresh rows and must reach gcloud: {effects:?}"
    );
}
