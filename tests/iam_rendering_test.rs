//! IAM service accounts reach the screen.
//!
//! A service can be in `SERVICE_REGISTRY`, satisfy every `ResourceServiceKind`
//! walk, hold rows in state, and still never paint one — every one of those
//! tests passes and the tab is blank. So these assertions are made on the
//! terminal buffer, through the real reducer and the real renderer, not on
//! the projection model.
//!
//! The service is list-only in this slice, so what is asserted here is the
//! table: that it paints, that it heads the four agreed columns with the
//! email under the identity key, and — the assertion this file exists for —
//! that a disabled account reads `Disabled` on its own row while an absent
//! display name paints as a dash rather than as Rust's `None`.

use nuvem::app::projection::columns::{SERVICE_ACCOUNT_COLUMNS, catalog_meta};
use nuvem::app::projection::project_resources;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{ResourceServiceKind, ServiceAccount};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

const FRAME_WIDTH: u16 = 200;
const FRAME_HEIGHT: u16 = 40;

/// Wide enough that the table area fits the whole catalogue unelided: the
/// email cell is the widest column, and the full
/// `local@project.iam.gserviceaccount.com` spelling is what the assertions
/// key rows on.
const WIDE_FRAME_WIDTH: u16 = 320;

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

/// Three accounts: one fully populated, one disabled, and one with no display
/// name at all.
fn accounts() -> Vec<ServiceAccount> {
    vec![
        ServiceAccount {
            email: "sa-vault@dev-project.iam.gserviceaccount.com".to_string(),
            display_name: Some("Vault node pool".to_string()),
            disabled: false,
            unique_id: "110727432291788759739".to_string(),
        },
        ServiceAccount {
            email: "old-deployer@dev-project.iam.gserviceaccount.com".to_string(),
            display_name: Some("Retired deploy account".to_string()),
            disabled: true,
            unique_id: "104405669981102690742".to_string(),
        },
        ServiceAccount {
            email: "unnamed@dev-project.iam.gserviceaccount.com".to_string(),
            display_name: None,
            disabled: false,
            unique_id: "117303179448968546820".to_string(),
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
            ResourceServiceKind::IamServiceAccounts,
        )),
    );
    app.root.workspace = Workspace::Resources;
    app
}

fn app_showing_accounts() -> AppState {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ServiceAccountsLoaded {
            context,
            accounts: accounts(),
        }),
    );
    app
}

/// A state showing the three fixture accounts with `filter` applied through
/// the real filter actions. Any filter matching no account leaves the list
/// with rows loaded and none visible.
fn app_with_filter(filter: &str) -> AppState {
    let mut app = app_showing_accounts();
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
    painted_at(app, FRAME_WIDTH)
}

fn painted_at(app: AppState, width: u16) -> String {
    let session = RuntimeSession::new_for_test(app);
    let backend = TestBackend::new(width, FRAME_HEIGHT);
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
/// selected account's own email and state. A `contains` over the whole line
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

// --- The catalogue ---

#[test]
fn the_catalogue_is_the_four_agreed_columns_in_order() {
    let keys = SERVICE_ACCOUNT_COLUMNS
        .iter()
        .map(|column| column.spec.key)
        .collect::<Vec<_>>();

    // The identity column's key is the literal `name` even though its header
    // says EMAIL: `name` is what the column-layout config and the
    // identity-pinning rule match on, so it cannot vary by service.
    assert_eq!(keys, vec!["name", "display_name", "state", "unique_id"]);
}

#[test]
fn every_accounts_column_sorts() {
    let unsortable = catalog_meta(ResourceServiceKind::IamServiceAccounts)
        .iter()
        .filter(|column| !column.sortable)
        .map(|column| column.key)
        .collect::<Vec<&str>>();

    // Unlike the tables that carry labels, none of these cells is a flattened
    // map whose ordering would mean nothing.
    assert_eq!(unsortable, Vec::<&str>::new());
}

#[test]
fn the_identity_column_heads_email() {
    let identity = SERVICE_ACCOUNT_COLUMNS
        .iter()
        .find(|column| column.spec.key == "name")
        .expect("the accounts table carries the identity column");

    assert_eq!(
        identity.spec.header, "EMAIL",
        "the identity cell carries the email, and the header must say what \
         the cell shows"
    );
}

// --- The table ---

#[test]
fn the_accounts_table_paints_a_row_per_account() {
    let table = table_text(&painted_at(app_showing_accounts(), WIDE_FRAME_WIDTH));

    // The local parts rather than the full addresses: an email longer than
    // the column's preferred width elides its tail with an ellipsis, which is
    // correct behaviour and useless to assert a full address against.
    for email in ["sa-vault@", "old-deployer@", "unnamed@"] {
        assert!(
            table.contains(email),
            "the table must paint a row for {email}, got:\n{table}"
        );
    }
}

#[test]
fn the_table_heads_the_four_columns() {
    let table = table_text(&painted_at(app_showing_accounts(), WIDE_FRAME_WIDTH));

    for header in ["EMAIL", "DISPLAY NAME", "STATE", "UNIQUE ID"] {
        assert!(
            table.contains(header),
            "the table must head {header}, got:\n{table}"
        );
    }
}

#[test]
fn a_disabled_account_reads_disabled_on_its_own_row() {
    // The assertion this file exists for. "This identity cannot authenticate"
    // is the fact an access review scans the table for, and it must sit on
    // the disabled account's own row — not somewhere in the frame.
    let painted = painted_at(app_showing_accounts(), WIDE_FRAME_WIDTH);
    let row = table_lines(&painted)
        .into_iter()
        .find(|line| line.contains("old-deployer"))
        .map(str::to_string)
        .unwrap_or_else(|| panic!("the table paints the old-deployer row:\n{painted}"));

    assert!(
        row.contains("Disabled"),
        "the disabled account's own row must say Disabled: {row:?}"
    );
}

#[test]
fn an_enabled_account_reads_enabled_rather_than_carrying_a_blank() {
    // The other half of the pair: the ordinary case carries its own word too,
    // so the two populations are distinguishable — and filterable — by the
    // exact text their cells paint.
    let painted = painted_at(app_showing_accounts(), WIDE_FRAME_WIDTH);
    let row = table_lines(&painted)
        .into_iter()
        .find(|line| line.contains("sa-vault"))
        .map(str::to_string)
        .unwrap_or_else(|| panic!("the table paints the sa-vault row:\n{painted}"));

    assert!(
        row.contains("Enabled"),
        "an enabled account's row must say Enabled: {row:?}"
    );
    assert!(
        !row.contains("Disabled"),
        "an enabled account must not read as disabled: {row:?}"
    );
}

#[test]
fn an_absent_display_name_paints_empty_rather_than_the_word_none() {
    let painted = painted_at(app_showing_accounts(), WIDE_FRAME_WIDTH);
    let row = table_lines(&painted)
        .into_iter()
        .find(|line| line.contains("unnamed@"))
        .map(str::to_string)
        .unwrap_or_else(|| panic!("the table paints the unnamed row:\n{painted}"));

    assert!(
        !row.contains("None"),
        "an absent optional renders as an empty cell, not as Rust's None: {row:?}"
    );
}

#[test]
fn the_unique_id_paints_on_its_own_row() {
    let painted = painted_at(app_showing_accounts(), WIDE_FRAME_WIDTH);
    let row = table_lines(&painted)
        .into_iter()
        .find(|line| line.contains("sa-vault"))
        .map(str::to_string)
        .unwrap_or_else(|| panic!("the table paints the sa-vault row:\n{painted}"));

    assert!(
        row.contains("110727432291788759739"),
        "the unique id is the only stable spelling of a deleted-and-recreated \
         account and must paint on the row: {row:?}"
    );
}

// --- The empty states ---

/// The three status-selected empty branches of `render_service_accounts_list`.
///
/// On the Pub/Sub twin, each of the status comparisons selecting them
/// survived mutation from `==` to `!=`, because every rendering test above
/// paints a populated table. A wrong branch here shows an operator "loading"
/// when a fetch has failed, or a bare empty list when rows are merely stale.
#[test]
fn the_accounts_list_paints_a_distinct_empty_state_per_status() {
    // Loading: a fetch in flight against a collection with no rows yet.
    let mut loading = app_with_context();
    loading.root.resources.begin_load(
        ResourceServiceKind::IamServiceAccounts,
        std::time::Instant::now(),
    );
    let frame = painted(loading);
    assert!(
        frame.contains("Loading service accounts"),
        "an in-flight accounts fetch must paint the loading state, naming the \
         service it is loading:\n{frame}"
    );

    // Failed: the load-failed result for the active context.
    let mut failed = app_with_context();
    let context = failed.root.current_context().expect("active context");
    reduce(
        &mut failed,
        &Action::EffectResult(EffectResult::ServiceAccountsLoadFailed {
            context,
            error: nuvem::app::AppError::gcloud("permission denied"),
        }),
    );
    let frame = painted(failed);
    assert!(
        frame.contains("Press r to retry."),
        "a failed accounts fetch must paint the retry affordance, which is \
         what distinguishes it from an empty or stale list:\n{frame}"
    );

    // Stale: rows cleared pending a background refresh, with no load phase.
    let mut stale = app_showing_accounts();
    stale
        .root
        .resources
        .mark_active_stale_and_clear(ResourceServiceKind::IamServiceAccounts);
    let frame = painted(stale);
    assert!(
        frame.contains("Resource rows are stale. Press r to refresh."),
        "stale account rows must say they are stale, not read as a project \
         with no service accounts in it:\n{frame}"
    );
}

/// The three *non*-status empty branches: a project with no accounts, a list
/// that was never loaded, and a filter that matches nothing. The three read
/// as opposite facts to an operator: "this project has no service accounts",
/// "nothing has been fetched yet", and "your filter hid them all".
#[test]
fn the_accounts_list_distinguishes_no_accounts_from_not_loaded_from_filtered_out() {
    // Ready with zero rows: the project genuinely has no service accounts.
    let mut empty = app_with_context();
    let context = empty.root.current_context().expect("active context");
    reduce(
        &mut empty,
        &Action::EffectResult(EffectResult::ServiceAccountsLoaded {
            context,
            accounts: Vec::new(),
        }),
    );
    let frame = painted(empty);
    assert!(
        frame.contains("No service accounts found for"),
        "a completed fetch that returned nothing must say the configuration \
         has no service accounts:\n{frame}"
    );
    assert!(
        !frame.contains("Press r to load service accounts."),
        "a completed fetch must not invite a load it has already done:\n{frame}"
    );
    assert!(
        !frame.contains("No service accounts match filter"),
        "an empty project is not a hidden-by-filter list:\n{frame}"
    );

    // Never loaded: the service is selected but no fetch has completed.
    let frame = painted(app_with_context());
    assert!(
        frame.contains("Press r to load service accounts."),
        "a list nobody has fetched must offer the load, not report an empty \
         project:\n{frame}"
    );
    assert!(
        !frame.contains("No service accounts found for"),
        "nothing is known about the project's accounts before a fetch:\n{frame}"
    );
    assert!(
        !frame.contains("No service accounts match filter"),
        "an unfetched list is not a hidden-by-filter list:\n{frame}"
    );

    // Filtered out: rows exist, none of them survive the applied filter.
    let frame = painted(app_with_filter("zzz-matches-nothing"));
    assert!(
        frame.contains("No service accounts match filter \"zzz-matches-nothing\""),
        "a filter that hides every row must name itself, so the operator \
         knows the rows are hidden rather than absent:\n{frame}"
    );
    assert!(
        !frame.contains("No service accounts found for"),
        "rows that exist behind a filter must not be reported as absent:\n{frame}"
    );
}

/// The list-tools header on the empty-state path: the line that tells an
/// operator staring at "no matches" what filter is applied and how many rows
/// it hid.
#[test]
fn a_filtered_empty_accounts_list_keeps_its_filter_and_row_count_header() {
    let frame = painted(app_with_filter("zzz-matches-nothing"));

    assert!(
        frame.contains("Filter: zzz-matches-nothing"),
        "the empty-state list must still carry the applied filter in its \
         tools header:\n{frame}"
    );
    assert!(
        frame.contains("Rows: 0/3"),
        "the tools header must report how many of the loaded rows survived \
         the filter, which is what distinguishes a hiding filter from an \
         empty project:\n{frame}"
    );
}

/// `TableModel::selected_row` through the projection — the state-level
/// `selected_index` goes through a different accessor, so a state assertion
/// pins the wrong function (the lesson of the Pub/Sub twin, whose comparison
/// survived mutation to `==`, `<` and `>=`).
#[test]
fn the_table_model_selects_a_row_while_rows_are_visible_and_none_when_the_filter_empties_it() {
    let app = app_showing_accounts();
    let model = project_resources(&app.root.resources)
        .table
        .expect("list mode builds a table model");

    assert!(!model.rows.is_empty(), "the fixture must project rows");
    assert_eq!(
        model.selected_row,
        Some(0),
        "a populated table starts with its first row selected; None leaves \
         the table with nothing highlighted"
    );

    let filtered = app_with_filter("zzz-matches-nothing");
    let model = project_resources(&filtered.root.resources)
        .table
        .expect("the empty-state path still builds a table model");

    assert!(
        model.rows.is_empty(),
        "the fixture needs a filter that hides every row"
    );
    assert_eq!(
        model.selected_row, None,
        "with no visible rows there is no row to select, and reporting one \
         points the cursor at a row the operator cannot see"
    );
}

// --- The preview ---

/// The preview pane's accounts arm, on the projection: the email as the
/// title, and the display name, state and unique id as the lines. The
/// mechanically transformed arm originally read Pub/Sub's fields here, so the
/// three lines are pinned to the account's own facts.
#[test]
fn the_preview_shows_the_selected_accounts_own_facts() {
    let app = app_showing_accounts();
    // The default sort is Email ascending, so the selected first row is
    // old-deployer, not the fixture's first element.
    let preview = nuvem::app::projection::project_resource_preview(&app.root.resources)
        .expect("a selected account previews");

    assert_eq!(
        preview.title, "old-deployer@dev-project.iam.gserviceaccount.com",
        "the preview titles the selected account by its email"
    );
    assert_eq!(
        preview.lines,
        vec![
            (
                "Display name".to_string(),
                "Retired deploy account".to_string()
            ),
            ("State".to_string(), "Disabled".to_string()),
            ("Unique ID".to_string(), "104405669981102690742".to_string()),
        ],
        "the preview lines carry the account's display name, state and \
         unique id — not another service's fields"
    );
}
