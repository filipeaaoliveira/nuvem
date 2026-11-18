use nuvem::app::state::operations::OperationOutcome;
use nuvem::app::{AppError, AppState, EffectResult};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::ui::home::{CONTEXT_CARD_MIN_HEIGHT, MIN_CARD_HEIGHT};
use nuvem::ui::shared::frame::split_frame;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::diagnostics::{DiagnosticProblem, GcloudDiagnosticsReport, GcloudErrorKind};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceServiceKind, ResourceStatus};
use ratatui::layout::Rect;
use ratatui::style::Color;
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

/// Reads the renderer's `+N more` overflow marker off the painted screen.
///
/// Anchored on the whole `+N more` shape rather than a scan for the first `+`
/// on screen: the frame chrome, a resource label or an operation line may carry
/// a `+`, and a looser parse would read a digit out of one of those and call
/// the accounting satisfied. Panics on a second marker so a stray one cannot be
/// mistaken for the band's.
fn overflow_count(rendered: &str) -> usize {
    let mut found: Option<usize> = None;
    for row in rendered.lines() {
        // Border glyphs abut the content, so `│+3 more` would tokenize as one
        // word; blanking them leaves the paragraph's own text.
        let row = row.replace(['│', '┃'], " ");
        let tokens: Vec<&str> = row.split_whitespace().collect();
        for pair in tokens.windows(2) {
            if pair[1] != "more" {
                continue;
            }
            let Some(count) = pair[0]
                .strip_prefix('+')
                .and_then(|digits| digits.parse::<usize>().ok())
            else {
                continue;
            };
            assert!(
                found.is_none(),
                "more than one overflow marker on screen:\n{rendered}"
            );
            found = Some(count);
        }
    }
    found.unwrap_or(0)
}

/// Every service in the registry reaches the screen when there is room.
///
/// Painted output, not the projection: the projection was already correct for
/// Cloud SQL and Cloud Run, both of which were built on every frame and then
/// dropped by a two-column split that swallowed every index past 1.
#[tokio::test]
async fn every_service_kind_is_painted_on_a_roomy_terminal() {
    let session = RuntimeSession::new_for_test(AppState::new());
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered = lines(&terminal).join("\n");

    for kind in ResourceServiceKind::ALL {
        assert!(
            rendered.contains(kind.label()),
            "{kind:?} has no Home inventory line; rendered:\n{rendered}"
        );
    }
    assert_eq!(
        overflow_count(&rendered),
        0,
        "nothing was left out, so nothing may be counted as missing; rendered:\n{rendered}"
    );
}

/// Where the band is too short, the services left out are counted on screen.
/// The defect this replaces was silent: `columns.get(index)` returned `None`
/// and the `if let Some` swallowed it, with no panic, warning or test.
#[tokio::test]
async fn services_that_do_not_fit_are_counted_not_dropped() {
    let session = RuntimeSession::new_for_test(AppState::new());
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered = lines(&terminal).join("\n");

    let painted = ResourceServiceKind::ALL
        .into_iter()
        .filter(|kind| rendered.contains(kind.label()))
        .count();
    let overflow = overflow_count(&rendered);

    assert!(
        overflow > 0,
        "this terminal is too short for every service, so the band must say so; \
         rendered:\n{rendered}"
    );
    assert_eq!(
        painted + overflow,
        ResourceServiceKind::ALL.len(),
        "{painted} painted plus {overflow} counted does not account for every \
         service; rendered:\n{rendered}"
    );
}

/// The accounting holds at every height the band is drawn at, not just at the
/// two the tests above happen to pick.
///
/// The boundary heights are where an off-by-one lives: the height that fits
/// exactly every service, and the one below it where a line must be given back
/// to the count. 17 rows is the shortest terminal that leaves the band its two
/// borders plus a content row (frame chrome takes 6, the context card 8);
/// shorter than that the band is not drawn at all and there is nothing on
/// screen to account with.
#[tokio::test]
async fn every_terminal_height_accounts_for_every_service() {
    let session = RuntimeSession::new_for_test(AppState::new());
    for height in 17..=40 {
        let mut terminal = Terminal::new(TestBackend::new(120, height)).unwrap();
        terminal
            .draw(|frame| nuvem::ui::render(frame, &session))
            .unwrap();
        let rendered = lines(&terminal).join("\n");

        let painted = ResourceServiceKind::ALL
            .into_iter()
            .filter(|kind| rendered.contains(kind.label()))
            .count();
        let overflow = overflow_count(&rendered);
        assert_eq!(
            painted + overflow,
            ResourceServiceKind::ALL.len(),
            "at height {height}, {painted} painted plus {overflow} counted does not \
             account for every service; rendered:\n{rendered}"
        );
    }
}

/// The band grows into slack only, never into the operations card.
///
/// The sweep above holds under a band that takes every row it wants: at
/// heights just above the shortest one that draws the band, nine rows of
/// inventory leave the operations card under `MIN_CARD_HEIGHT`, so it is not
/// drawn at all — every service is accounted for on a screen that has silently
/// lost a whole card. The threshold is derived, not picked: the frame chrome
/// takes whatever `split_frame` does not leave as body, and the body then owes
/// the context card its fixed height plus a minimum card each to the band and
/// to operations.
#[tokio::test]
async fn the_operations_card_survives_every_height_that_can_fit_it() {
    let session = RuntimeSession::new_for_test(AppState::new());
    for height in 17..=40 {
        let body = split_frame(Rect::new(0, 0, 120, height)).body.height;
        if body < CONTEXT_CARD_MIN_HEIGHT + MIN_CARD_HEIGHT + MIN_CARD_HEIGHT {
            continue;
        }
        let mut terminal = Terminal::new(TestBackend::new(120, height)).unwrap();
        terminal
            .draw(|frame| nuvem::ui::render(frame, &session))
            .unwrap();
        let rendered = lines(&terminal).join("\n");

        assert!(
            rendered.contains("Recent operations"),
            "at height {height} the body has room for the operations card, but the \
             inventory band took it; rendered:\n{rendered}"
        );
    }
}

/// A service's status breakdown reaches its line.
///
/// The counts are the whole point of a loaded service's row: without them the
/// line says only that something loaded, which is what "Not loaded" already
/// distinguishes itself from.
#[tokio::test]
async fn a_loaded_service_shows_its_status_breakdown() {
    let mut session = session_with_dev_configuration().await;
    let context = session.app().root.current_context().unwrap();
    session.app_mut().root.resources.set_instances(
        context,
        vec![ComputeInstance {
            id: "a".into(),
            name: "web-1".into(),
            zone: "europe-west1-b".into(),
            status: ResourceStatus::Running,
            machine_type: "e2-medium".into(),
            internal_ip: None,
            external_ip: None,
        }],
    );

    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered = lines(&terminal).join("\n");

    assert!(
        rendered.contains("1 loaded"),
        "the freshness is missing from the service line; rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("Running 1"),
        "the status breakdown is missing from the service line; rendered:\n{rendered}"
    );
}

#[tokio::test]
async fn home_dashboard_shows_context_and_inventory_headings() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: None,
            }],
        })
        .await;

    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered = lines(&terminal).join("\n");
    assert!(rendered.contains("GCE instances"));
    assert!(rendered.contains("GKE clusters"));
    assert!(rendered.contains("dev-project"));
}

async fn session_with_dev_configuration() -> RuntimeSession {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: None,
            }],
        })
        .await;
    session
}

async fn session_with_degraded_gce() -> RuntimeSession {
    let mut session = session_with_dev_configuration().await;
    let context = session.app().root.current_context().unwrap();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoadFailed {
            context,
            error: AppError::gcloud("permission denied: caller lacks compute.instances.list"),
        })
        .await;
    session
}

#[tokio::test]
async fn wide_inventory_card_shows_headline_and_inline_detail_preview() {
    let session = session_with_degraded_gce().await;

    // AC #5: the headline is always visible ("{label} — {user action}"),
    // never the raw category or message alone.
    //
    // 165 columns, not 120: the inventory is one line per service now, so the
    // preview shares the headline's line and is appended only where all of it
    // still fits. Below that the headline keeps the row — see the narrow test.
    //
    // 165 is exactly the threshold for this diagnostic, and the arithmetic is
    // worth writing down because the number moves whenever anything on the row
    // gets wider. `inventory_line` compares `used + headline + preview` against
    // `area.width - 2 borders - SPARKLINE_WIDTH`, and for this card that is
    // 28 + 66 + 57 = 151 against `width - 14`. So 165 fits and 164 does not.
    //
    // It was 160 while the label column was a hard-coded 18. The projection derives
    // that column from the widest `ResourceServiceKind` label instead, which is
    // 21, so every row's label costs five more columns and the preview's
    // threshold moved with it. The preview is explicitly the thing that yields
    // when the row is tight ("a truncated preview spends columns to say
    // nothing"), so paying for the alignment out of its budget is the trade the
    // renderer already says it makes.
    let mut terminal = Terminal::new(TestBackend::new(165, 30)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered = lines(&terminal).join("\n");
    assert!(rendered.contains("permission denied — Check IAM roles for this account and project"));
    // AC #6: at this width the sanitized bounded detail preview is also
    // shown inline, alongside (not instead of) the headline.
    assert!(rendered.contains("caller lacks compute.instances.list"));
}

/// A preview that does not fit whole is not started.
///
/// 140 columns is between the two widths that matter for this diagnostic: the
/// headline still has room to spare, but the label, freshness, headline and
/// preview together need about 147 columns. A preview appended here would be
/// cut mid-sentence by `fit_spans`, spending the row's remaining columns to
/// say part of a word — which is what an arithmetic slip in the fit test buys,
/// and why the test asserts on a width where the line is not otherwise
/// truncated at all.
#[tokio::test]
async fn a_detail_preview_that_does_not_fit_whole_is_not_started() {
    let session = session_with_degraded_gce().await;

    let mut terminal = Terminal::new(TestBackend::new(140, 30)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered = lines(&terminal).join("\n");

    assert!(
        rendered.contains("permission denied — Check IAM roles for this account and project"),
        "the headline must keep the row's columns; rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("caller lacks"),
        "the preview does not fit whole at this width, so no part of it may be \
         painted; rendered:\n{rendered}"
    );
}

#[tokio::test]
async fn narrow_inventory_card_shows_headline_but_hides_inline_detail_preview() {
    let session = session_with_degraded_gce().await;

    // At this width the service line has no room left for the preview once the
    // headline is on it. The headline recovery hint must still be visible — it
    // gets the row's columns first — but the extra technical detail must not be
    // shown (no full overlay panel per the design).
    let mut terminal = Terminal::new(TestBackend::new(60, 30)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered = lines(&terminal).join("\n");
    assert!(rendered.contains("permission denied"));
    assert!(!rendered.contains("caller lacks compute.instances.list"));

    // The headline is cut to fit here, and a truncated headline still has to
    // read as a problem: this is the width where the colour carries the most.
    let buffer = terminal.backend().buffer();
    let inventory_row = (0..buffer.area.height)
        .find(|y| {
            (0..buffer.area.width)
                .map(|x| buffer.cell((x, *y)).unwrap().symbol())
                .collect::<String>()
                .contains("GCE instances")
        })
        .expect("an inventory row for GCE instances");
    let row: String = (0..buffer.area.width)
        .map(|x| {
            buffer
                .cell((x, inventory_row))
                .unwrap()
                .symbol()
                .to_string()
        })
        .collect();
    let headline_x = row.find("permission denied").unwrap() as u16;
    assert_eq!(
        buffer.cell((headline_x, inventory_row)).unwrap().fg,
        Color::Red,
        "the truncated headline lost the error style; row was: {row}"
    );

    // And it has to read as cut, not as finished. A line trimmed span by span
    // ends on the truncation marker; a line the block merely clipped ends on
    // whatever character the border happened to land after, so the operator has
    // no way to tell a shortened hint from the whole of it.
    let content = row.trim_matches(|character| character == '│' || character == '┃');
    assert!(
        content.trim_end().ends_with("..."),
        "the row was cut without saying so; row was: {row}"
    );
}

/// FIX 1 (AC#5): common auth failures return `available: true` alongside a
/// problem (e.g. an empty/failed `gcloud auth list`). The health card must
/// still surface the problem headline in that case, and must not use the
/// healthy/green style when a problem is present.
#[tokio::test]
async fn health_card_surfaces_problem_even_when_available_is_true() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: None,
            }],
        })
        .await;
    session
        .apply_effect_result(EffectResult::GcloudDiagnosticsProbed {
            report: GcloudDiagnosticsReport {
                available: true,
                active_account: None,
                problem: Some(DiagnosticProblem::new(
                    GcloudErrorKind::Unauthenticated,
                    "no active gcloud account",
                )),
            },
        })
        .await;

    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered = lines(&terminal).join("\n");
    assert!(
        rendered.contains("not authenticated") && rendered.contains("gcloud auth login"),
        "rendered output was:\n{rendered}"
    );

    let buffer = terminal.backend().buffer();
    // Locate the line containing the headline explicitly and assert no cell
    // on it is styled green (healthy).
    let mut found_problem_line_not_green = false;
    for y in 0..buffer.area.height {
        let row: String = (0..buffer.area.width)
            .map(|x| buffer.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        if row.contains("not authenticated") {
            let any_green = (0..buffer.area.width)
                .map(|x| buffer.cell((x, y)).unwrap())
                .any(|cell| cell.fg == Color::Green);
            assert!(
                !any_green,
                "problem headline row was healthy-styled (green)"
            );
            found_problem_line_not_green = true;
        }
    }
    assert!(
        found_problem_line_not_green,
        "did not find problem headline row"
    );
}

/// A batch cancellation resolves entries to the terminal states
/// `Cancelled` and `Unknown`. If the Recent operations card painted them with
/// the loading style they would be pixel-identical to `Pending`, so the
/// operator would still read settled work as work in flight — this ticket's
/// defect, re-created at the render layer.
#[tokio::test]
async fn terminal_outcomes_are_not_painted_with_the_loading_style() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    {
        let operations = &mut session.app_mut().root.system.operations;
        operations.begin("Stop instance", "web-pending");
        let cancelled = operations.begin("Stop instance", "web-cancelled");
        operations.resolve_by_id(cancelled, OperationOutcome::Cancelled, None);
        let unknown = operations.begin("Stop instance", "web-unknown");
        operations.resolve_by_id(unknown, OperationOutcome::Unknown, None);
    }

    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();

    // Default-theme role colors (src/config/theme.rs). Naming them keeps the
    // assertions about *semantics* rather than about "some other color": a
    // mutation that swapped Cancelled to the error style and Unknown to the
    // healthy style would still be mutually distinct and still not loading,
    // yet would invert what the operator reads off the card.
    let loading = Color::Yellow;
    let healthy = Color::Green;
    let error = Color::Red;
    let muted = Color::DarkGray;
    let stale = Color::LightYellow;
    let buffer = terminal.backend().buffer();
    let row_fg = |needle: &str| -> Color {
        for y in 0..buffer.area.height {
            let row: String = (0..buffer.area.width)
                .map(|x| buffer.cell((x, y)).unwrap().symbol().to_string())
                .collect();
            if row.contains(needle) {
                let x = row.find(needle).unwrap() as u16;
                return buffer.cell((x, y)).unwrap().fg;
            }
        }
        panic!("did not find a rendered row containing {needle}");
    };

    assert_eq!(
        row_fg("Stop instance web-pending"),
        loading,
        "Pending must keep the loading style"
    );
    let cancelled_fg = row_fg("Stop instance web-cancelled");
    let unknown_fg = row_fg("Stop instance web-unknown");
    assert_ne!(
        cancelled_fg, loading,
        "Cancelled is terminal and must not render as work in progress"
    );
    assert_ne!(
        unknown_fg, loading,
        "Unknown is terminal and must not render as work in progress"
    );
    assert_ne!(
        cancelled_fg, unknown_fg,
        "Cancelled (nothing to do) and Unknown (go re-check) must be distinguishable"
    );
    // Semantics, not just difference. Cancelled must not read as a failure:
    // nothing was issued, so there is nothing that went wrong. Unknown must
    // not read as success: the whole point is that the operator has to go
    // re-check the resource.
    assert_ne!(
        cancelled_fg, error,
        "Cancelled did not fail — it must not render with the error style"
    );
    assert_ne!(
        unknown_fg, healthy,
        "Unknown is not a success — it must not render with the healthy style"
    );
    assert_eq!(cancelled_fg, muted, "Cancelled renders muted/neutral");
    assert_eq!(unknown_fg, stale, "Unknown renders with the warning style");
}

/// The FinOps teaser is painted, not merely projected: it is one line squeezed
/// between the inventory band and the operations card, and a layout that
/// allocated it no height would drop it silently.
#[tokio::test]
async fn the_savings_teaser_is_painted_and_absent_without_a_figure() {
    use nuvem::app::state::finops::CellState;
    use nuvem_domain::gcp::configuration::GcloudContext;
    use nuvem_domain::gcp::finops::{CostRecommendation, RecommendationState};
    use nuvem_domain::gcp::money::Money;

    let paint = |app: AppState| {
        let session = RuntimeSession::new_for_test(app);
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| nuvem::ui::render(frame, &session))
            .unwrap();
        lines(&terminal).join("\n")
    };

    let without = paint(AppState::new());
    assert!(
        !without.contains("Potential savings"),
        "an unprobed FinOps state earns no teaser:\n{without}"
    );

    let mut app = AppState::new();
    let context = GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("proj".to_string()),
        region: None,
        zone: None,
    };
    let probes = app
        .root
        .finops
        .begin_refresh(context, &["europe-west1-b".to_string()]);
    for (recommender, location, ticket) in probes {
        app.root.finops.install_cell(
            ticket,
            recommender,
            &location,
            CellState::Ready(vec![CostRecommendation {
                name: "r".to_string(),
                description: "d".to_string(),
                state: RecommendationState::Active,
                priority: None,
                xor_group: None,
                monthly: Some(Money {
                    currency: "USD".to_string(),
                    nanos: 2_500_000_000,
                }),
                last_refresh: None,
                target_resources: Vec::new(),
            }]),
        );
    }

    let with = paint(app);
    assert!(with.contains("Potential savings: USD 10.00/mo"), "{with}");
    assert!(with.contains("Ctrl-f"), "{with}");
}

/// A terminal too short to grant the teaser its row must still render, not
/// panic: the layout hands back a zero-height rect and ratatui draws nothing
/// into it. This is what makes the removed height guard safe to have removed.
#[tokio::test]
async fn a_terminal_too_short_for_the_teaser_still_renders() {
    use nuvem::app::state::finops::CellState;
    use nuvem_domain::gcp::configuration::GcloudContext;
    use nuvem_domain::gcp::finops::{CostRecommendation, RecommendationState};
    use nuvem_domain::gcp::money::Money;

    let mut app = AppState::new();
    let context = GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("proj".to_string()),
        region: None,
        zone: None,
    };
    let probes = app
        .root
        .finops
        .begin_refresh(context, &["europe-west1-b".to_string()]);
    for (recommender, location, ticket) in probes {
        app.root.finops.install_cell(
            ticket,
            recommender,
            &location,
            CellState::Ready(vec![CostRecommendation {
                name: "r".to_string(),
                description: "d".to_string(),
                state: RecommendationState::Active,
                priority: None,
                xor_group: None,
                monthly: Some(Money {
                    currency: "USD".to_string(),
                    nanos: 2_500_000_000,
                }),
                last_refresh: None,
                target_resources: Vec::new(),
            }]),
        );
    }

    let session = RuntimeSession::new_for_test(app);
    for height in 1..=12 {
        let mut terminal = Terminal::new(TestBackend::new(120, height)).unwrap();
        terminal
            .draw(|frame| nuvem::ui::render(frame, &session))
            .unwrap();
    }
}

/// Where `needle` starts in `haystack`, counted in characters rather than
/// bytes: these rows are bounded by box-drawing glyphs, which are three bytes
/// each, so a byte offset used as a character offset lands mid-row.
fn char_index_of(haystack: &str, needle: &str) -> usize {
    let byte = haystack.find(needle).expect("row was found by this needle");
    haystack[..byte].chars().count()
}

/// Every inventory label is separated from its freshness text by at least one
/// space, for every service in the registry.
///
/// An ALL-walk rather than a check on the label that happens to be longest
/// today: the defect was a hand-picked `SERVICE_LABEL_WIDTH` of 18 that a
/// later service ("Pub/Sub subscriptions", 21 columns) outgrew, and `{:<N}`
/// pads without truncating, so the freshness text started flush against the
/// label with no gap. A test pinned to the current longest name would go green
/// on the fix and stay green through the next service that outgrows the width.
///
/// Read off the painted buffer, not off `inventory_line`: the gap is a property
/// of what reaches the screen.
#[tokio::test]
async fn every_inventory_label_keeps_a_gap_before_its_freshness() {
    let session = RuntimeSession::new_for_test(AppState::new());
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered = lines(&terminal);

    for kind in ResourceServiceKind::ALL {
        let label = kind.label();
        let row = rendered
            .iter()
            .find(|line| line.contains(label))
            .unwrap_or_else(|| {
                panic!(
                    "{label} has no Home inventory line; rendered:\n{}",
                    rendered.join("\n")
                )
            });
        let start = char_index_of(row, label);
        let after: String = row.chars().skip(start + label.chars().count()).collect();
        assert!(
            after.starts_with(' '),
            "{label} is flush against what follows it: {row:?}"
        );
    }
}

/// The freshness column starts at one screen column for every service, so the
/// band reads as a table rather than as ragged text.
#[tokio::test]
async fn every_inventory_freshness_starts_at_the_same_column() {
    let session = RuntimeSession::new_for_test(AppState::new());
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered = lines(&terminal);

    let mut columns = Vec::new();
    for kind in ResourceServiceKind::ALL {
        let label = kind.label();
        let row = rendered
            .iter()
            .find(|line| line.contains(label))
            .unwrap_or_else(|| panic!("{label} has no Home inventory line"));
        let start = char_index_of(row, label);
        let after: String = row.chars().skip(start + label.chars().count()).collect();
        let gap = after.chars().take_while(|c| *c == ' ').count();
        columns.push((label, start + label.chars().count() + gap));
    }

    let (_, first) = columns[0];
    for (label, column) in &columns {
        assert_eq!(
            *column, first,
            "{label} starts its freshness at column {column}, not {first}; \
             the band is not aligned"
        );
    }
}

/// The rows inside the block whose title bar contains `title`, up to its
/// closing border.
fn card_rows(rendered: &[String], title: &str) -> Vec<String> {
    let top = rendered
        .iter()
        .position(|line| line.contains(title) && line.contains('┌'))
        .unwrap_or_else(|| panic!("no {title} card; rendered:\n{}", rendered.join("\n")));
    rendered[top + 1..]
        .iter()
        .take_while(|line| !line.contains('└'))
        .map(|line| line.trim_matches(['│', ' ']).to_string())
        .collect()
}

/// Home's whole job is the inventory, so a short terminal must still show some
/// of it.
///
/// At 60x18 the band's one content row used to go to `+16 more` and the
/// screen named no service at all. Two rows had gone somewhere that carried
/// nothing — one to the sixth content row the context card reserved
/// unconditionally and then left blank, one to a marker counting what was not
/// shown.
#[tokio::test]
async fn a_short_terminal_still_names_some_of_the_inventory() {
    let session = RuntimeSession::new_for_test(AppState::new());
    let mut terminal = Terminal::new(TestBackend::new(60, 18)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered = lines(&terminal);

    let inventory = card_rows(&rendered, "Inventory").join("\n");
    assert!(
        ResourceServiceKind::ALL
            .into_iter()
            .any(|kind| inventory.contains(kind.label())),
        "the inventory band names no service at all; band:\n{inventory}\n\
         rendered:\n{}",
        rendered.join("\n")
    );
}

/// The context card takes the rows it fills and no more.
///
/// It reserved a sixth content row for the gcloud detail preview whether or not
/// there was one to show, so every Home without a gcloud diagnostic — which is
/// every healthy Home — carried a blank row at every width, and on a short
/// terminal that row was the inventory's.
#[tokio::test]
async fn the_context_card_reserves_no_row_it_leaves_blank() {
    let session = RuntimeSession::new_for_test(AppState::new());
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered = lines(&terminal);

    let context = card_rows(&rendered, "Context");
    assert!(
        !context.is_empty(),
        "the context card painted no rows at all"
    );
    assert!(
        context.iter().all(|row| !row.is_empty()),
        "the context card reserves a row it leaves blank: {context:?}"
    );
}
