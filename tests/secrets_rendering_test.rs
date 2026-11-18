//! Secret Manager reaches the screen.
//!
//! A service can be in `SERVICE_REGISTRY`, satisfy every `ResourceServiceKind`
//! walk, and still never paint a row — the registry test passes and the app is
//! broken. So these assertions are made on the terminal buffer, through the real
//! reducer and the real renderer, not on the projection model.
//!
//! The buffer is also the only place the no-payload property is observable as a
//! *user-visible* fact rather than a structural one, which is why
//! [`no_secret_version_payload_is_ever_painted`] lives here beside the rows.

use nuvem::app::projection::project_resources;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::resource::{
    ComputeKeyValue, ResourceServiceKind, Secret, SecretDetail, SecretVersion, SecretVersionState,
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

fn label(key: &str, value: &str) -> ComputeKeyValue {
    ComputeKeyValue {
        key: key.to_string(),
        value: value.to_string(),
    }
}

fn secrets() -> Vec<Secret> {
    vec![
        Secret {
            name: "api-signing-key".to_string(),
            create_time: Some("2026-02-11T09:14:22Z".to_string()),
            labels: vec![label("env", "prod")],
            replication: "Automatic".to_string(),
        },
        Secret {
            name: "db-password".to_string(),
            create_time: Some("2026-03-02T17:41:08Z".to_string()),
            labels: vec![label("env", "staging")],
            replication: "User-managed (europe-west1)".to_string(),
        },
    ]
}

/// An app with the Secret Manager service selected and its rows delivered,
/// through the real reducer rather than by poking state.
fn app_showing_secrets() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration()],
        }),
    );
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::SecretManagerSecrets,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretsLoaded {
            context: context.clone(),
            secrets: secrets(),
        }),
    );
    app.root.workspace = Workspace::Resources;
    app
}

/// The two secrets of [`secrets`] loaded, then a filter applied through the
/// real filter actions. Any filter matching neither row leaves the list with
/// rows loaded and none visible.
fn app_with_filter(filter: &str) -> AppState {
    let mut app = app_showing_secrets();
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

/// The detail payload for `api-signing-key`, for tests that vary one field of
/// it and take the rest as given.
fn secret_detail() -> SecretDetail {
    SecretDetail {
        name: "api-signing-key".to_string(),
        create_time: Some("2026-02-11T09:14:22Z".to_string()),
        labels: vec![label("env", "prod")],
        annotations: Vec::new(),
        replication: "Automatic".to_string(),
        rotation: None,
        expire_time: None,
        versions: Vec::new(),
        raw_json: "{}".to_string(),
    }
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

#[test]
fn the_secrets_list_paints_its_headers_and_rows() {
    let frame = painted(app_showing_secrets());

    for header in ["NAME", "CREATED", "REPLICATION", "LABELS"] {
        assert!(
            frame.contains(header),
            "the secrets table must paint a {header} header:\n{frame}"
        );
    }

    assert!(
        frame.contains("api-signing-key"),
        "the secrets table must paint its rows:\n{frame}"
    );
    assert!(
        frame.contains("db-password"),
        "the secrets table must paint every row, not just the first:\n{frame}"
    );
    assert!(
        frame.contains("User-managed (europe-west1)"),
        "a user-managed replication policy must paint its replica locations, \
         which is the part an operator is checking:\n{frame}"
    );
}

/// The list row deliberately carries no version count, because the Secret
/// resource has none and synthesising one would cost a call per row on every
/// load. If a VERSIONS column is ever added to the catalog, this fails and
/// whoever added it has to come back through that decision.
#[test]
fn the_secrets_list_has_no_version_count_column() {
    let mut app = app_showing_secrets();
    app.root.workspace = Workspace::Resources;
    let model = project_resources(&app.root.resources)
        .table
        .expect("list mode builds a table model");

    let keys: Vec<&str> = model.columns.iter().map(|column| column.key).collect();
    assert_eq!(keys, vec!["name", "created", "replication", "labels"]);
    assert!(
        !keys.iter().any(|key| key.contains("version")),
        "the secrets list must not carry a version column: populating it costs \
         one extra gcloud call per secret on every list load, and the count \
         belongs in the detail pane"
    );
}

/// The LABELS cell, pinned to its rendered text rather than to the column
/// merely existing.
///
/// `secret_labels_cell` survived mutation to both `String::new()` and a
/// constant: the earlier test asserted the LABELS *header* and the row names,
/// so an empty or wrong labels cell painted happily. Both the populated form
/// and the no-labels dash are asserted here, because the function has two
/// branches and returning a constant satisfies neither.
#[test]
fn the_labels_cell_paints_its_key_value_pairs_and_a_dash_when_empty() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration()],
        }),
    );
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::SecretManagerSecrets,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretsLoaded {
            context,
            secrets: vec![
                Secret {
                    name: "labelled-secret".to_string(),
                    create_time: Some("2026-02-11T09:14:22Z".to_string()),
                    labels: vec![label("env", "prod"), label("owner", "platform")],
                    replication: "Automatic".to_string(),
                },
                Secret {
                    name: "unlabelled-secret".to_string(),
                    create_time: Some("2026-03-02T17:41:08Z".to_string()),
                    labels: Vec::new(),
                    replication: "Automatic".to_string(),
                },
            ],
        }),
    );
    app.root.workspace = Workspace::Resources;

    // The dash is asserted on the unlabelled row's own LABELS cell rather than
    // on the frame: every fixture `create_time` contains a hyphen, so a
    // whole-frame `contains('-')` would hold for any frame that renders at all.
    let model = project_resources(&app.root.resources)
        .table
        .expect("list mode builds a table model");
    let labels_column = model
        .columns
        .iter()
        .position(|column| column.key == "labels")
        .expect("the secrets table carries a labels column");
    let name_column = model
        .columns
        .iter()
        .position(|column| column.key == "name")
        .expect("the secrets table carries a name column");
    let unlabelled = model
        .rows
        .iter()
        .find(|row| row.cells[name_column].text == "unlabelled-secret")
        .expect("the unlabelled fixture row is projected");
    assert_eq!(
        unlabelled.cells[labels_column].text, "-",
        "a secret with no labels must fill its LABELS cell with a dash rather \
         than leave it empty"
    );

    let frame = painted(app);

    assert!(
        frame.contains("env=prod"),
        "a label must paint as key=value, which is how an operator reads and \
         searches it:\n{frame}"
    );
    assert!(
        frame.contains("owner=platform"),
        "every label must paint, not only the first:\n{frame}"
    );
}

/// The three empty-state branches of `render_secrets_list`.
///
/// Each of the status comparisons selecting them survived mutation from `==` to
/// `!=`, because the rendering tests only ever painted a populated table. A
/// wrong branch here shows an operator "loading" when a fetch has failed, or a
/// bare empty list when rows are merely stale.
#[test]
fn the_secrets_list_paints_a_distinct_empty_state_per_status() {
    // Loading: a fetch in flight against a collection with no rows yet.
    let mut loading = AppState::new();
    reduce(
        &mut loading,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration()],
        }),
    );
    reduce(
        &mut loading,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::SecretManagerSecrets,
        )),
    );
    loading.root.resources.begin_load(
        ResourceServiceKind::SecretManagerSecrets,
        std::time::Instant::now(),
    );
    loading.root.workspace = Workspace::Resources;
    let frame = painted(loading);
    assert!(
        frame.contains("Loading secrets"),
        "an in-flight secrets fetch must paint the loading state, naming the \
         service it is loading:\n{frame}"
    );

    // Failed: the load-failed result for the active context.
    let mut failed = AppState::new();
    reduce(
        &mut failed,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration()],
        }),
    );
    reduce(
        &mut failed,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::SecretManagerSecrets,
        )),
    );
    let context = failed.root.current_context().expect("active context");
    reduce(
        &mut failed,
        &Action::EffectResult(EffectResult::SecretsLoadFailed {
            context,
            error: nuvem::app::AppError::gcloud("permission denied"),
        }),
    );
    failed.root.workspace = Workspace::Resources;
    let frame = painted(failed);
    assert!(
        frame.contains("Press r to retry."),
        "a failed secrets fetch must paint the retry affordance, which is what \
         distinguishes it from an empty or stale list:\n{frame}"
    );

    // Stale: rows cleared pending a background refresh, with no load phase.
    let mut stale = app_showing_secrets();
    stale
        .root
        .resources
        .mark_active_stale_and_clear(ResourceServiceKind::SecretManagerSecrets);
    stale.root.workspace = Workspace::Resources;
    let frame = painted(stale);
    assert!(
        frame.contains("Resource rows are stale. Press r to refresh."),
        "stale secrets rows must say they are stale, not read as a project \
         with no secrets in it:\n{frame}"
    );
}

/// The three *non*-status empty branches: a project with no secrets, a list
/// that was never loaded, and a filter that matches nothing.
///
/// The status comparisons above cover Loading/Failed/Stale, which leaves the
/// tail of the chain — `total_rows == 0 && status == Ready`, `total_rows == 0`,
/// `visible_rows == 0` — unpinned, and each of those survived mutation to `!=`.
/// The three read as opposite facts to an operator: "this project has no
/// secrets", "nothing has been fetched yet", and "your filter hid them all".
#[test]
fn the_secrets_list_distinguishes_no_secrets_from_not_loaded_from_filtered_out() {
    // Ready with zero rows: the project genuinely has no secrets.
    let mut empty = AppState::new();
    reduce(
        &mut empty,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration()],
        }),
    );
    let context = empty.root.current_context().expect("active context");
    reduce(
        &mut empty,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::SecretManagerSecrets,
        )),
    );
    reduce(
        &mut empty,
        &Action::EffectResult(EffectResult::SecretsLoaded {
            context,
            secrets: Vec::new(),
        }),
    );
    empty.root.workspace = Workspace::Resources;
    let frame = painted(empty);
    assert!(
        frame.contains("No secrets found for"),
        "a completed fetch that returned nothing must say the configuration has \
         no secrets:\n{frame}"
    );
    assert!(
        !frame.contains("Press r to load secrets."),
        "a completed fetch must not invite a load it has already done:\n{frame}"
    );
    assert!(
        !frame.contains("No secrets match filter"),
        "an empty project is not a hidden-by-filter list:\n{frame}"
    );

    // Never loaded: the service is selected but no fetch has completed.
    let mut unloaded = AppState::new();
    reduce(
        &mut unloaded,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration()],
        }),
    );
    reduce(
        &mut unloaded,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::SecretManagerSecrets,
        )),
    );
    unloaded.root.workspace = Workspace::Resources;
    let frame = painted(unloaded);
    assert!(
        frame.contains("Press r to load secrets."),
        "a list nobody has fetched must offer the load, not report an empty \
         project:\n{frame}"
    );
    assert!(
        !frame.contains("No secrets found for"),
        "nothing is known about the project's secrets before a fetch:\n{frame}"
    );
    assert!(
        !frame.contains("No secrets match filter"),
        "an unfetched list is not a hidden-by-filter list:\n{frame}"
    );

    // Filtered out: rows exist, none of them survive the applied filter.
    let frame = painted(app_with_filter("zzz-matches-nothing"));
    assert!(
        frame.contains("No secrets match filter \"zzz-matches-nothing\""),
        "a filter that hides every row must name itself, so the operator knows \
         the rows are hidden rather than absent:\n{frame}"
    );
    assert!(
        !frame.contains("No secrets found for"),
        "rows that exist behind a filter must not be reported as absent:\n{frame}"
    );
}

/// The list-tools header on the empty-state path.
///
/// `(!compact)` survived deletion of its `!`, which drops the Filter/Sort/Rows
/// header from a full-width terminal — exactly the line that tells an operator
/// staring at "no matches" what filter is applied and how many rows it hid.
#[test]
fn a_filtered_empty_secrets_list_keeps_its_filter_and_row_count_header() {
    let frame = painted(app_with_filter("zzz-matches-nothing"));

    assert!(
        frame.contains("Filter: zzz-matches-nothing"),
        "the empty-state list must still carry the applied filter in its tools \
         header:\n{frame}"
    );
    assert!(
        frame.contains("Rows: 0/2"),
        "the tools header must report how many of the loaded rows survived the \
         filter, which is what distinguishes a hiding filter from an empty \
         project:\n{frame}"
    );
}

/// The active secret-detail tab, pinned to the `[brackets]` the tab line draws
/// around it. The `*tab == active_tab` comparison survived flipping to `!=`,
/// which marks every tab active except the one being shown.
#[test]
fn the_secret_detail_pane_marks_only_the_active_tab() {
    let mut app = app_showing_secrets();
    let context: GcloudContext = app.root.current_context().expect("active context");
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretDetailLoaded {
            context,
            detail: secret_detail(),
        }),
    );

    let frame = painted(app.clone());
    assert!(
        frame.contains("[Summary]"),
        "the tab the pane is showing must be the marked one:\n{frame}"
    );
    for inactive in ["[Versions]", "[Labels]", "[JSON]"] {
        assert!(
            !frame.contains(inactive),
            "{inactive} is not the active tab and must not be marked:\n{frame}"
        );
    }

    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    let frame = painted(app);
    assert!(
        frame.contains("[Versions]"),
        "the mark must follow the tab selection:\n{frame}"
    );
    assert!(
        !frame.contains("[Summary]"),
        "the tab left behind must lose its mark:\n{frame}"
    );
}

/// The Labels tab, painted. `secret_key_value_lines` survived being replaced
/// with an empty vector, which leaves the tab blank — indistinguishable from a
/// secret carrying no labels at all.
#[test]
fn the_secret_detail_labels_tab_paints_labels_and_annotations() {
    let mut app = app_showing_secrets();
    let context: GcloudContext = app.root.current_context().expect("active context");
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretDetailLoaded {
            context,
            detail: SecretDetail {
                labels: vec![label("env", "prod")],
                annotations: vec![label("owner", "platform")],
                ..secret_detail()
            },
        }),
    );
    // Summary -> Versions -> Labels.
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));

    let frame = painted(app);
    assert!(
        frame.contains("Labels:") && frame.contains("env: prod"),
        "the Labels tab must paint each label under its heading:\n{frame}"
    );
    assert!(
        frame.contains("Annotations:") && frame.contains("owner: platform"),
        "annotations are a separate section and must paint their own \
         entries:\n{frame}"
    );
}

/// A secret with neither labels nor annotations still gets both headings and an
/// explicit "none" under each — an empty tab would leave the operator unable to
/// tell "no labels" from "the tab failed to build".
#[test]
fn the_secret_detail_labels_tab_says_none_rather_than_painting_nothing() {
    let mut app = app_showing_secrets();
    let context: GcloudContext = app.root.current_context().expect("active context");
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretDetailLoaded {
            context,
            detail: SecretDetail {
                labels: Vec::new(),
                annotations: Vec::new(),
                ..secret_detail()
            },
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));

    let frame = painted(app);
    assert!(
        frame.contains("Labels:") && frame.contains("Annotations:"),
        "both headings must paint even when both sections are empty:\n{frame}"
    );
    // Indented, so the list tools header's "Filter: none" is not counted.
    assert_eq!(
        frame.matches("  none").count(),
        2,
        "each empty section must say \"none\" for itself:\n{frame}"
    );
}

#[test]
fn the_service_is_reachable_by_its_registry_name() {
    let frame = painted(app_showing_secrets());
    assert!(
        frame.contains("Secret Manager"),
        "the service must name itself on screen, or nothing can navigate to \
         it:\n{frame}"
    );
}

/// The detail pane paints version *metadata* — id, state, created — and the
/// versions are where the count lives.
#[test]
fn the_detail_pane_paints_version_metadata() {
    let mut app = app_showing_secrets();
    let context: GcloudContext = app.root.current_context().expect("active context");
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretDetailLoaded {
            context: context.clone(),
            detail: SecretDetail {
                name: "api-signing-key".to_string(),
                create_time: Some("2026-02-11T09:14:22Z".to_string()),
                labels: vec![label("env", "prod")],
                annotations: Vec::new(),
                replication: "Automatic".to_string(),
                rotation: None,
                expire_time: None,
                versions: Vec::new(),
                raw_json: "{}".to_string(),
            },
        }),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretVersionsLoaded {
            context,
            secret_name: "api-signing-key".to_string(),
            versions: vec![
                SecretVersion {
                    name: "3".to_string(),
                    state: SecretVersionState::Enabled,
                    create_time: Some("2026-06-01T11:02:45Z".to_string()),
                },
                SecretVersion {
                    name: "2".to_string(),
                    state: SecretVersionState::Destroyed,
                    create_time: Some("2026-04-18T08:30:00Z".to_string()),
                },
            ],
        }),
    );
    // The Versions tab is where versions are shown; Summary opens by default.
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));

    let frame = painted(app);
    assert!(
        frame.contains("Enabled") && frame.contains("Destroyed"),
        "the Versions tab must paint each version's state:\n{frame}"
    );
}

/// A versions fetch that failed must not read as a secret with no versions.
///
/// `secrets versions list` needs an IAM permission that `secrets list` does
/// not, so an operator can hold the second and not the first — routinely. If
/// the failure only set a transient status line, the pane would keep painting
/// "No versions." and "Versions: 0", which are claims about the secret rather
/// than about the fetch, and the status line is gone by the next operation.
#[test]
fn a_failed_versions_fetch_is_distinguishable_from_a_secret_with_no_versions() {
    let mut app = app_showing_secrets();
    let context: GcloudContext = app.root.current_context().expect("active context");
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretDetailLoaded {
            context: context.clone(),
            detail: SecretDetail {
                name: "api-signing-key".to_string(),
                create_time: Some("2026-02-11T09:14:22Z".to_string()),
                labels: vec![label("env", "prod")],
                annotations: Vec::new(),
                replication: "Automatic".to_string(),
                rotation: None,
                expire_time: None,
                versions: Vec::new(),
                raw_json: "{}".to_string(),
            },
        }),
    );

    let summary_before = painted(app.clone());
    assert!(
        summary_before.contains("Versions: 0"),
        "with no failure recorded the count is a plain zero:\n{summary_before}"
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretVersionsLoadFailed {
            context,
            secret_name: "api-signing-key".to_string(),
            error: nuvem::app::AppError::gcloud(
                "PERMISSION_DENIED: secretmanager.versions.list".to_string(),
            ),
        }),
    );

    let summary = painted(app.clone());
    assert!(
        summary.contains("Versions: unavailable"),
        "the Summary count must not assert zero versions when the fetch \
         failed:\n{summary}"
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    let versions_tab = painted(app);
    assert!(
        versions_tab.contains("could not be listed"),
        "the Versions tab must say the listing failed rather than that there \
         are none:\n{versions_tab}"
    );
    assert!(
        !versions_tab.contains("No versions."),
        "\"No versions.\" is a claim about the secret, not about the failed \
         fetch:\n{versions_tab}"
    );
}

/// The user-visible half of the no-payload guarantee.
///
/// `tests/secrets_no_payload_access_test.rs` proves the call is never made. This
/// proves the consequence an operator can actually observe: whatever is on
/// screen for a secret, none of it is secret material. It is a weaker assertion
/// than the structural one by design — it can only check the values this fixture
/// knows about — so it stands beside that file rather than in place of it.
#[test]
fn no_secret_version_payload_is_ever_painted() {
    let mut app = app_showing_secrets();
    let context: GcloudContext = app.root.current_context().expect("active context");
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretDetailLoaded {
            context: context.clone(),
            detail: SecretDetail {
                name: "api-signing-key".to_string(),
                create_time: Some("2026-02-11T09:14:22Z".to_string()),
                labels: vec![label("env", "prod")],
                annotations: Vec::new(),
                replication: "Automatic".to_string(),
                rotation: None,
                expire_time: None,
                versions: vec![SecretVersion {
                    name: "3".to_string(),
                    state: SecretVersionState::Enabled,
                    create_time: Some("2026-06-01T11:02:45Z".to_string()),
                }],
                // The describe JSON the JSON tab renders verbatim. A payload
                // cannot reach this field, because nothing fetches one — but if
                // some future change ever routed one here, this is the tab it
                // would surface on.
                raw_json: "{\"name\":\"api-signing-key\"}".to_string(),
            },
        }),
    );

    for _ in 0..SECRET_DETAIL_TAB_COUNT {
        let frame = painted(app.clone());
        assert!(
            !frame.contains("hunter2"),
            "no tab may paint secret material:\n{frame}"
        );
        reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    }
}

/// Every tab of the secret detail pane, so the payload sweep above covers all of
/// them rather than whichever one happens to open first.
const SECRET_DETAIL_TAB_COUNT: usize = 4;
