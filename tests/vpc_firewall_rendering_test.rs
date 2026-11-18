//! The firewall rules table reaches the screen (slice 3).
//!
//! Asserted on the terminal buffer through the real reducer and renderer, for
//! the reason the networks rendering test records: a service can satisfy every
//! `ResourceServiceKind` walk, hold rows, and still paint nothing.
//!
//! Two claims carry this file, and both are about a row that is not what it
//! looks like. `ACTION` must reach the screen as a word, because the API
//! reports a *key* and no raw field spells `ALLOW` or `DENY`; and the disabled
//! state must reach it on the identity column, because a disabled DENY rule
//! reads exactly like a live one in every other cell while traffic an operator
//! believes is blocked is flowing.
//!
//! ## Fixture provenance
//!
//! The `default-allow-*` shape is a live capture (2026-08-25). Every other row
//! below is CONSTRUCTED from Google's published `compute/v1/firewalls`
//! reference: no project this account can reach has an EGRESS rule, a `denied`
//! rule, a target-tagged rule, a service-account-scoped rule or a disabled
//! rule. The two whose misrendering is dangerous — DENY and disabled — are
//! therefore both transcribed rather than captured, which is exactly why they
//! are asserted on the buffer here rather than trusted from the parser.

use nuvem::app::projection::columns::{FIREWALL_RULE_COLUMNS, catalog_meta};
use nuvem::app::projection::table::CellKind;
use nuvem::app::projection::{project_resource_preview, project_resources};
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::command::CommandDescriptor;
use nuvem::app::state::resources::DISABLED_BADGE;
use nuvem::app::state::root::Workspace;
use nuvem::app::{
    Action, AppError, AppState, CommandAction, Effect, EffectResult, ResourcesAction,
};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::resource::{
    ComputeInstance, FirewallAction, FirewallDirection, FirewallRule, FirewallTargets,
    ResourceServiceKind, ResourceStatus,
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

/// The captured shape plus the three constructed rows, so no column of this
/// table is the same on every row.
fn rules() -> Vec<FirewallRule> {
    vec![
        // Captured shape: INGRESS, ALLOW, unscoped, source range.
        FirewallRule {
            name: "default-allow-icmp".to_string(),
            network: "default".to_string(),
            direction: FirewallDirection::Ingress,
            priority: Some(65534),
            action: FirewallAction::Allow,
            targets: FirewallTargets::AllInstances,
            source_or_dest: vec!["0.0.0.0/0".to_string()],
            disabled: false,
        },
        // CONSTRUCTED: the DENY half of the discrimination, and the only
        // EGRESS row. Its `source_or_dest` is a destination range.
        FirewallRule {
            name: "deny-all-egress".to_string(),
            network: "default".to_string(),
            direction: FirewallDirection::Egress,
            priority: Some(65535),
            action: FirewallAction::Deny,
            targets: FirewallTargets::AllInstances,
            source_or_dest: vec!["0.0.0.0/0".to_string()],
            disabled: false,
        },
        // CONSTRUCTED: target tags, so the TARGETS cell has something to paint
        // other than the unscoped phrase.
        FirewallRule {
            name: "allow-web-to-tagged".to_string(),
            network: "default".to_string(),
            direction: FirewallDirection::Ingress,
            priority: Some(1000),
            action: FirewallAction::Allow,
            targets: FirewallTargets::Tags(vec!["web".to_string()]),
            source_or_dest: vec!["10.0.0.0/8".to_string()],
            disabled: false,
        },
        // CONSTRUCTED: disabled, service-account scoped, and reporting no
        // ranges at all — every optional cell of this table at once.
        FirewallRule {
            name: "disabled-sa-scoped-ssh".to_string(),
            network: "default".to_string(),
            direction: FirewallDirection::Ingress,
            priority: Some(900),
            action: FirewallAction::Allow,
            targets: FirewallTargets::Tags(vec![
                "bastion@nuvem-fixture-project.iam.gserviceaccount.com".to_string(),
            ]),
            source_or_dest: Vec::new(),
            disabled: true,
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
            ResourceServiceKind::FirewallRules,
        )),
    );
    app.root.workspace = Workspace::Resources;
    app
}

fn app_showing_rules() -> AppState {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::FirewallRulesLoaded {
            context,
            rules: rules(),
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
/// rule's own values, so a `contains` over the whole line would pass on a
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
fn the_catalogue_is_the_seven_agreed_columns_in_order() {
    let keys = FIREWALL_RULE_COLUMNS
        .iter()
        .map(|column| column.spec.key)
        .collect::<Vec<_>>();

    assert_eq!(
        keys,
        vec![
            "name",
            "network",
            "direction",
            "priority",
            "action",
            "targets",
            "source_or_dest"
        ]
    );
}

/// `ACTION` is the second-most protected column, ahead of everything but the
/// identity.
///
/// It carries the whole verdict: whether the rule permits or blocks. A table
/// that dropped it on a narrow terminal would paint a list of rules with no
/// way to tell what any of them does. Asserting the priority rather than the
/// position is what pins the *drop order*, which is the property that matters.
#[test]
fn action_outranks_every_column_but_the_name() {
    let priorities: Vec<(&str, u8)> = FIREWALL_RULE_COLUMNS
        .iter()
        .map(|column| (column.spec.key, column.spec.priority))
        .collect();

    let action = priorities
        .iter()
        .find(|(key, _)| *key == "action")
        .expect("the catalogue has an action column")
        .1;
    let name = priorities
        .iter()
        .find(|(key, _)| *key == "name")
        .expect("the catalogue has a name column")
        .1;

    assert!(
        name < action,
        "the identity column is protected above all others, and it is the one \
         carrying the DISABLED badge"
    );
    for (key, priority) in &priorities {
        if *key == "name" || *key == "action" {
            continue;
        }
        assert!(
            action < *priority,
            "ACTION must outrank {key:?}: a rule list that cannot say whether \
             each rule permits or blocks says nothing at all"
        );
    }
}

#[test]
fn every_column_sorts_because_there_is_no_labels_cell() {
    let unsortable = catalog_meta(ResourceServiceKind::FirewallRules)
        .iter()
        .filter(|column| !column.sortable)
        .map(|column| column.key)
        .collect::<Vec<_>>();

    assert!(unsortable.is_empty(), "got {unsortable:?}");
}

// --- The table ---

#[test]
fn the_table_heads_the_seven_columns() {
    let table = table_text(&painted(app_showing_rules()));

    for header in [
        "NAME",
        "NETWORK",
        "DIRECTION",
        "PRIORITY",
        "ACTION",
        "TARGETS",
        "SOURCE/DEST",
    ] {
        assert!(
            table.contains(header),
            "the table must head {header}, got:\n{table}"
        );
    }
}

/// Both verdicts reach the screen as words.
///
/// This is the assertion the whole service turns on. The API sends an
/// `allowed` key or a `denied` key and never the words; a projection that
/// dropped the action, or defaulted it, would paint a plausible table in which
/// a blocking rule reads as a permitting one.
#[test]
fn both_verdicts_are_painted_as_words_the_api_never_sent() {
    let table = table_text(&painted(app_showing_rules()));

    assert!(
        table.contains("ALLOW"),
        "a permitting rule must say so:\n{table}"
    );
    assert!(
        table.contains("DENY"),
        "and a blocking rule must say so — the API sent a `denied` key, not \
         this word, so nothing but the projection puts it here:\n{table}"
    );
}

/// The disabled rule is badged on its name cell, where no width can drop it.
#[test]
fn a_disabled_rule_wears_its_badge_on_the_identity_column() {
    let painted = painted(app_showing_rules());
    let table = table_text(&painted);

    let badged: Vec<&str> = table_lines(&painted)
        .into_iter()
        .filter(|line| line.contains(DISABLED_BADGE))
        .collect();

    assert_eq!(
        badged.len(),
        1,
        "exactly the one disabled rule is badged:\n{table}"
    );
    assert!(
        badged[0].contains("disabled-sa-scoped-ssh"),
        "and it is that rule's own row: {:?}",
        badged[0]
    );
    let badge_at = badged[0]
        .find(DISABLED_BADGE)
        .expect("the badge is on the line");
    let name_at = badged[0]
        .find("disabled-sa-scoped-ssh")
        .expect("the name is on the line");
    assert!(
        badge_at < name_at,
        "the badge leads the cell: the name column elides from the right, so a \
         trailing badge would be the first thing lost on exactly the rows it \
         exists to mark: {:?}",
        badged[0]
    );
}

/// The live rules carry no badge, which is what makes the badge mean something.
#[test]
fn a_rule_that_is_in_force_carries_no_badge() {
    let painted = painted(app_showing_rules());

    for line in table_lines(&painted) {
        if line.contains("default-allow-icmp") || line.contains("deny-all-egress") {
            assert!(
                !line.contains(DISABLED_BADGE),
                "a live rule must not be badged: {line:?}"
            );
        }
    }
}

/// An unscoped rule says it applies to every instance, rather than blanking.
///
/// An empty `targetTags`/`targetServiceAccounts` is the *widest* scope there
/// is. A blank cell reads as the narrowest, which inverts the fact.
///
/// The table is asserted on the cell's opening rather than on the whole
/// phrase, because `TARGETS` is the second-lowest-priority column and elides
/// at any width a real terminal offers beside the preview pane. The pane,
/// which has the room, is held to the phrase in full. Both halves are needed:
/// the pane alone would pass over a table cell that painted nothing.
#[test]
fn an_unscoped_rule_says_it_reaches_every_instance() {
    let painted = painted(app_showing_rules());
    let table = table_text(&painted);

    let unscoped: Vec<&str> = table_lines(&painted)
        .into_iter()
        .filter(|line| line.contains("default-allow-icmp") || line.contains("deny-all-egress"))
        .collect();
    assert_eq!(
        unscoped.len(),
        2,
        "both unscoped rules are on screen:\n{table}"
    );
    for line in &unscoped {
        assert!(
            line.contains("all in"),
            "an empty target list is the widest scope, not an absent value: \
             {line:?}"
        );
    }

    let mut app = app_showing_rules();
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    let preview = project_resource_preview(&app.root.resources).expect("a row is selected");
    assert_eq!(preview.title, "default-allow-icmp");
    assert!(
        preview
            .lines
            .contains(&("Targets".to_string(), "all instances".to_string())),
        "the pane spells the phrase out where the column elides it: {:?}",
        preview.lines
    );
}

/// The cells are ids, never the self-link URLs the API sent.
#[test]
fn no_cell_paints_a_self_link_url() {
    let table = table_text(&painted(app_showing_rules()));

    assert!(
        !table.contains("googleapis.com") && !table.contains("compute/v1"),
        "NETWORK paints an id, not a self-link:\n{table}"
    );
}

/// A direction the parser could not read dashes rather than painting a word.
///
/// `FirewallDirection::Unspecified` becomes `None` at the projection, which is
/// what makes the cell dash. Rendering it as an empty `Plain` cell would look
/// identical on a buffer and read as a value; falling back to `INGRESS` would
/// be a claim about a rule that reported no direction, and on this table a
/// wrong direction means the SOURCE/DEST column is describing the wrong end of
/// the connection.
#[test]
fn a_rule_with_no_reported_direction_dashes_rather_than_guessing() {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::FirewallRulesLoaded {
            context,
            // CONSTRUCTED, and a shape no capture contains: the API always
            // reports a direction. The arm exists because `from_api` carries
            // absent and empty through rather than defaulting them.
            rules: vec![FirewallRule {
                name: "no-direction-reported".to_string(),
                network: "default".to_string(),
                direction: FirewallDirection::Unspecified,
                priority: Some(1000),
                action: FirewallAction::Allow,
                targets: FirewallTargets::AllInstances,
                source_or_dest: vec!["0.0.0.0/0".to_string()],
                disabled: false,
            }],
        }),
    );

    // Asserted on the cell rather than on the painted line: a `Plain` cell
    // holding the empty string and a `Missing` cell are indistinguishable in a
    // terminal buffer once the surrounding row is stripped, and the row's own
    // name is full of hyphens, so a `contains('-')` over the line would pass
    // on either. `CellKind` is the only place the difference exists.
    let projection = project_resources(&app.root.resources);
    let model = projection.table.as_ref().expect("the table is projected");
    let direction = model
        .columns
        .iter()
        .position(|column| column.key == "direction")
        .expect("the catalogue has a direction column");
    let cell = &model.rows[0].cells[direction];

    assert_eq!(
        cell.kind,
        CellKind::Missing,
        "an unreported direction is an absent value, not an empty word: {cell:?}"
    );
    assert_eq!(cell.text, "-");
}

// --- The empty-state ladder ---

/// A list nobody has fetched offers the load; it does not report an empty
/// project.
///
/// The two are different claims and the ladder in `render_firewall_rules_list`
/// is the only thing that tells them apart. "No firewall rules found for
/// dev-project" over a project that was never queried is the worst possible
/// reading of this table: it says the project has no firewall at all.
#[test]
fn a_table_nobody_has_fetched_offers_the_load_rather_than_claiming_an_empty_project() {
    let unloaded = app_with_context();

    let painted = painted(unloaded);

    assert!(
        painted.contains("Press r to load firewall rules."),
        "a list nobody has fetched must offer the load:\n{painted}"
    );
    assert!(
        !painted.contains("No firewall rules found for"),
        "nothing is known about the project's rules before a fetch, and \
         claiming none exist says the project has no firewall:\n{painted}"
    );
}

/// A filter matching nothing says so, and the tools line still counts what it
/// hid.
///
/// "No firewall rules match filter" and "No firewall rules found for" are the
/// same shape of empty screen and opposite claims — one is about the filter,
/// the other about the project — so both are asserted against the other's
/// wording as well as their own. The row count is on the tools line, which is
/// the only place the hidden rows are reported at all.
#[test]
fn a_filter_matching_nothing_says_so_and_still_counts_the_rows() {
    let mut app = app_showing_rules();
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for character in "zzz-no-such-rule".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(character)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    let painted = painted(app);

    assert!(
        painted.contains("No firewall rules match filter"),
        "a filter with no matches says so:\n{painted}"
    );
    assert!(
        !painted.contains("No firewall rules found for"),
        "a filtered-out list is not an empty project:\n{painted}"
    );
    assert!(
        painted.contains("Rows: 0/4"),
        "the tools line still counts what the filter hid:\n{painted}"
    );
}

// --- The preview ---

/// The preview spells out what the columns abbreviate.
#[test]
fn the_preview_reads_the_selected_rule_out_in_full() {
    let app = app_showing_rules();

    let preview = project_resource_preview(&app.root.resources).expect("a row is selected");

    // Default ordering is by name: allow-web-to-tagged sorts first.
    assert_eq!(preview.title, "allow-web-to-tagged");
    assert!(
        preview
            .lines
            .contains(&("Action".to_string(), "ALLOW".to_string())),
        "the pane names the verdict: {:?}",
        preview.lines
    );
    assert!(
        preview
            .lines
            .contains(&("Direction".to_string(), "INGRESS".to_string())),
        "and the direction: {:?}",
        preview.lines
    );
    assert!(
        preview
            .lines
            .contains(&("Source/dest".to_string(), "10.0.0.0/8".to_string())),
        "and the ranges, which are the other half of what the direction means: \
         {:?}",
        preview.lines
    );
    assert!(
        preview
            .lines
            .contains(&("Targets".to_string(), "web".to_string())),
        "and the scope: {:?}",
        preview.lines
    );
}

/// A populated SOURCE/DEST cell paints its ranges rather than dashing.
///
/// The dashing case is asserted twice already — a rule reporting no ranges, in
/// the preview — and a label that answered `None` for every rule would satisfy
/// both while emptying the column for the whole table. The positive half is
/// what makes those negative assertions mean something.
#[test]
fn a_rule_that_reported_ranges_paints_them() {
    let painted = painted(app_showing_rules());
    let table = table_text(&painted);

    let with_ranges: Vec<&str> = table_lines(&painted)
        .into_iter()
        .filter(|line| line.contains("default-allow-icmp") || line.contains("allow-web-to-tagged"))
        .collect();
    assert_eq!(with_ranges.len(), 2, "both rules are on screen:\n{table}");

    assert!(
        with_ranges
            .iter()
            .any(|line| line.contains("0.0.0.0/0") && line.contains("default-allow-icmp")),
        "the captured rule's source range reaches its own row:\n{table}"
    );
    assert!(
        with_ranges
            .iter()
            .any(|line| line.contains("10.0.0.0/8") && line.contains("allow-web-to-tagged")),
        "and so does the constructed one's:\n{table}"
    );
}

/// The disabled state leads the preview title as well as badging the row.
///
/// The pane is where an operator reads a rule in full, and "this rule is not
/// in force" is the fact that changes what every line under it means.
#[test]
fn the_preview_title_says_a_disabled_rule_is_disabled() {
    let mut app = app_showing_rules();
    // By name: allow-web-to-tagged, default-allow-icmp, deny-all-egress,
    // disabled-sa-scoped-ssh. Three steps reach the disabled row.
    for _ in 0..3 {
        reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    }

    let preview = project_resource_preview(&app.root.resources).expect("a row is selected");

    assert_eq!(
        preview.title,
        format!("disabled-sa-scoped-ssh ({DISABLED_BADGE})")
    );
    assert!(
        preview
            .lines
            .contains(&("Source/dest".to_string(), "-".to_string())),
        "a rule reporting no ranges dashes rather than painting an empty line: \
         {:?}",
        preview.lines
    );
}

/// An action the parser could not derive says so in words rather than dashing.
///
/// The column has no room to explain itself, so it dashes. The pane does have
/// the room, and an operator who sees a dash in the `ACTION` column needs to
/// be told the rule carried neither key — not left to guess which way it
/// failed.
#[test]
fn the_preview_explains_an_underivable_action_where_the_column_can_only_dash() {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::FirewallRulesLoaded {
            context,
            // CONSTRUCTED, and deliberately a shape the API is documented
            // never to send: neither `allowed` nor `denied`.
            rules: vec![FirewallRule {
                name: "neither-key".to_string(),
                network: "default".to_string(),
                direction: FirewallDirection::Ingress,
                priority: Some(1000),
                action: FirewallAction::Unspecified,
                targets: FirewallTargets::AllInstances,
                source_or_dest: vec!["0.0.0.0/0".to_string()],
                disabled: false,
            }],
        }),
    );

    let preview = project_resource_preview(&app.root.resources).expect("a row is selected");

    assert!(
        preview.lines.contains(&(
            "Action".to_string(),
            "Unspecified (the rule carried neither allowed nor denied)".to_string()
        )),
        "the pane says why the column is dashing: {:?}",
        preview.lines
    );
}

// --- Empty states ---

#[test]
fn a_project_with_no_firewall_rules_says_so() {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::FirewallRulesLoaded {
            context,
            rules: Vec::new(),
        }),
    );

    let painted = painted(app);

    assert!(
        painted.contains("No firewall rules found for"),
        "an empty project names itself:\n{painted}"
    );
}

// --- The palette guard ---

/// The `Go to` entry is offered, and the log entry never is.
///
/// The refusal is the interesting half. A firewall rule's hits *are* logged —
/// against the subnetwork the traffic crossed, with the rule named in the
/// payload — so a rule-scoped query is the one an operator would most expect
/// to work, and it would return zero rows. On a DENY rule zero rows reads as
/// "nothing was blocked", which is the most dangerous empty result in this
/// epic.
#[test]
fn the_palette_offers_the_table_but_never_a_rule_scoped_log_jump() {
    let mut app = app_showing_rules();
    reduce(&mut app, &Action::Command(CommandAction::Open));
    let visible = app.root.command.visible_commands(&app.root);

    assert!(
        visible.contains(&CommandDescriptor::GoToService(
            ResourceServiceKind::FirewallRules
        )),
        "precondition: the service itself is reachable from the palette"
    );
    assert!(
        !visible.contains(&CommandDescriptor::OpenLogsForSelected(
            ResourceServiceKind::FirewallRules
        )),
        "a rule-scoped log query can only return zero rows, and zero rows on a \
         DENY rule reads as `nothing was blocked`"
    );
}

// --- Context guarding ---

/// A listing that resolved for a configuration the operator has since left is
/// dropped, on both the success and the failure path.
///
/// Both halves are asserted together because the failure half is the one that
/// regresses silently: an inverted success guard stops rows appearing and is
/// loud, while an inverted failure guard leaves a correct-looking table that
/// empties the next time some unrelated project's load is refused.
#[test]
fn a_result_for_a_context_the_operator_has_left_is_dropped_on_both_paths() {
    let other = GcloudContext {
        configuration: GcloudConfigurationName::new("prod"),
        account: Some("dev@example.com".to_string()),
        project: Some("prod-project".to_string()),
        region: None,
        zone: None,
    };

    let mut app = app_showing_rules();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::FirewallRulesLoaded {
            context: other.clone(),
            rules: vec![FirewallRule {
                name: "prod-only".to_string(),
                network: "prod-vpc".to_string(),
                direction: FirewallDirection::Ingress,
                priority: Some(100),
                action: FirewallAction::Deny,
                targets: FirewallTargets::AllInstances,
                source_or_dest: vec!["10.0.0.0/8".to_string()],
                disabled: false,
            }],
        }),
    );
    let names: Vec<&str> = app
        .root
        .resources
        .firewall_rules()
        .iter()
        .map(|rule| rule.name.as_str())
        .collect();
    assert!(
        !names.contains(&"prod-only"),
        "another configuration's rules must not land here: {names:?}"
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::FirewallRulesLoadFailed {
            context: other,
            error: AppError::gcloud("PERMISSION_DENIED on prod-project".to_string()),
        }),
    );
    assert_eq!(
        app.root.resources.firewall_rules().len(),
        4,
        "another configuration's refusal must not clear this table"
    );
    assert!(
        app.root.resources.firewall_rules_problem().is_none(),
        "nor become this table's problem"
    );
}

// --- Filtering does not re-fetch ---

/// Applying a `/` filter emits **no effect** for this table, and the rows stay
/// where they are.
///
/// The networks and subnets finding, one service later, and the sharpest
/// instance of it. `load_firewall_rules` ignores the filter it is handed
/// because ACTION is derived from which of `allowed`/`denied` arrived, TARGETS
/// reads `all instances` where the rule scopes nothing, and the DISABLED badge
/// is a word this application adds — a pushed clause would match none of them.
///
/// The consequence is what makes it worth a fix rather than a cleanup. A
/// failed refresh runs `mark_active_stale_and_clear`, which drops the rows and
/// the context. So an operator who types `/DENY` to ask what this project
/// blocks would, on any transient failure of a call that was never needed,
/// have that answer replaced by a blank table — at the moment they asked the
/// question.
///
/// **Paired with a service that does re-fetch**, because a reducer mutated to
/// return `Vec::new()` unconditionally satisfies the negative half alone and
/// would silently stop every table narrowing server-side.
#[test]
fn applying_a_filter_refetches_the_pushdown_services_and_not_this_one() {
    let mut rules = app_showing_rules();
    reduce(&mut rules, &Action::Resources(ResourcesAction::StartFilter));
    for character in "DENY".chars() {
        reduce(
            &mut rules,
            &Action::Resources(ResourcesAction::AppendFilterChar(character)),
        );
    }
    let before = rules.root.resources.firewall_rules().len();
    let effects = reduce(&mut rules, &Action::Resources(ResourcesAction::ApplyFilter));

    assert!(
        effects.is_empty(),
        "a client-side narrowing must not spawn a gcloud process, and must not \
         put an operator's view of what this project blocks at the mercy of \
         that call failing: {effects:?}"
    );
    assert!(before > 0, "precondition: there were rows to keep");
    assert_eq!(
        rules.root.resources.firewall_rules().len(),
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
    let mut app = app_showing_rules();

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));

    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::RefreshFirewallRules { .. })),
        "`r` is an explicit ask for fresh rows and must reach gcloud: {effects:?}"
    );
}
