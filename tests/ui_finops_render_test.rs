//! Render coverage for the FinOps workspace body: the unprobed state, a
//! partially covered card, and a refusal that names its remedy.

use nuvem::app::AppState;
use nuvem::app::state::finops::CellState;
use nuvem::app::state::root::Workspace;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::diagnostics::GcloudErrorKind;
use nuvem_domain::gcp::finops::{CostRecommendation, RecommendationState, RecommenderId};
use nuvem_domain::gcp::money::Money;
use ratatui::{Terminal, backend::TestBackend};

fn lines(terminal: &Terminal<TestBackend>) -> Vec<String> {
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer.cell((x, y)).unwrap().symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

fn context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

fn recommendation(nanos: i128, description: &str) -> CostRecommendation {
    CostRecommendation {
        name: description.to_string(),
        description: description.to_string(),
        state: RecommendationState::Active,
        priority: Some("P2".to_string()),
        xor_group: None,
        monthly: Some(Money {
            currency: "USD".to_string(),
            nanos,
        }),
        last_refresh: None,
        target_resources: Vec::new(),
    }
}

fn render(app: AppState) -> String {
    render_at(app, 40)
}

/// Taller renders exist for the row-cut test: four cards share the body, so at
/// an ordinary height a card holds only a few lines and clips the rest.
fn render_at(app: AppState, height: u16) -> String {
    let session = RuntimeSession::new_for_test(app);
    let mut terminal = Terminal::new(TestBackend::new(120, height)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    lines(&terminal).join("\n")
}

fn finops_app() -> AppState {
    let mut app = AppState::new();
    app.root.workspace = Workspace::FinOps;
    app
}

/// An unprobed workspace must not paint a currency figure anywhere: a
/// "USD 0.00" here is the false good news the whole feature is built to avoid.
#[test]
fn an_unprobed_workspace_renders_no_figure_and_says_what_to_do() {
    let rendered = render(finops_app());

    // Twice: once as the workspace total, once per card. Asserting the count
    // rather than mere presence is what makes this bite — "not probed" reaches
    // the screen from the total line alone, so `contains` would pass with every
    // card headline blank.
    assert_eq!(
        rendered.matches("not probed").count(),
        1 + RecommenderId::ALL.len() + 1,
        "the total line, one headline per card, and the budget block: {rendered}"
    );
    assert!(!rendered.contains("0.00"), "{rendered}");
    assert!(
        rendered.to_lowercase().contains("resources"),
        "the reason names what would make it probe: {rendered}"
    );
    for recommender in RecommenderId::ALL {
        assert!(
            rendered.contains(recommender.label()),
            "missing the {} card",
            recommender.label()
        );
    }
}

/// A card that answered in one zone and was refused in another renders its
/// figure AND its coverage, on the same screen.
#[test]
fn a_partially_covered_card_renders_its_total_beside_its_gap() {
    let mut app = finops_app();
    let probes = app.root.finops.begin_refresh(
        context(),
        &["europe-west1-b".to_string(), "us-central1-a".to_string()],
    );
    for (recommender, location, ticket) in probes {
        let cell = if location.starts_with("europe") {
            CellState::Ready(vec![recommendation(
                12_340_000_000,
                "Save cost by stopping idle VM",
            )])
        } else {
            CellState::Unavailable {
                kind: GcloudErrorKind::PermissionDenied,
                detail: None,
            }
        };
        app.root
            .finops
            .install_cell(ticket, recommender, &location, cell);
    }

    let rendered = render(app);
    assert!(rendered.contains("USD 12.34"), "{rendered}");
    assert!(rendered.contains("us-central1-a"), "{rendered}");
    assert!(rendered.contains("permission denied"), "{rendered}");
}

/// A disabled API renders the enable remedy rather than an IAM audit, all the
/// way to the screen.
#[test]
fn a_disabled_api_renders_the_enable_remedy() {
    let mut app = finops_app();
    let probes = app
        .root
        .finops
        .begin_refresh(context(), &["europe-west1-b".to_string()]);
    for (recommender, location, ticket) in probes {
        app.root.finops.install_cell(
            ticket,
            recommender,
            &location,
            CellState::Unavailable {
                kind: GcloudErrorKind::ApiDisabled,
                detail: None,
            },
        );
    }

    let rendered = render(app);
    assert!(rendered.contains("API disabled"), "{rendered}");
    assert!(rendered.to_lowercase().contains("enable"), "{rendered}");
}

/// The workspace paints its own name in the shared chrome, so the operator can
/// tell where they are.
#[test]
fn the_frame_names_the_finops_workspace() {
    let rendered = render(finops_app());
    assert!(rendered.contains("FinOps"), "{rendered}");
}

/// Each card state names itself in its own headline, in words that appear
/// nowhere else on the screen. A blank headline reads as "nothing to save",
/// which is the one thing a cost card must never say by accident.
#[test]
fn each_card_state_names_itself_in_the_headline() {
    let mut app = finops_app();
    let probes = app
        .root
        .finops
        .begin_refresh(context(), &["europe-west1-b".to_string()]);
    for (index, (recommender, location, ticket)) in probes.into_iter().enumerate() {
        let cell = match index {
            0 => CellState::Ready(vec![recommendation(1_000_000_000, "a")]),
            1 => CellState::Empty,
            2 => CellState::Unavailable {
                kind: GcloudErrorKind::Quota,
                detail: None,
            },
            _ => continue,
        };
        app.root
            .finops
            .install_cell(ticket, recommender, &location, cell);
    }

    let rendered = render(app);
    assert!(
        rendered.contains("USD 1.00/mo"),
        "ready headline: {rendered}"
    );
    assert!(
        rendered.contains("no recommendations"),
        "empty headline: {rendered}"
    );
    assert!(
        rendered.contains("unavailable: quota"),
        "unavailable headline: {rendered}"
    );
    assert!(
        rendered.contains("loading…"),
        "the untouched card is still in flight: {rendered}"
    );
}

/// The row list is cut at five, and the headline says so. Exactly five is not
/// cut — an off-by-one there would claim a card is showing 5 of 5, which is
/// noise, or hide the sixth row with nothing on screen to say it exists.
#[test]
fn a_card_cuts_its_rows_at_five_and_says_how_many_it_dropped() {
    let render_with = |count: usize| {
        let mut app = finops_app();
        let probes = app
            .root
            .finops
            .begin_refresh(context(), &["europe-west1-b".to_string()]);
        let (recommender, location, ticket) = probes[0].clone();
        let rows: Vec<_> = (0..count)
            .map(|index| recommendation(1_000_000_000, &format!("row-{index}")))
            .collect();
        app.root
            .finops
            .install_cell(ticket, recommender, &location, CellState::Ready(rows));
        render_at(app, 60)
    };

    let exactly_five = render_with(5);
    assert!(exactly_five.contains("row-4"), "{exactly_five}");
    assert!(
        !exactly_five.contains("showing"),
        "five rows fit, so nothing was dropped: {exactly_five}"
    );

    let six = render_with(6);
    assert!(six.contains("row-4"), "{six}");
    assert!(six.contains("showing 5 of 6"), "{six}");
    assert!(
        !six.contains("row-5"),
        "the sixth row is the dropped one: {six}"
    );
}

/// The budget block reaches the screen with the amount, the thresholds and the
/// scope — and no gauge, because the API returns no spend to draw one against.
#[test]
fn the_budget_block_renders_amount_thresholds_and_scope() {
    use nuvem::app::state::finops::BudgetsState;
    use nuvem::services::resources::finops::Budget;

    let mut app = finops_app();
    app.root.finops.begin_refresh(context(), &[]);
    let ticket = app.root.finops.current_ticket();
    app.root.finops.install_budgets(
        ticket,
        BudgetsState::Ready(vec![Budget {
            display_name: "Monthly cap".to_string(),
            amount: Some(Money {
                currency: "EUR".to_string(),
                nanos: 500_000_000_000,
            }),
            matches_last_period: false,
            thresholds: vec![50, 100],
            covers_whole_account: true,
        }]),
    );

    let rendered = render(app);
    assert!(rendered.contains("Budgets"), "{rendered}");
    assert!(rendered.contains("Monthly cap"), "{rendered}");
    assert!(rendered.contains("EUR 500.00"), "{rendered}");
    assert!(rendered.contains("alerts at 50%, 100%"), "{rendered}");
    assert!(rendered.contains("billing account"), "{rendered}");
}

/// A project nothing is billed against says so, rather than showing an empty
/// budget list that reads as "nobody set one".
#[test]
fn a_project_with_no_billing_account_says_so_in_the_budget_block() {
    use nuvem::app::state::finops::BudgetsState;

    let mut app = finops_app();
    app.root.finops.begin_refresh(context(), &[]);
    let ticket = app.root.finops.current_ticket();
    app.root
        .finops
        .install_budgets(ticket, BudgetsState::NoBillingAccount);

    let rendered = render(app);
    assert!(
        rendered.contains("no billing account is linked to this project"),
        "{rendered}"
    );
}

/// An account with no budgets is a different sentence again.
#[test]
fn an_account_with_no_budgets_says_none_are_set() {
    use nuvem::app::state::finops::BudgetsState;

    let mut app = finops_app();
    app.root.finops.begin_refresh(context(), &[]);
    let ticket = app.root.finops.current_ticket();
    app.root
        .finops
        .install_budgets(ticket, BudgetsState::Ready(Vec::new()));

    let rendered = render(app);
    assert!(
        rendered.contains("no budgets set on this billing account"),
        "{rendered}"
    );
}

/// The inventory panel counts only services that have actually been listed.
/// Summing unlisted ones would state a project size derived mostly from what
/// nobody has opened yet.
#[test]
fn the_inventory_panel_counts_only_listed_services() {
    let nothing_listed = render(finops_app());
    assert!(nothing_listed.contains("Inventory"), "{nothing_listed}");
    assert!(
        nothing_listed.contains("nothing listed yet"),
        "{nothing_listed}"
    );

    let mut app = finops_app();
    app.root.resources.set_instances(
        context(),
        vec![
            nuvem_domain::gcp::resource::ComputeInstance {
                id: "1".to_string(),
                name: "vm-1".to_string(),
                zone: "europe-west1-b".to_string(),
                status: nuvem_domain::gcp::resource::ResourceStatus::Running,
                machine_type: "e2-medium".to_string(),
                internal_ip: None,
                external_ip: None,
            },
            nuvem_domain::gcp::resource::ComputeInstance {
                id: "2".to_string(),
                name: "vm-2".to_string(),
                zone: "europe-west1-b".to_string(),
                status: nuvem_domain::gcp::resource::ResourceStatus::Running,
                machine_type: "e2-medium".to_string(),
                internal_ip: None,
                external_ip: None,
            },
        ],
    );

    let one_listed = render(app);
    assert!(
        one_listed.contains("2 resource(s) across 1 of"),
        "{one_listed}"
    );
}
