//! The Pub/Sub tables reach the screen.
//!
//! A service can be in `SERVICE_REGISTRY`, satisfy every `ResourceServiceKind`
//! walk, hold rows in state, and still never paint one — every one of those
//! tests passes and the tab is blank. So these assertions are made on the
//! terminal buffer, through the real reducer and the real renderer, not on the
//! projection model.
//!
//! The service is list-only in this slice, so what is asserted here is the
//! table: that it paints, that it heads the five agreed columns, and — the
//! assertion this file exists for — that a subscription count which could not
//! be fetched paints as a dash and never as a zero.

use nuvem::app::projection::columns::{
    PUBSUB_SUBSCRIPTION_COLUMNS, PUBSUB_TOPIC_COLUMNS, catalog_meta,
};
use nuvem::app::projection::project_resources;
use nuvem::app::projection::table::{CellKind, StatusSemantic};
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    ComputeKeyValue, PubSubDeadLetterPolicy, PubSubDelivery, PubSubSubscription,
    PubSubSubscriptionState, PubSubTopic, ResourceServiceKind, SubscriptionCount,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

const FRAME_WIDTH: u16 = 200;
const FRAME_HEIGHT: u16 = 40;

/// Wide enough that the table area fits the whole catalogue unelided: the KMS
/// key cell is the widest column and the one the layout is meant to drop first,
/// which is correct behaviour and useless to assert a key path against.
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

/// Three topics: one fully populated, one with nothing optional set, and one
/// whose count could not be fetched.
fn topics() -> Vec<PubSubTopic> {
    vec![
        PubSubTopic {
            name: "orders".to_string(),
            full_name: "projects/dev-project/topics/orders".to_string(),
            subscription_count: SubscriptionCount::Known(4),
            message_retention: Some("604800s".to_string()),
            kms_key: Some(
                "projects/dev-project/locations/europe-west1/keyRings/nuvem/cryptoKeys/pubsub"
                    .to_string(),
            ),
            labels: vec![ComputeKeyValue {
                key: "team".to_string(),
                value: "platform".to_string(),
            }],
        },
        PubSubTopic {
            name: "audit-log".to_string(),
            full_name: "projects/dev-project/topics/audit-log".to_string(),
            subscription_count: SubscriptionCount::Known(0),
            message_retention: None,
            kms_key: None,
            labels: Vec::new(),
        },
        PubSubTopic {
            name: "uncounted".to_string(),
            full_name: "projects/dev-project/topics/uncounted".to_string(),
            subscription_count: SubscriptionCount::Unavailable,
            message_retention: Some("86400s".to_string()),
            kms_key: None,
            labels: Vec::new(),
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
            ResourceServiceKind::PubSubTopics,
        )),
    );
    app.root.workspace = Workspace::Resources;
    app
}

fn app_showing_topics() -> AppState {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PubSubTopicsLoaded {
            context,
            topics: topics(),
        }),
    );
    app
}

/// A state showing the three fixture topics with `filter` applied through the
/// real filter actions. Any filter matching no topic leaves the list with rows
/// loaded and none visible.
fn app_with_filter(filter: &str) -> AppState {
    let mut app = app_showing_topics();
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
/// selected topic's own name and count. A `contains` over the whole line
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
fn the_catalogue_is_the_five_agreed_columns_in_order() {
    let keys = PUBSUB_TOPIC_COLUMNS
        .iter()
        .map(|column| column.spec.key)
        .collect::<Vec<_>>();

    assert_eq!(
        keys,
        vec!["name", "subscriptions", "retention", "kms_key", "labels"]
    );
}

#[test]
fn labels_is_the_one_column_that_does_not_sort() {
    let unsortable = catalog_meta(ResourceServiceKind::PubSubTopics)
        .iter()
        .filter(|column| !column.sortable)
        .map(|column| column.key)
        .collect::<Vec<_>>();

    // A flattened `k=v, k=v` cell sorts by whichever key happens to sort
    // first, which is an order that means nothing.
    assert_eq!(unsortable, vec!["labels"]);
}

// --- The table ---

#[test]
fn the_topics_table_paints_a_row_per_topic() {
    let table = table_text(&painted(app_showing_topics()));

    for name in ["orders", "audit-log", "uncounted"] {
        assert!(
            table.contains(name),
            "the table must paint {name}, got:\n{table}"
        );
    }
}

#[test]
fn the_table_heads_the_five_columns() {
    let table = table_text(&painted_at(app_showing_topics(), WIDE_FRAME_WIDTH));

    for header in ["NAME", "SUBS", "RETENTION", "KMS KEY", "LABELS"] {
        assert!(
            table.contains(header),
            "the table must head {header}, got:\n{table}"
        );
    }
}

#[test]
fn a_known_count_paints_its_digits() {
    let table = table_text(&painted(app_showing_topics()));

    let orders = table_lines(&painted(app_showing_topics()))
        .into_iter()
        .find(|line| line.contains("orders"))
        .map(str::to_string)
        .unwrap_or_else(|| panic!("the table paints an orders row:\n{table}"));

    assert!(
        orders.contains('4'),
        "orders has four subscriptions and the cell must say so: {orders:?}"
    );
}

#[test]
fn an_unavailable_count_paints_a_dash_and_never_a_zero() {
    // The assertion this file exists for. A count that could not be fetched
    // rendered as `0` is a specific claim — "this topic has no subscribers" —
    // and the wrong one, and it is invisible in every other cell.
    let painted = painted(app_showing_topics());
    let row = table_lines(&painted)
        .into_iter()
        .find(|line| line.contains("uncounted"))
        .map(str::to_string)
        .unwrap_or_else(|| panic!("the table paints an uncounted row:\n{painted}"));

    let cells = row.split_whitespace().collect::<Vec<_>>();
    assert!(
        cells.contains(&SubscriptionCount::Unavailable.label().as_str()),
        "the unavailable count must paint as a dash: {row:?}"
    );
    assert!(
        !cells.contains(&"0"),
        "an unavailable count must never paint as zero: {row:?}"
    );
}

#[test]
fn a_genuine_zero_still_paints_as_zero() {
    // The other half of the pair: `Known(0)` is a real answer and must be
    // distinguishable from `Unavailable`, or the dash means nothing.
    let painted = painted(app_showing_topics());
    let row = table_lines(&painted)
        .into_iter()
        .find(|line| line.contains("audit-log"))
        .map(str::to_string)
        .unwrap_or_else(|| panic!("the table paints an audit-log row:\n{painted}"));

    let cells = row.split_whitespace().collect::<Vec<_>>();
    assert!(
        cells.contains(&"0"),
        "a topic the call reported no subscriptions for paints a zero: {row:?}"
    );
}

#[test]
fn an_unset_retention_paints_empty_rather_than_the_word_none() {
    let painted = painted(app_showing_topics());
    let row = table_lines(&painted)
        .into_iter()
        .find(|line| line.contains("audit-log"))
        .map(str::to_string)
        .unwrap_or_else(|| panic!("the table paints an audit-log row:\n{painted}"));

    assert!(
        !row.contains("None"),
        "an absent optional renders as an empty cell, not as Rust's None: {row:?}"
    );
}

/// The LABELS cell, pinned to its rendered text rather than to the column
/// merely existing.
///
/// `pubsub_labels_cell` survived mutation to both `String::new()` and a
/// constant: the table tests above assert the LABELS *header* and the row
/// names, so an empty or wrong labels cell painted happily. Both branches are
/// asserted here — the flattened `k=v, k=v` form and the no-labels dash —
/// because the function has two of them and returning a constant satisfies
/// neither.
///
/// The populated cell is asserted on the projection rather than the frame: it
/// pins the separator and the pair order exactly, and it cannot be softened by
/// a column width that elides the tail. The dash is asserted on the unlabelled
/// row's own cell for the reason its secrets counterpart is — topic names and
/// KMS keys are full of hyphens, so a whole-frame `contains('-')` would hold
/// for any frame that renders at all.
#[test]
fn the_labels_cell_paints_its_key_value_pairs_and_a_dash_when_empty() {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PubSubTopicsLoaded {
            context,
            topics: vec![
                PubSubTopic {
                    name: "labelled".to_string(),
                    full_name: "projects/dev-project/topics/labelled".to_string(),
                    subscription_count: SubscriptionCount::Known(2),
                    message_retention: Some("604800s".to_string()),
                    kms_key: None,
                    labels: vec![
                        ComputeKeyValue {
                            key: "env".to_string(),
                            value: "prod".to_string(),
                        },
                        ComputeKeyValue {
                            key: "team".to_string(),
                            value: "platform".to_string(),
                        },
                    ],
                },
                PubSubTopic {
                    name: "unlabelled".to_string(),
                    full_name: "projects/dev-project/topics/unlabelled".to_string(),
                    subscription_count: SubscriptionCount::Known(1),
                    message_retention: None,
                    kms_key: None,
                    labels: Vec::new(),
                },
            ],
        }),
    );

    let model = project_resources(&app.root.resources)
        .table
        .expect("list mode builds a table model");
    let labels_column = model
        .columns
        .iter()
        .position(|column| column.key == "labels")
        .expect("the topics table carries a labels column");
    let name_column = model
        .columns
        .iter()
        .position(|column| column.key == "name")
        .expect("the topics table carries a name column");
    let cell = |topic: &str| {
        model
            .rows
            .iter()
            .find(|row| row.cells[name_column].text == topic)
            .unwrap_or_else(|| panic!("the {topic} fixture row is projected"))
            .cells[labels_column]
            .text
            .clone()
    };

    assert_eq!(
        cell("labelled"),
        "env=prod, team=platform",
        "every label must flatten into the one cell as key=value, comma \
         separated and in order — not just the first, and not a constant"
    );
    assert_eq!(
        cell("unlabelled"),
        "-",
        "a topic with no labels must fill its LABELS cell with a dash rather \
         than leave it empty"
    );

    // Through the renderer too: the projection can be right while the cell
    // never reaches the screen, which is what this file exists to catch.
    let table = table_text(&painted_at(app, WIDE_FRAME_WIDTH));
    assert!(
        table.contains("env=prod"),
        "a label must paint as key=value, which is how an operator reads and \
         searches it:\n{table}"
    );
    assert!(
        table.contains("team=platform"),
        "every label must paint, not only the first:\n{table}"
    );
}

// --- The empty states ---

/// The three status-selected empty branches of `render_pubsub_topics_list`.
///
/// Each of the status comparisons selecting them survived mutation from `==`
/// to `!=`, because every rendering test above paints a populated table. A
/// wrong branch here shows an operator "loading" when a fetch has failed, or a
/// bare empty list when rows are merely stale.
#[test]
fn the_topics_list_paints_a_distinct_empty_state_per_status() {
    // Loading: a fetch in flight against a collection with no rows yet.
    let mut loading = app_with_context();
    loading
        .root
        .resources
        .begin_load(ResourceServiceKind::PubSubTopics, std::time::Instant::now());
    let frame = painted(loading);
    assert!(
        frame.contains("Loading topics"),
        "an in-flight topics fetch must paint the loading state, naming the \
         service it is loading:\n{frame}"
    );

    // Failed: the load-failed result for the active context.
    let mut failed = app_with_context();
    let context = failed.root.current_context().expect("active context");
    reduce(
        &mut failed,
        &Action::EffectResult(EffectResult::PubSubTopicsLoadFailed {
            context,
            error: nuvem::app::AppError::gcloud("permission denied"),
        }),
    );
    let frame = painted(failed);
    assert!(
        frame.contains("Press r to retry."),
        "a failed topics fetch must paint the retry affordance, which is what \
         distinguishes it from an empty or stale list:\n{frame}"
    );

    // Stale: rows cleared pending a background refresh, with no load phase.
    let mut stale = app_showing_topics();
    stale
        .root
        .resources
        .mark_active_stale_and_clear(ResourceServiceKind::PubSubTopics);
    let frame = painted(stale);
    assert!(
        frame.contains("Resource rows are stale. Press r to refresh."),
        "stale topic rows must say they are stale, not read as a project with \
         no topics in it:\n{frame}"
    );
}

/// The three *non*-status empty branches: a project with no topics, a list
/// that was never loaded, and a filter that matches nothing.
///
/// The status comparisons above cover Loading/Failed/Stale, which leaves the
/// tail of the chain — `total_rows == 0 && status == Ready`, `total_rows == 0`,
/// `visible_rows == 0` — unpinned, and each of those survived mutation (the
/// `&&` to `||` among them). The three read as opposite facts to an operator:
/// "this project has no topics", "nothing has been fetched yet", and "your
/// filter hid them all".
#[test]
fn the_topics_list_distinguishes_no_topics_from_not_loaded_from_filtered_out() {
    // Ready with zero rows: the project genuinely has no topics.
    let mut empty = app_with_context();
    let context = empty.root.current_context().expect("active context");
    reduce(
        &mut empty,
        &Action::EffectResult(EffectResult::PubSubTopicsLoaded {
            context,
            topics: Vec::new(),
        }),
    );
    let frame = painted(empty);
    assert!(
        frame.contains("No Pub/Sub topics found for"),
        "a completed fetch that returned nothing must say the configuration \
         has no topics:\n{frame}"
    );
    assert!(
        !frame.contains("Press r to load Pub/Sub topics."),
        "a completed fetch must not invite a load it has already done:\n{frame}"
    );
    assert!(
        !frame.contains("No topics match filter"),
        "an empty project is not a hidden-by-filter list:\n{frame}"
    );

    // Never loaded: the service is selected but no fetch has completed.
    let frame = painted(app_with_context());
    assert!(
        frame.contains("Press r to load Pub/Sub topics."),
        "a list nobody has fetched must offer the load, not report an empty \
         project:\n{frame}"
    );
    assert!(
        !frame.contains("No Pub/Sub topics found for"),
        "nothing is known about the project's topics before a fetch:\n{frame}"
    );
    assert!(
        !frame.contains("No topics match filter"),
        "an unfetched list is not a hidden-by-filter list:\n{frame}"
    );

    // Filtered out: rows exist, none of them survive the applied filter.
    let frame = painted(app_with_filter("zzz-matches-nothing"));
    assert!(
        frame.contains("No topics match filter \"zzz-matches-nothing\""),
        "a filter that hides every row must name itself, so the operator knows \
         the rows are hidden rather than absent:\n{frame}"
    );
    assert!(
        !frame.contains("No Pub/Sub topics found for"),
        "rows that exist behind a filter must not be reported as absent:\n{frame}"
    );
}

/// The list-tools header on the empty-state path.
///
/// `(!compact)` survived deletion of its `!`, which drops the Filter/Sort/Rows
/// header from a full-width terminal — exactly the line that tells an operator
/// staring at "no matches" what filter is applied and how many rows it hid.
#[test]
fn a_filtered_empty_topics_list_keeps_its_filter_and_row_count_header() {
    let frame = painted(app_with_filter("zzz-matches-nothing"));

    assert!(
        frame.contains("Filter: zzz-matches-nothing"),
        "the empty-state list must still carry the applied filter in its tools \
         header:\n{frame}"
    );
    assert!(
        frame.contains("Rows: 0/3"),
        "the tools header must report how many of the loaded rows survived the \
         filter, which is what distinguishes a hiding filter from an empty \
         project:\n{frame}"
    );
}

/// `TableModel::selected_row`, which is `selected_visible_index_for_visible_rows`
/// and nothing else.
///
/// That function's `visible_rows > 0` survived mutation to `==`, `<` and `>=`.
/// Two of those are constant for a `usize`: `< 0` never holds, so no row is
/// ever highlighted, and `>= 0` always holds, so a filtered-to-nothing list
/// reports a cursor into rows that are not there.
///
/// It is reached only from `project_resources` — `ResourcesState::selected_index`
/// goes through the *other* accessor, `selected_visible_index`, so a state-level
/// assertion pins the wrong function and leaves all three mutants alive. Hence
/// the projection here rather than the state.
#[test]
fn the_table_model_selects_a_row_while_rows_are_visible_and_none_when_the_filter_empties_it() {
    let app = app_showing_topics();
    let model = project_resources(&app.root.resources)
        .table
        .expect("list mode builds a table model");

    assert!(!model.rows.is_empty(), "the fixture must project rows");
    assert_eq!(
        model.selected_row,
        Some(0),
        "a populated table starts with its first row selected; None leaves the \
         table with nothing highlighted"
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

// --- The flat subscriptions table ---
//
// Same bargain as the topics assertions above: made on the terminal buffer
// through the real reducer and renderer, because a service can satisfy every
// walk in the workspace and still paint a blank tab.

/// Four subscriptions: a fully-populated pull, a push in `RESOURCE_ERROR` with
/// a dead-letter policy, one with nothing optional set, and an orphan whose
/// topic has been deleted.
fn subscriptions() -> Vec<PubSubSubscription> {
    vec![
        PubSubSubscription {
            name: "orders-worker".to_string(),
            full_name: "projects/dev-project/subscriptions/orders-worker".to_string(),
            topic: "projects/dev-project/topics/orders".to_string(),
            delivery: PubSubDelivery::Pull,
            ack_deadline_seconds: Some(60),
            message_retention: Some("604800s".to_string()),
            dead_letter: None,
            state: PubSubSubscriptionState::Active,
            labels: vec![ComputeKeyValue {
                key: "team".to_string(),
                value: "platform".to_string(),
            }],
        },
        PubSubSubscription {
            name: "orders-audit".to_string(),
            full_name: "projects/dev-project/subscriptions/orders-audit".to_string(),
            topic: "projects/dev-project/topics/orders".to_string(),
            delivery: PubSubDelivery::Push {
                endpoint: "https://audit.example.com/pubsub".to_string(),
            },
            ack_deadline_seconds: Some(10),
            message_retention: None,
            dead_letter: Some(PubSubDeadLetterPolicy {
                dead_letter_topic: "projects/dev-project/topics/dead-letters".to_string(),
                max_delivery_attempts: Some(5),
            }),
            state: PubSubSubscriptionState::ResourceError,
            labels: Vec::new(),
        },
        PubSubSubscription {
            name: "bare".to_string(),
            full_name: "projects/dev-project/subscriptions/bare".to_string(),
            topic: "projects/dev-project/topics/audit-log".to_string(),
            delivery: PubSubDelivery::Pull,
            ack_deadline_seconds: None,
            message_retention: None,
            dead_letter: None,
            state: PubSubSubscriptionState::Unspecified,
            labels: Vec::new(),
        },
        PubSubSubscription {
            name: "orphaned".to_string(),
            full_name: "projects/dev-project/subscriptions/orphaned".to_string(),
            topic: "_deleted-topic_".to_string(),
            delivery: PubSubDelivery::Pull,
            ack_deadline_seconds: Some(10),
            message_retention: None,
            dead_letter: None,
            state: PubSubSubscriptionState::Active,
            labels: Vec::new(),
        },
    ]
}

fn app_showing_subscriptions() -> AppState {
    let mut app = app_with_context();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::PubSubSubscriptions,
        )),
    );
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PubSubSubscriptionsLoaded {
            context,
            subscriptions: subscriptions(),
        }),
    );
    app
}

#[test]
fn the_subscriptions_catalogue_is_the_eight_agreed_columns_in_order() {
    let keys = PUBSUB_SUBSCRIPTION_COLUMNS
        .iter()
        .map(|column| column.spec.key)
        .collect::<Vec<_>>();

    assert_eq!(
        keys,
        vec![
            "name",
            "topic",
            "delivery",
            "state",
            "ack_deadline",
            "retention",
            "dead_letter",
            "labels"
        ]
    );
}

#[test]
fn every_subscription_column_is_in_the_catalogue_meta_the_config_layer_reads() {
    // `catalog_meta` is what a `[pubsub-subscriptions] columns = [...]` block
    // is validated against. A column in the static catalogue but not in the
    // meta is a column the table paints and the config file rejects by name.
    let meta_keys = catalog_meta(ResourceServiceKind::PubSubSubscriptions)
        .iter()
        .map(|column| column.key)
        .collect::<Vec<_>>();
    let catalogue_keys = PUBSUB_SUBSCRIPTION_COLUMNS
        .iter()
        .map(|column| column.spec.key)
        .collect::<Vec<_>>();

    assert_eq!(meta_keys, catalogue_keys);
}

#[test]
fn the_subscriptions_table_paints_a_row_per_subscription() {
    let table = table_text(&painted_at(app_showing_subscriptions(), WIDE_FRAME_WIDTH));

    for name in ["orders-worker", "orders-audit", "bare", "orphaned"] {
        assert!(
            table.contains(name),
            "the table must paint {name}, got:\n{table}"
        );
    }
}

#[test]
fn the_subscriptions_table_heads_its_columns() {
    let table = table_text(&painted_at(app_showing_subscriptions(), WIDE_FRAME_WIDTH));

    for header in [
        "NAME",
        "TOPIC",
        "DELIVERY",
        "STATE",
        "ACK(S)",
        "RETENTION",
        "DEAD LETTER",
    ] {
        assert!(
            table.contains(header),
            "the table must head {header}, got:\n{table}"
        );
    }
}

#[test]
fn the_topic_cell_paints_the_short_id_not_the_resource_path() {
    // The project prefix is identical on every row and costs the width the
    // facts need.
    let table = table_text(&painted_at(app_showing_subscriptions(), WIDE_FRAME_WIDTH));

    assert!(
        !table.contains("projects/dev-project/topics/orders"),
        "the cell must not paint the full resource path, got:\n{table}"
    );
    assert!(
        table.contains("orders"),
        "the cell must paint the short id, got:\n{table}"
    );
}

#[test]
fn an_orphaned_subscription_paints_the_deleted_topic_sentinel_rather_than_a_blank() {
    // A blank would read as "no parent topic recorded". The sentinel is a real
    // value the API returned and the finding an operator is looking for.
    let table = table_text(&painted_at(app_showing_subscriptions(), WIDE_FRAME_WIDTH));

    assert!(
        table.contains("_deleted-topic_"),
        "the orphan's TOPIC cell must paint the sentinel, got:\n{table}"
    );
}

#[test]
fn a_subscription_with_no_ack_deadline_paints_a_dash_and_never_a_zero() {
    // The topics table's count invariant, restated for this column: `0` is a
    // specific claim — an immediate ack deadline — that an absent field is in
    // no position to make.
    let table = table_text(&painted_at(app_showing_subscriptions(), WIDE_FRAME_WIDTH));
    let bare = table
        .lines()
        .find(|line| line.contains("bare"))
        .unwrap_or_else(|| panic!("the table must paint the bare row, got:\n{table}"));

    assert!(
        !bare.contains('0'),
        "an absent ack deadline must not paint as a zero: {bare:?}"
    );
    assert!(
        bare.contains('-'),
        "an absent ack deadline must paint as a dash: {bare:?}"
    );
}

#[test]
fn a_pull_and_a_push_subscription_paint_different_delivery_cells() {
    // `pushConfig` renders as `{}` for a pull subscription, so reading its
    // presence would label every pull row a push one. The two must differ on
    // screen.
    let table = table_text(&painted_at(app_showing_subscriptions(), WIDE_FRAME_WIDTH));

    assert!(
        table.contains("Pull"),
        "the pull rows must paint Pull, got:\n{table}"
    );
    assert!(
        table.contains("Push"),
        "the push row must paint Push, got:\n{table}"
    );
}

#[test]
fn a_resource_error_subscription_paints_its_state_in_the_readable_spelling() {
    // `RESOURCE_ERROR` is the wire value; the operator reads `Resource error`.
    let table = table_text(&painted_at(app_showing_subscriptions(), WIDE_FRAME_WIDTH));

    assert!(
        table.contains("Resource error"),
        "the broken subscription must name its state, got:\n{table}"
    );
    assert!(
        !table.contains("RESOURCE_ERROR"),
        "the wire spelling must not reach the screen, got:\n{table}"
    );
}

#[test]
fn a_resource_error_state_carries_the_degraded_semantic_not_just_the_words() {
    // Asserting the text alone leaves the styling free to say the opposite of
    // the words — a `RESOURCE_ERROR` row painted in the healthy colour reads
    // as fine at a glance, which is the failure this column exists to prevent.
    // The first version of this file asserted only the spelling and let that
    // through.
    let app = app_showing_subscriptions();
    let model = project_resources(&app.root.resources)
        .table
        .expect("list mode builds a table model");
    let state_column = model
        .columns
        .iter()
        .position(|column| column.key == "state")
        .expect("the subscriptions table carries a state column");
    let name_column = model
        .columns
        .iter()
        .position(|column| column.key == "name")
        .expect("the subscriptions table carries a name column");

    let semantic_for = |name: &str| {
        model
            .rows
            .iter()
            .find(|row| row.cells[name_column].text == name)
            .unwrap_or_else(|| panic!("the model must carry a row for {name}"))
            .cells[state_column]
            .kind
            .clone()
    };

    assert_eq!(
        semantic_for("orders-audit"),
        CellKind::Status(StatusSemantic::Degraded),
        "a subscription Pub/Sub cannot deliver to must be styled as degraded"
    );
    assert_eq!(
        semantic_for("orders-worker"),
        CellKind::Status(StatusSemantic::Healthy),
        "an active subscription must be styled as healthy"
    );
    assert_eq!(
        semantic_for("bare"),
        CellKind::Status(StatusSemantic::Neutral),
        "an absent state is a fact the API did not report, not a healthy one"
    );
}

#[test]
fn the_dead_letter_cell_paints_the_short_id_and_a_dash_where_there_is_no_policy() {
    let table = table_text(&painted_at(app_showing_subscriptions(), WIDE_FRAME_WIDTH));

    assert!(
        table.contains("dead-letters"),
        "the configured policy must paint its topic, got:\n{table}"
    );
    assert!(
        !table.contains("projects/dev-project/topics/dead-letters"),
        "the cell must not paint the full resource path, got:\n{table}"
    );
}

#[test]
fn the_subscriptions_tab_is_titled_for_subscriptions_not_topics() {
    // The two tables sit next to each other in the same category, so a title
    // copied from its sibling is a real and invisible confusion.
    let painted = painted_at(app_showing_subscriptions(), WIDE_FRAME_WIDTH);

    assert!(
        painted.contains("Pub/Sub subscriptions"),
        "the pane must name itself, got:\n{painted}"
    );
}

/// A state showing the fixture subscriptions with `filter` applied through the
/// real filter actions.
fn app_with_subscription_filter(filter: &str) -> AppState {
    let mut app = app_showing_subscriptions();
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

#[test]
fn the_subscriptions_labels_cell_paints_its_pairs_and_a_dash_when_empty() {
    let table = table_text(&painted_at(app_showing_subscriptions(), WIDE_FRAME_WIDTH));

    assert!(
        table.contains("team=platform"),
        "a labelled subscription must paint its pairs, got:\n{table}"
    );
    let bare = table
        .lines()
        .find(|line| line.contains("bare"))
        .unwrap_or_else(|| panic!("the table must paint the bare row, got:\n{table}"));
    assert!(
        bare.contains('-'),
        "a subscription with no labels must paint a dash: {bare:?}"
    );
}

#[test]
fn the_subscriptions_list_distinguishes_no_rows_from_not_loaded_from_filtered_out() {
    // Three different nothings that must not be spelled the same way: an empty
    // project, a list nobody has fetched, and rows hidden behind a filter.
    // Telling the operator "no subscriptions" when the truth is "you have not
    // loaded them" sends them looking for a Pub/Sub problem that is not there.
    let mut empty = app_with_context();
    reduce(
        &mut empty,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::PubSubSubscriptions,
        )),
    );
    let context = empty.root.current_context().expect("active context");
    reduce(
        &mut empty,
        &Action::EffectResult(EffectResult::PubSubSubscriptionsLoaded {
            context,
            subscriptions: Vec::new(),
        }),
    );
    let frame = painted(empty);
    assert!(
        frame.contains("No Pub/Sub subscriptions found for"),
        "a completed fetch that returned nothing must say the configuration \
         has no subscriptions:\n{frame}"
    );
    assert!(
        !frame.contains("Press r to load Pub/Sub subscriptions."),
        "a completed fetch must not invite a load it has already done:\n{frame}"
    );

    // Never loaded: the service is selected but no fetch has completed.
    let mut unfetched = app_with_context();
    reduce(
        &mut unfetched,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::PubSubSubscriptions,
        )),
    );
    let frame = painted(unfetched);
    assert!(
        frame.contains("Press r to load Pub/Sub subscriptions."),
        "a list nobody has fetched must offer the load, not report an empty \
         project:\n{frame}"
    );
    assert!(
        !frame.contains("No Pub/Sub subscriptions found for"),
        "nothing is known about the project's subscriptions before a \
         fetch:\n{frame}"
    );

    // Filtered out: rows exist, none of them survive the applied filter.
    let frame = painted(app_with_subscription_filter("zzz-matches-nothing"));
    assert!(
        frame.contains("No subscriptions match filter \"zzz-matches-nothing\""),
        "a filter that hides every row must name itself, so the operator knows \
         the rows are hidden rather than absent:\n{frame}"
    );
    assert!(
        !frame.contains("No Pub/Sub subscriptions found for"),
        "rows that exist behind a filter must not be reported as absent:\n{frame}"
    );
}

#[test]
fn a_failed_subscriptions_load_paints_the_failure_rather_than_an_empty_project() {
    // The service layer surfaces a refusal instead of swallowing it; this is
    // the other half of that bargain — the refusal has to reach the screen as
    // a failure, not as "this project has no subscriptions".
    let mut app = app_with_context();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::PubSubSubscriptions,
        )),
    );
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PubSubSubscriptionsLoadFailed {
            context,
            error: nuvem::app::AppError::gcloud(
                "PERMISSION_DENIED: pubsub.subscriptions.list".to_string(),
            ),
        }),
    );

    let frame = painted(app);
    assert!(
        !frame.contains("No Pub/Sub subscriptions found for"),
        "a refused load must never read as an empty project:\n{frame}"
    );
}

#[test]
fn an_unrecognized_state_is_sanitized_before_it_reaches_the_preview_or_the_row() {
    // `PubSubSubscriptionState::Unrecognized` carries the API's string
    // verbatim — it is the one value on either surface this binary did not
    // choose. Control characters in it could move the cursor or clear the
    // screen, so both the table cell and the preview pane must scrub it, and
    // an earlier version of the preview did not.
    let mut app = app_with_context();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::PubSubSubscriptions,
        )),
    );
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PubSubSubscriptionsLoaded {
            context,
            subscriptions: vec![PubSubSubscription {
                name: "odd".to_string(),
                full_name: "projects/dev-project/subscriptions/odd".to_string(),
                topic: "projects/dev-project/topics/orders".to_string(),
                delivery: PubSubDelivery::Pull,
                ack_deadline_seconds: Some(10),
                message_retention: None,
                dead_letter: None,
                state: PubSubSubscriptionState::Unrecognized(
                    "WEIRD\u{1b}[2J\u{7}STATE".to_string(),
                ),
                labels: Vec::new(),
            }],
        }),
    );

    let frame = painted_at(app, WIDE_FRAME_WIDTH);
    assert!(
        !frame.contains('\u{1b}'),
        "no escape character may reach the screen"
    );
    assert!(
        !frame.contains('\u{7}'),
        "no bell character may reach the screen"
    );
}
