//! Discoverability of the two keys that lead to every other key.
//!
//! The footer is a curated per-view list and has to stay one: `ResourceList`
//! alone reaches 35 catalog bindings, and `render_footer` fills a single line
//! left-to-right. What it must never do is drop the *pointer* to the complete
//! list. Before this pass, `? Help` appeared in exactly one footer in the
//! application — Home — and `:` (the command palette, which reaches every
//! command by name) appeared in none, so leaving Home lost both the overlay
//! that teaches the keys and the modal that runs the commands.
//!
//! These two keys are therefore *pinned*: projected separately from the view's
//! own hints, and rendered right-aligned with their width reserved before the
//! view hints are laid out, so the narrow terminal that truncates the view
//! hints is exactly the terminal that keeps the way out. A modal or text-entry
//! sub-view pins nothing, because in those modes neither key dispatches — a
//! pinned hint is a promise about a key, and `/` filter mode would type a `?`
//! rather than open help.

use nuvem::app::projection::binding::command_key;
use nuvem::app::projection::{FrameHint, project_app_frame, project_app_frame_with};
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::command::CommandDescriptor;
use nuvem::app::{
    Action, AppState, CommandAction, ConfigurationsAction, EffectResult, JumpsAction,
    LoggingAction, NavigatorAction, ResourcesAction, RootState, SystemAction, Workspace,
    WorkspaceAction,
};
use nuvem::config::hotkeys::KeyOverrides;
use nuvem::ui::shared::frame::render_footer;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::k9s::resolve_launch_target;
use nuvem_domain::gcp::resource::{
    ComputeInstance, EndpointVisibility, GkeCluster, GkeClusterStatus, ResourceServiceKind,
    ResourceStatus,
};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use std::collections::BTreeMap;

/// The two pinned keys, in the order they are rendered left-to-right inside
/// the right-aligned region.
fn pinned() -> Vec<FrameHint> {
    vec![
        FrameHint {
            key: ":".to_string(),
            label: "Command palette",
        },
        FrameHint {
            key: "?".to_string(),
            label: "Help",
        },
    ]
}

fn root_in(workspace: Workspace) -> RootState {
    RootState {
        workspace,
        ..Default::default()
    }
}

/// An app holding one loaded compute instance in the Resources workspace.
fn app_with_one_instance() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            }],
        }),
    );
    let context = app
        .root
        .current_context()
        .expect("an active context")
        .clone();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![ComputeInstance {
                id: "api-dev-1-id".to_string(),
                name: "api-dev-1".to_string(),
                zone: "europe-west1-b".to_string(),
                status: ResourceStatus::Running,
                machine_type: "e2-medium".to_string(),
                internal_ip: Some("10.0.0.4".to_string()),
                external_ip: None,
            }],
        }),
    );
    app
}

/// One active configuration and nothing else — the Configurations workspace
/// with a context to edit.
fn app_with_configuration() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration()],
        }),
    );
    reduce(
        &mut app,
        &Action::Workspace(WorkspaceAction::Configurations),
    );
    app
}

fn configuration() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

fn gke_cluster() -> GkeCluster {
    GkeCluster {
        name: "platform-dev".to_string(),
        location: "europe-west1".to_string(),
        status: GkeClusterStatus::Running,
        current_master_version: Some("1.31.5-gke.1023000".to_string()),
        current_node_count: Some(3),
        endpoint_visibility: EndpointVisibility::Public,
    }
}

/// The Resources workspace with the GKE service active and one cluster
/// selected.
fn app_with_selected_gke() -> AppState {
    let mut app = app_with_configuration();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );
    let context = app
        .root
        .current_context()
        .expect("an active context")
        .clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context,
            clusters: vec![gke_cluster()],
        }),
    );
    app
}

/// Loads a plugin file written to a temporary path, so a test can hold one
/// without a fixture file in the repository.
fn plugins_from(body: &str) -> nuvem::config::plugins::Plugins {
    use std::io::Write as _;

    let mut file = tempfile::NamedTempFile::new().expect("a temp plugin file");
    file.write_all(body.as_bytes()).expect("the fixture writes");

    let mut problems = nuvem::config::problems::ConfigProblems::default();
    let plugins = nuvem::config::plugins::Plugins::load(file.path(), &mut problems);
    assert!(
        problems.is_empty(),
        "the fixture must be a valid plugin file: {:?}",
        problems.messages()
    );
    plugins
}

/// The palette route to a plugin: `:`, its rendered label, Enter. A plugin's
/// `CommandDescriptor::label` is a placeholder, so it is filtered by the label
/// its file declares rather than by the descriptor's.
fn submit_the_plugin_named(app: &mut AppState, label: &str) {
    reduce(app, &Action::Command(CommandAction::Open));
    for character in label.chars() {
        reduce(app, &Action::Command(CommandAction::AppendChar(character)));
    }
    reduce(app, &Action::Command(CommandAction::Submit));
}

/// Opens the palette, filters it to `descriptor`'s own label and submits —
/// the operator's route, so the state under test is the one the app reaches.
fn run_from_the_palette(app: &mut AppState, descriptor: CommandDescriptor) {
    reduce(app, &Action::Command(CommandAction::Open));
    for character in descriptor.label().chars() {
        reduce(app, &Action::Command(CommandAction::AppendChar(character)));
    }
    assert_eq!(
        app.root.command.selected_visible_command(&app.root),
        Some(descriptor),
        "the palette must offer {descriptor:?}, or the fixture proves nothing"
    );
    reduce(app, &Action::Command(CommandAction::Submit));
}

#[test]
fn every_normal_workspace_pins_the_palette_and_help_keys() {
    for workspace in [
        Workspace::Home,
        Workspace::Configurations,
        Workspace::Resources,
        Workspace::Logging,
        Workspace::Tunnels,
        Workspace::FinOps,
    ] {
        let projection = project_app_frame(&root_in(workspace));
        assert_eq!(
            projection.footer_pinned_hints,
            pinned(),
            "{workspace:?} should pin the palette and help keys"
        );
    }
}

/// The detail pane is its own footer branch, and it is the view an operator is
/// most likely to be stuck in.
#[test]
fn a_resource_detail_view_pins_the_palette_and_help_keys() {
    let mut app = app_with_one_instance();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(
        app.root.resources.is_showing_detail(),
        "the fixture must reach the detail view, or this asserts nothing"
    );

    assert_eq!(project_app_frame(&app.root).footer_pinned_hints, pinned());
}

/// A pinned hint is a promise about a key. `?` in a filter is a character, so
/// the filter footer promises nothing.
#[test]
fn a_text_entry_sub_view_pins_nothing() {
    let mut app = app_with_one_instance();
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));

    assert!(
        project_app_frame(&app.root).footer_pinned_hints.is_empty(),
        "a filter footer must not advertise keys the filter swallows"
    );

    let mut configurations = AppState::new();
    reduce(
        &mut configurations,
        &Action::Workspace(WorkspaceAction::Configurations),
    );
    reduce(
        &mut configurations,
        &Action::Configurations(ConfigurationsAction::StartFiltering),
    );

    assert!(
        project_app_frame(&configurations.root)
            .footer_pinned_hints
            .is_empty()
    );

    let mut creating = AppState::new();
    reduce(
        &mut creating,
        &Action::Workspace(WorkspaceAction::Configurations),
    );
    reduce(
        &mut creating,
        &Action::Configurations(ConfigurationsAction::StartCreate),
    );
    assert!(
        project_app_frame(&creating.root)
            .footer_pinned_hints
            .is_empty()
    );

    let mut logging = AppState::new();
    reduce(&mut logging, &Action::Workspace(WorkspaceAction::Logging));
    reduce(&mut logging, &Action::Logging(LoggingAction::StartFilter));
    assert!(
        project_app_frame(&logging.root)
            .footer_pinned_hints
            .is_empty(),
        "the logging filter reads `?` as a character too"
    );

    let mut navigating = app_with_one_instance();
    reduce(
        &mut navigating,
        &Action::Navigator(NavigatorAction::StartFilter),
    );
    assert!(
        project_app_frame(&navigating.root)
            .footer_pinned_hints
            .is_empty(),
        "so does the service navigator's filter"
    );
}

/// The help overlay is reached *by* the pinned key; re-advertising it there
/// would be advertising a key whose meaning has changed (`?` closes).
#[test]
fn the_open_help_overlay_pins_nothing() {
    let mut app = AppState::new();
    reduce(&mut app, &Action::System(SystemAction::ToggleHelp));

    assert!(project_app_frame(&app.root).footer_pinned_hints.is_empty());
}

/// Every overlay that owns the keyboard, one test per overlay.
///
/// One per overlay rather than one for "an overlay": `a_mode_owns_the_keyboard`
/// is a disjunction, and a disjunction is only as tested as its least-tested
/// term — a single case leaves every other term free to be deleted. These are
/// the overlays that render *without a footer of their own*, so they reach the
/// normal view's footer and are exactly the ones a pin could wrongly ride on.
#[test]
fn a_tunnel_launch_preview_pins_nothing() {
    let mut app = app_with_one_instance();
    reduce(&mut app, &Action::Resources(ResourcesAction::RequestTunnel));
    assert!(
        app.root.resources.tunnel_launch_preview().is_some(),
        "the fixture must open the preview, or this asserts nothing"
    );

    assert!(project_app_frame(&app.root).footer_pinned_hints.is_empty());
}

#[test]
fn an_ssh_launch_preview_pins_nothing() {
    let mut app = app_with_one_instance();
    run_from_the_palette(&mut app, CommandDescriptor::SshSelectedComputeInstance);
    assert!(
        app.root.resources.ssh_launch_preview().is_some(),
        "the fixture must open the preview, or this asserts nothing"
    );

    assert!(project_app_frame(&app.root).footer_pinned_hints.is_empty());
}

#[test]
fn the_related_jump_picker_pins_nothing() {
    let mut app = app_with_one_instance();
    reduce(&mut app, &Action::Jumps(JumpsAction::Open));
    assert!(
        app.root.jump_picker.is_some(),
        "the fixture must open the picker, or this asserts nothing"
    );

    assert!(project_app_frame(&app.root).footer_pinned_hints.is_empty());
}

#[test]
fn a_k9s_launch_confirmation_pins_nothing() {
    let mut app = app_with_selected_gke();
    let context = app
        .root
        .current_context()
        .expect("an active context")
        .clone();
    let target = resolve_launch_target(&context, &gke_cluster()).expect("target resolves");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::K9sLaunchPrepared {
            existing_contexts: Some(vec![target.kube_context.clone()]),
            target,
        }),
    );
    assert!(
        app.root.resources.k9s_launch_confirmation().is_some(),
        "the fixture must open the confirmation, or this asserts nothing"
    );

    assert!(project_app_frame(&app.root).footer_pinned_hints.is_empty());
}

#[test]
fn the_plugin_output_viewer_pins_nothing() {
    let mut app = app_with_one_instance();
    let id = app
        .root
        .system
        .operations
        .begin("plugin", "Describe web-01");
    let context = app
        .root
        .current_context()
        .expect("an active context")
        .clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PluginRunSucceeded {
            context,
            label: "Describe web-01".to_string(),
            output: "name: web-01\n".to_string(),
            stderr: String::new(),
            operation_id: id,
        }),
    );
    run_from_the_palette(&mut app, CommandDescriptor::ShowPluginOutput);
    assert!(
        app.root.system.plugin_output().is_some(),
        "the fixture must open the viewer, or this asserts nothing"
    );

    assert!(project_app_frame(&app.root).footer_pinned_hints.is_empty());
}

#[test]
fn a_plugin_confirmation_pins_nothing() {
    let mut app = app_with_one_instance();
    app.root.plugins = plugins_from(
        "version = 1\n\n[[plugins]]\nid = \"describe\"\n\
         scope = \"GCE instances\"\nkey = \"d\"\n\
         label = \"Describe instance\"\n\
         args = [\"compute\", \"instances\", \"describe\", \"{name}\"]\n",
    );
    submit_the_plugin_named(&mut app, "Describe instance");
    assert!(
        app.root.resources.plugin_confirmation().is_some(),
        "the fixture must open the confirmation, or this asserts nothing"
    );

    assert!(project_app_frame(&app.root).footer_pinned_hints.is_empty());
}

#[test]
fn a_plugin_input_prompt_pins_nothing() {
    let mut app = app_with_one_instance();
    app.root.plugins = plugins_from(
        "version = 1\n\n[[plugins]]\nid = \"tail\"\n\
         scope = \"GCE instances\"\nkey = \"t\"\n\
         label = \"Tail instance\"\n\
         args = [\"compute\", \"instances\", \"tail\", \"{name}\", \"--lines={lines}\"]\n\n\
         [[plugins.inputs]]\nname = \"lines\"\ntype = \"string\"\n",
    );
    submit_the_plugin_named(&mut app, "Tail instance");
    assert!(
        app.root.resources.plugin_prompt().is_some(),
        "the fixture must open the prompt, or this asserts nothing"
    );

    assert!(project_app_frame(&app.root).footer_pinned_hints.is_empty());
}

/// The two configuration modals, which are text entry rather than overlays but
/// reach the same footer.
#[test]
fn the_configuration_context_modals_pin_nothing() {
    let mut editing = app_with_configuration();
    reduce(
        &mut editing,
        &Action::Configurations(ConfigurationsAction::StartEditActiveContext),
    );
    assert!(
        editing.root.configurations.context_edit_form().is_some(),
        "the fixture must open the edit form, or this asserts nothing"
    );
    assert!(
        project_app_frame(&editing.root)
            .footer_pinned_hints
            .is_empty()
    );

    reduce(
        &mut editing,
        &Action::Configurations(ConfigurationsAction::OpenContextPicker),
    );
    assert!(
        editing.root.configurations.context_picker().is_some(),
        "the fixture must open the picker, or this asserts nothing"
    );
    assert!(
        project_app_frame(&editing.root)
            .footer_pinned_hints
            .is_empty()
    );
}

/// The pin survives a rebind: it names whichever key opens help now.
#[test]
fn a_rebound_help_key_is_the_key_that_gets_pinned() {
    let raw: BTreeMap<String, String> = [("help".to_string(), "H".to_string())]
        .into_iter()
        .collect();
    let overrides = KeyOverrides::from_file(&raw).expect("H should be a valid help override");

    let projection = project_app_frame_with(&RootState::default(), &overrides);

    assert!(
        projection
            .footer_pinned_hints
            .iter()
            .any(|hint| hint.label == "Help" && hint.key == "H"),
        "the pinned help hint should follow the override, got {:?}",
        projection.footer_pinned_hints
    );
}

/// The pins cost the view nothing while everything fits, take no more than a
/// third of a row that has already overflowed, and give up their labels
/// before they give up their keys.
#[test]
fn the_pins_take_a_bounded_share_of_a_footer_that_has_overflowed() {
    let projection = project_app_frame(&root_in(Workspace::Resources));

    let footer_at = |width: u16| -> String {
        let mut terminal =
            Terminal::new(TestBackend::new(width, 1)).expect("terminal should initialize");
        terminal
            .draw(|frame| render_footer(frame, Rect::new(0, 0, width, 1), &projection))
            .expect("footer should render");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };

    // Room for both groups: the pins cost the view nothing at all.
    let roomy = footer_at(200);
    assert!(
        roomy.trim_end().ends_with(": Command palette  ? Help"),
        "a roomy footer carries both pins with their labels, got {roomy:?}"
    );
    assert!(
        roomy.contains("Enter Open / enter"),
        "and still every view hint, got {roomy:?}"
    );

    // 80 columns: the view's hints overflow on their own, and a third of the
    // row is not enough for both pins — the palette is the one that goes.
    let full = footer_at(80);
    assert!(
        full.trim_end().ends_with("? Help"),
        "a full footer keeps the help pin whole, got {full:?}"
    );
    assert!(
        !full.contains("Command palette"),
        "and drops the palette pin first, got {full:?}"
    );

    // Narrower than any hint: the help pin keeps its key and gives up its
    // label rather than the row losing the way out.
    let narrow = footer_at(20);
    assert!(
        narrow.trim_end().ends_with('?'),
        "the way out survives as a bare key, got {narrow:?}"
    );
    assert!(
        !narrow.contains("? Help"),
        "the label is what it gives up, got {narrow:?}"
    );
}

/// The pins take the slack, not a fixed share of the row. Home's own hints
/// leave 29 columns spare at 80 — enough for both pins — where a third of the
/// row would only have covered help, so the two rules disagree here and the
/// slack is the one that holds.
#[test]
fn a_footer_with_room_to_spare_keeps_both_pins_whole() {
    let projection = project_app_frame(&root_in(Workspace::Home));
    let width: u16 = 80;

    let mut terminal =
        Terminal::new(TestBackend::new(width, 1)).expect("terminal should initialize");
    terminal
        .draw(|frame| render_footer(frame, Rect::new(0, 0, width, 1), &projection))
        .expect("footer should render");
    let line = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(
        line.trim_end().ends_with(": Command palette  ? Help"),
        "both pins should survive a row with slack to spare, got {line:?}"
    );
    assert!(
        line.contains("c Configurations") && line.contains("q Quit"),
        "and cost Home none of its own hints, got {line:?}"
    );
}

/// A footer with nothing pinned is the footer it always was: the whole row
/// belongs to the view. Rendered at exactly the width its own hints need, so a
/// reserve of any size at all would cost it the last one.
#[test]
fn a_footer_with_nothing_pinned_gives_the_whole_row_to_the_view() {
    let mut app = app_with_one_instance();
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    let projection = project_app_frame(&app.root);
    assert!(
        projection.footer_pinned_hints.is_empty(),
        "the fixture must pin nothing, or this asserts nothing"
    );

    let width: u16 = 46;
    let mut terminal =
        Terminal::new(TestBackend::new(width, 1)).expect("terminal should initialize");
    terminal
        .draw(|frame| render_footer(frame, Rect::new(0, 0, width, 1), &projection))
        .expect("footer should render");
    let line = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert_eq!(
        line, "Enter Apply  Backspace Delete char  Esc Cancel",
        "every filter hint should fit the row it exactly fills"
    );
}

/// A palette row teaches the key that runs the same command, so the lookup is
/// paid once.
#[test]
fn a_palette_row_names_the_key_that_runs_it_in_the_underlying_view() {
    let root = root_in(Workspace::Home);

    assert_eq!(
        command_key(
            CommandDescriptor::RefreshCurrentView,
            &root,
            &KeyOverrides::empty()
        ),
        Some("r".to_string())
    );
    assert_eq!(
        command_key(
            CommandDescriptor::GoToTunnels,
            &root,
            &KeyOverrides::empty()
        ),
        Some("Ctrl-t".to_string())
    );
}

/// ...and only where it runs it. `L` opens the selection's logs from a
/// resource list and dispatches nowhere on Home, so Home's palette must not
/// name it.
#[test]
fn a_palette_row_names_no_key_where_that_key_does_not_dispatch() {
    let home = root_in(Workspace::Home);
    let resources = root_in(Workspace::Resources);
    let command = CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::ComputeInstances);

    assert_eq!(command_key(command, &home, &KeyOverrides::empty()), None);
    assert_eq!(
        command_key(command, &resources, &KeyOverrides::empty()),
        Some("L".to_string())
    );
}

/// Most commands are palette-only. Naming a key for them would be inventing
/// one.
#[test]
fn a_palette_only_command_names_no_key() {
    assert_eq!(
        command_key(
            CommandDescriptor::StopSelectedComputeInstance,
            &root_in(Workspace::Resources),
            &KeyOverrides::empty(),
        ),
        None
    );
}

/// No footer hint spends its columns on punctuation alone.
///
/// `hint_spans` used to give the label whatever was left after the key, and
/// `fit_text` answers a width of 1, 2 or 3 with that many literal periods. At 60
/// columns the Resources footer read `r ..`, which is not a shorter way of
/// writing `r Refresh` — it reads as corruption, and it costs the next hint the
/// columns it wasted.
///
/// The key alone is a complete instruction, so the label yields entirely rather
/// than degenerating into dots.
#[test]
fn a_footer_hint_never_renders_a_label_made_only_of_dots() {
    let projection = project_app_frame(&root_in(Workspace::Resources));

    // Every width from cramped to roomy, not one hand-picked one: which hint
    // lands on the boundary moves with the width, so a single width tests a
    // single hint and the defect is about the rule, not about `Refresh`.
    for width in 20u16..=140 {
        let mut terminal =
            Terminal::new(TestBackend::new(width, 1)).expect("terminal should initialize");
        terminal
            .draw(|frame| render_footer(frame, Rect::new(0, 0, width, 1), &projection))
            .expect("footer should render");
        let line = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        // Split per hint, not per whitespace token: hints are joined with two
        // spaces, and a label truncated at a word boundary legitimately renders
        // as `Focus ...`, whose trailing `...` is its own token. What must never
        // appear is a hint whose *whole* label is punctuation.
        for hint in line.split("  ") {
            let hint = hint.trim();
            let Some((key, label)) = hint.split_once(' ') else {
                continue;
            };
            assert!(
                !label.chars().all(|character| character == '.'),
                "at {width} columns the {key:?} hint's label is nothing but \
                 dots: {line:?}"
            );
            // The sharper half of the same defect: a key truncated to `.` names
            // no key at all, so the hint is unusable rather than merely terse.
            assert!(
                !key.chars().all(|character| character == '.'),
                "at {width} columns a hint's key is nothing but dots, so it \
                 names no key to press: {line:?}"
            );
        }
    }
}
