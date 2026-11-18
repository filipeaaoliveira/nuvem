//! Evidence: a plugin's declared `key` reaches its confirmation.
//!
//! Before this slice `plugin.key` was validated (uniqueness within a scope,
//! `src/config/plugins.rs`) and dispatched nowhere, so a user wrote `key = "d"`
//! and nothing was bound.

use std::io::Write;

use nuvem::app::projection::project_app_frame;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::root::Workspace;
use nuvem::app::state::system::OperationPhase;
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction, WorkspaceAction};
use nuvem::config::plugins::Plugins;
use nuvem::config::problems::ConfigProblems;
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::resource::{
    BigQueryDataset, CloudFunction, CloudFunctionEnvironment, CloudFunctionState, CloudRunService,
    CloudRunServiceStatus, CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, ComputeInstance,
    DiskStatus, EndpointVisibility, FirewallAction, FirewallDirection, FirewallRule,
    FirewallTargets, GkeCluster, GkeClusterStatus, IamPolicyMember, PersistentDisk, PubSubDelivery,
    PubSubSubscription, PubSubSubscriptionState, PubSubTopic, ResourceServiceKind, ResourceStatus,
    Secret, ServiceAccount, StorageBucket, SubscriptionCount, VpcNetwork, VpcRoutingMode,
    VpcSubnet, VpcSubnetMode,
};
use tempfile::NamedTempFile;

/// The plugin every test here declares, unless it is about a plugin that
/// cannot resolve: name-only, so any service's row can fill it.
const DESCRIBE_ARGS: &[&str] = &["compute", "instances", "describe", "{name}"];

fn configuration() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("sre@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

/// The config layer's own spelling of a scope.
///
/// A second copy of `config::config_service_id`, which is `pub(crate)` and so
/// out of reach from an integration test. It cannot drift silently: a spelling
/// this map gets wrong either fails to parse — [`plugin_scoped_to`] asserts the
/// file loads clean — or parses as a *different* service, and the scope
/// assertion below catches that.
fn scope_id(kind: ResourceServiceKind) -> &'static str {
    match kind {
        ResourceServiceKind::ComputeInstances => "compute",
        ResourceServiceKind::GkeClusters => "gke",
        ResourceServiceKind::CloudSql => "cloudsql",
        ResourceServiceKind::CloudRunServices => "cloudrun",
        ResourceServiceKind::CloudStorage => "storage",
        ResourceServiceKind::SecretManagerSecrets => "secrets",
        ResourceServiceKind::CloudFunctions => "functions",
        ResourceServiceKind::PersistentDisks => "disks",
        ResourceServiceKind::PubSubTopics => "pubsub-topics",
        ResourceServiceKind::PubSubSubscriptions => "pubsub-subscriptions",
        ResourceServiceKind::BigQueryDatasets => "bigquery-datasets",
        ResourceServiceKind::IamServiceAccounts => "service-accounts",
        ResourceServiceKind::IamPolicy => "iam-policy",
        ResourceServiceKind::VpcNetworks => "vpc-networks",
        ResourceServiceKind::VpcSubnets => "vpc-subnets",
        ResourceServiceKind::FirewallRules => "firewall-rules",
    }
}

/// A plugin file, loaded the only way a plugin is ever loaded: through the real
/// config parser over a real file. A fixture that skipped validation would be
/// pressing a key bound to a `Plugin` the config layer would never hand over —
/// and the key is the very field this slice is about.
pub(crate) fn load(body: &str) -> Plugins {
    let mut file = NamedTempFile::new().expect("a temp plugin file");
    file.write_all(body.as_bytes()).expect("the fixture writes");

    let mut problems = ConfigProblems::default();
    let plugins = Plugins::load(file.path(), &mut problems);
    assert!(
        problems.is_empty(),
        "the fixture must be a valid plugin file: {:?}",
        problems.messages()
    );
    plugins
}

/// One plugin scoped to `kind`, bound to `key`.
fn plugin_scoped_to(kind: ResourceServiceKind, key: char, args: &[&str]) -> Plugins {
    let body = format!(
        "version = 1\n\n[[plugins]]\nid = \"describe\"\nscope = {:?}\nkey = {key:?}\n\
         label = \"Describe {{name}}\"\nargs = {args:?}\n",
        scope_id(kind)
    );
    let plugins = load(&body);
    assert_eq!(plugins.len(), 1, "the fixture plugin must load");
    assert_eq!(
        plugins.all()[0].scope,
        kind,
        "the fixture's scope spelling must name the service the test is about"
    );
    plugins
}

/// The state a keyed press is made in: the Resources workspace showing
/// `kind`, one row named `row_name` under the cursor, a resolved `dev`
/// context, and one plugin scoped to that service bound to `key`.
///
/// Exhaustive over `ResourceServiceKind` and parameterised by the row's name,
/// so the walk over every service can assert on the argv the row composes
/// rather than only on the fact that something opened.
pub(crate) fn root_with_plugin(
    kind: ResourceServiceKind,
    key: char,
    args: &[&str],
    row_name: &str,
) -> AppState {
    let mut app = AppState::new();
    app.root.plugins = plugin_scoped_to(kind, key, args);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration()],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(kind)),
    );
    let context = app
        .root
        .current_context()
        .expect("the dev configuration is the active context");
    reduce(
        &mut app,
        &Action::EffectResult(row_loaded(kind, context, row_name)),
    );

    assert_eq!(app.root.workspace, Workspace::Resources);
    assert_eq!(app.root.resources.active_service(), kind);
    app
}

/// One row of `kind`, whose identity — the value `{name}` renders to, which
/// for IAM is the member and the email rather than a `name` field — is
/// `row_name`.
fn row_loaded(kind: ResourceServiceKind, context: GcloudContext, row_name: &str) -> EffectResult {
    match kind {
        ResourceServiceKind::ComputeInstances => EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![compute_row(row_name)],
        },
        ResourceServiceKind::GkeClusters => EffectResult::GkeClustersLoaded {
            context,
            clusters: vec![GkeCluster {
                name: row_name.to_string(),
                location: "europe-west1".to_string(),
                status: GkeClusterStatus::Running,
                current_master_version: Some("1.30".to_string()),
                current_node_count: Some(3),
                endpoint_visibility: EndpointVisibility::Public,
            }],
        },
        ResourceServiceKind::CloudSql => EffectResult::CloudSqlInstancesLoaded {
            context,
            instances: vec![CloudSqlInstance {
                name: row_name.to_string(),
                database_version: "POSTGRES_15".to_string(),
                engine: "PostgreSQL".to_string(),
                region: "europe-west1".to_string(),
                tier: "db-custom-2-7680".to_string(),
                availability: CloudSqlAvailability::Zonal,
                status: CloudSqlStatus::Runnable,
                connectivity: EndpointVisibility::Public,
            }],
        },
        ResourceServiceKind::CloudRunServices => EffectResult::CloudRunServicesLoaded {
            context,
            services: vec![CloudRunService {
                name: row_name.to_string(),
                region: "europe-west1".to_string(),
                url: None,
                latest_ready_revision: None,
                latest_created_revision: None,
                traffic_summary: "-".to_string(),
                status: CloudRunServiceStatus::Ready,
                ready_message: None,
            }],
        },
        ResourceServiceKind::CloudStorage => EffectResult::StorageBucketsLoaded {
            context,
            buckets: vec![StorageBucket {
                name: row_name.to_string(),
                location: "EU".to_string(),
                location_type: Some("multi-region".to_string()),
                storage_class: "STANDARD".to_string(),
                versioning_enabled: false,
                uniform_bucket_level_access: true,
                public_access_prevention: Some("enforced".to_string()),
                time_created: None,
                updated: Some("2026-01-01T00:00:00Z".to_string()),
            }],
        },
        ResourceServiceKind::SecretManagerSecrets => EffectResult::SecretsLoaded {
            context,
            secrets: vec![Secret {
                name: row_name.to_string(),
                create_time: Some("2026-02-11T09:14:22Z".to_string()),
                labels: Vec::new(),
                replication: "Automatic".to_string(),
            }],
        },
        ResourceServiceKind::CloudFunctions => EffectResult::CloudFunctionsLoaded {
            context,
            functions: vec![CloudFunction {
                name: row_name.to_string(),
                region: "europe-west1".to_string(),
                state: CloudFunctionState::Active,
                environment: CloudFunctionEnvironment::Gen2,
                url: None,
                update_time: Some("2026-03-01T10:00:00Z".to_string()),
                labels: Vec::new(),
            }],
        },
        ResourceServiceKind::PersistentDisks => EffectResult::PersistentDisksLoaded {
            context,
            disks: vec![PersistentDisk {
                name: row_name.to_string(),
                location: "europe-west1-b".to_string(),
                status: DiskStatus::Ready,
                size_gb: 20,
                disk_type: "pd-ssd".to_string(),
                users: Vec::new(),
                labels: Vec::new(),
            }],
        },
        ResourceServiceKind::PubSubSubscriptions => EffectResult::PubSubSubscriptionsLoaded {
            context,
            subscriptions: vec![PubSubSubscription {
                name: row_name.to_string(),
                full_name: format!("projects/dev-project/subscriptions/{row_name}"),
                topic: "projects/dev-project/topics/orders".to_string(),
                delivery: PubSubDelivery::Pull,
                ack_deadline_seconds: Some(60),
                message_retention: Some("604800s".to_string()),
                dead_letter: None,
                state: PubSubSubscriptionState::Active,
                labels: Vec::new(),
            }],
        },
        ResourceServiceKind::PubSubTopics => EffectResult::PubSubTopicsLoaded {
            context,
            topics: vec![PubSubTopic {
                name: row_name.to_string(),
                full_name: format!("projects/dev-project/topics/{row_name}"),
                subscription_count: SubscriptionCount::Known(2),
                message_retention: Some("604800s".to_string()),
                kms_key: None,
                labels: Vec::new(),
            }],
        },
        ResourceServiceKind::VpcNetworks => EffectResult::VpcNetworksLoaded {
            context,
            networks: vec![VpcNetwork {
                name: row_name.to_string(),
                subnet_mode: VpcSubnetMode::Auto,
                routing_mode: VpcRoutingMode::Regional,
                subnet_count: 43,
                description: None,
            }],
        },
        ResourceServiceKind::VpcSubnets => EffectResult::VpcSubnetsLoaded {
            context,
            subnets: vec![VpcSubnet {
                name: row_name.to_string(),
                region: "us-central1".to_string(),
                network: "default".to_string(),
                ip_cidr_range: "10.128.0.0/20".to_string(),
                gateway_address: Some("10.128.0.1".to_string()),
                private_google_access: false,
            }],
        },
        // `disabled: false` is load-bearing rather than incidental: a disabled
        // rule's identity cell carries the `DISABLED` badge ahead of the name,
        // so a disabled row would not render to `row_name` and this walk would
        // be asserting against a string it did not ask for.
        ResourceServiceKind::FirewallRules => EffectResult::FirewallRulesLoaded {
            context,
            rules: vec![FirewallRule {
                name: row_name.to_string(),
                network: "default".to_string(),
                direction: FirewallDirection::Ingress,
                priority: Some(1000),
                action: FirewallAction::Allow,
                targets: FirewallTargets::AllInstances,
                source_or_dest: vec!["0.0.0.0/0".to_string()],
                disabled: false,
            }],
        },
        // The email is this table's identity, so it is what `{name}` renders.
        ResourceServiceKind::IamServiceAccounts => EffectResult::ServiceAccountsLoaded {
            context,
            accounts: vec![ServiceAccount {
                email: row_name.to_string(),
                display_name: Some("Deployer".to_string()),
                disabled: false,
                unique_id: "100000000000000000001".to_string(),
            }],
        },
        // And the member is this one's.
        ResourceServiceKind::IamPolicy => EffectResult::IamPolicyLoaded {
            context,
            members: vec![IamPolicyMember::from_wire("roles/viewer", row_name, None)],
        },
        ResourceServiceKind::BigQueryDatasets => EffectResult::BigQueryDatasetsLoaded {
            context,
            datasets: vec![BigQueryDataset {
                dataset_id: row_name.to_string(),
                project_id: "dev-project".to_string(),
                location: "europe-west1".to_string(),
                friendly_name: None,
                labels: Vec::new(),
            }],
        },
    }
}

/// The Compute row the tests below select. `external_ip` is deliberately
/// absent: it is the field a plugin can name to build the "bound but
/// unresolvable" case without a second fixture shape.
fn compute_row(name: &str) -> ComputeInstance {
    ComputeInstance {
        id: format!("{name}-id"),
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: Some("10.0.0.2".to_string()),
        external_ip: None,
    }
}

fn arm(app: &mut AppState) {
    reduce(app, &Action::Resources(ResourcesAction::ArmPluginPrefix));
}

fn press(app: &mut AppState, key: char) {
    reduce(
        app,
        &Action::Resources(ResourcesAction::PluginPrefixPress(key)),
    );
}

fn status(app: &AppState) -> Option<&str> {
    app.root.system.status_message.as_deref()
}

/// The whole point of the slice: the key opens the confirmation the palette
/// opens, carrying the argv composed from the row selected *now*.
#[test]
fn a_bound_key_opens_the_confirmation_for_that_plugin() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'a',
        DESCRIBE_ARGS,
        "web-01",
    );

    arm(&mut app);
    assert!(
        app.root.plugin_prefix_pending.is_some(),
        "a scope with a plugin arms, or the key that follows is read as an ordinary key"
    );
    press(&mut app, 'a');

    let confirmation = app
        .root
        .resources
        .plugin_confirmation()
        .expect("the bound key must open the confirmation the palette opens");
    // The argv rather than the label: the label is a caption, and a key wired
    // to the wrong plugin would still render a plausible one.
    assert_eq!(
        confirmation.argv(),
        [
            "--configuration",
            "dev",
            "compute",
            "instances",
            "describe",
            "web-01"
        ],
        "the confirmation must carry the argv composed from the selected row, \
         with the injected configuration first"
    );
}

/// The prefix does not survive the press it was armed for. A flag that
/// outlived its press would leave the next keystroke resolving as a plugin
/// key.
#[test]
fn a_press_clears_the_pending_prefix() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'a',
        DESCRIBE_ARGS,
        "web-01",
    );

    arm(&mut app);
    assert!(app.root.plugin_prefix_pending.is_some(), "the prefix armed");
    press(&mut app, 'a');

    assert!(
        app.root.plugin_prefix_pending.is_none(),
        "the press consumes the prefix, whatever it resolved to"
    );
}

/// `,` in a scope with no plugin says so and does not arm, so the operator's
/// next keypress does its normal job instead of being eaten.
#[test]
fn arming_in_a_scope_with_no_plugin_refuses_and_does_not_arm() {
    // A plugin *does* exist — scoped to Cloud SQL — and the operator is looking
    // at the Compute list. An `any_scoped` that only asked whether the file was
    // empty would arm here and then have nothing to answer with.
    let mut app = root_with_plugin(ResourceServiceKind::CloudSql, 'a', DESCRIBE_ARGS, "db-01");
    app.root
        .resources
        .select_service(ResourceServiceKind::ComputeInstances);

    arm(&mut app);

    assert!(
        app.root.plugin_prefix_pending.is_none(),
        "a scope with no plugin must not eat the next keypress"
    );
    assert_eq!(
        status(&app),
        Some("No plugins are declared for GCE instances")
    );
}

/// A bound key whose plugin cannot resolve for this row explains itself and
/// opens nothing.
#[test]
fn a_bound_key_that_cannot_resolve_sets_the_refusal_and_opens_no_confirmation() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'a',
        // The fixture row has no external IP, so this element names a field the
        // row cannot fill.
        &[
            "compute",
            "instances",
            "describe",
            "{name}",
            "--ip",
            "{external_ip}",
        ],
        "web-01",
    );

    arm(&mut app);
    press(&mut app, 'a');

    assert_eq!(
        status(&app),
        Some("Plugin \"describe\" needs external_ip, which this GCE instance has no value for"),
        "the press must reuse the refusal wording the plugin resolver composed"
    );
    assert!(
        app.root.resources.plugin_confirmation().is_none(),
        "a plugin that composed no argv must open nothing"
    );
    assert!(
        app.root.plugin_prefix_pending.is_none(),
        "a refused press consumes the prefix too"
    );
}

/// A plugin whose templates name a value the operator has not supplied opens
/// the prompt that collects it, and no confirmation.
///
/// The keyed door is the half of this claim the palette cannot make: the prompt's
/// own evidence drives both doors, but this file is where the press is proven
/// to reach the plugin at all, so the modal it now lands on is pinned here too.
/// A press that opened nothing — which is what this arm did while nothing
/// collected values — is exactly what a broken keybinding looks like from the
/// chair.
#[test]
fn a_plugin_still_waiting_for_values_opens_the_prompt_and_no_confirmation() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'a',
        DESCRIBE_ARGS,
        "web-01",
    );
    app.root.plugins = load(
        "version = 1\n\n\
         [[plugins]]\n\
         id = \"resize\"\n\
         scope = \"compute\"\n\
         key = \"a\"\n\
         label = \"Resize {name}\"\n\
         args = [\"compute\", \"instances\", \"resize\", \"{name}\", \"--size\", \"{disk_size}\"]\n\
         \n\
         [[plugins.inputs]]\n\
         name = \"disk_size\"\n\
         type = \"string\"\n",
    );

    arm(&mut app);
    press(&mut app, 'a');

    let prompt = app
        .root
        .resources
        .plugin_prompt()
        .expect("the press must open the prompt that collects the declared value");
    assert_eq!(
        prompt.label(),
        "Resize web-01",
        "the prompt is captioned with the label the operator was shown"
    );
    assert!(
        app.root.resources.plugin_confirmation().is_none(),
        "there is no argv to confirm until the values are collected"
    );
}

/// An undeclared key names itself and the selection noun — quoted, the way the
/// plugin resolver's `refusal` quotes the plugin id.
#[test]
fn an_unbound_key_says_so() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'a',
        DESCRIBE_ARGS,
        "web-01",
    );

    arm(&mut app);
    press(&mut app, 'z');

    assert_eq!(
        status(&app),
        Some("No plugin bound to 'z' for GCE instances")
    );
    assert!(
        app.root.resources.plugin_confirmation().is_none(),
        "an undeclared key must open nothing"
    );
    assert!(
        app.root.plugin_prefix_pending.is_none(),
        "an unbound press consumes the prefix too"
    );
}

/// The reason the character is quoted: `is_plain_text_key` admits `Char(' ')`,
/// so a stray space after `,` is an ordinary unbound press — and unquoted it
/// renders as "No plugin bound to   for GCE instances", a sentence with a hole
/// in it where the operator's mistake should be.
#[test]
fn an_unbound_whitespace_key_is_quoted_rather_than_rendered_raw() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'a',
        DESCRIBE_ARGS,
        "web-01",
    );

    arm(&mut app);
    press(&mut app, ' ');

    assert_eq!(
        status(&app),
        Some("No plugin bound to ' ' for GCE instances")
    );
}

/// A declared plugin with nothing under the cursor says so rather than eating
/// the keypress.
///
/// Reachable, not theoretical: `,` arms on whether the *file* declares a plugin
/// for this scope, and the keymap binds it in both resource views — the list
/// and a detail pane — with no selection predicate at all, so an empty list (a
/// project with no instances, or a first load still in flight) lands exactly
/// here. Silence would spend two keystrokes on nothing and leave the operator
/// with no way to tell the dead key from a plugin that ran.
///
/// The `plugin_prefix_pending` assertion is the second thing this pins: every
/// other case here has both a context and a row, so moving the arm's
/// unconditional clear below the resolution passes all of them — and leaks the
/// armed prefix on precisely this press, making the operator's *next* keystroke
/// resolve as a plugin key.
#[test]
fn a_press_with_nothing_selected_says_so_and_opens_nothing() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'a',
        DESCRIBE_ARGS,
        "web-01",
    );
    // The list reloaded empty under the operator, which is the state a project
    // with no instances is in from the moment it finishes loading.
    let context = app
        .root
        .current_context()
        .expect("the dev configuration is still active");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: Vec::new(),
        }),
    );

    arm(&mut app);
    assert!(
        app.root.plugin_prefix_pending.is_some(),
        "the scope still declares a plugin, so `,` still arms"
    );
    press(&mut app, 'a');

    assert_eq!(
        status(&app),
        Some("No GCE instance selected to run a plugin on")
    );
    assert!(
        app.root.resources.plugin_confirmation().is_none(),
        "there is no row to compose an argv from"
    );
    assert!(
        app.root.plugin_prefix_pending.is_none(),
        "the press consumes the prefix on this path too, or the next keystroke is \
         read as a plugin key"
    );
}

/// The other early return: a plugin pressed with no resolved gcloud context.
/// The argv carries an injected `--configuration` that comes from the context,
/// so there is nothing to compose — and, as with the row, nothing is what the
/// operator must not be told.
#[test]
fn a_press_with_no_resolved_context_says_so_and_opens_nothing() {
    let mut app = AppState::new();
    app.root.plugins = plugin_scoped_to(ResourceServiceKind::ComputeInstances, 'a', DESCRIBE_ARGS);
    app.root
        .resources
        .select_service(ResourceServiceKind::ComputeInstances);
    assert!(
        app.root.current_context().is_none(),
        "no configuration has been loaded, so there is no context to run under"
    );

    arm(&mut app);
    assert!(
        app.root.plugin_prefix_pending.is_some(),
        "arming asks about the file, which declares a plugin here"
    );
    press(&mut app, 'a');

    assert_eq!(
        status(&app),
        Some("Plugin not run: no active gcloud configuration")
    );
    assert!(
        app.root.resources.plugin_confirmation().is_none(),
        "nothing can be composed without the configuration the argv names"
    );
    assert!(
        app.root.plugin_prefix_pending.is_none(),
        "the press consumes the prefix on this path too"
    );
}

/// `--read-only` refuses the keyed press exactly as it refuses the palette's,
/// through the one helper both call. The palette's own guard
/// (`reducer::command`) is unreachable from here, which is why the helper
/// exists.
///
/// The argv is this file's one departure from [`DESCRIBE_ARGS`]: now
/// the mode admits a plugin whose declared command path nuvem can prove reads,
/// so a `describe` here would be refused by nothing and the test would pass
/// with the keyed door's guard deleted. The shared fixture's label still reads
/// "Describe", and the refusal quotes it: a label is the operator's own text
/// and has no bearing on what the declaration is classified as.
#[test]
fn read_only_refuses_a_keyed_press() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'a',
        &["compute", "instances", "delete", "{name}"],
        "web-01",
    );
    app.root.read_only = true;

    arm(&mut app);
    press(&mut app, 'a');

    assert!(
        app.root.resources.plugin_confirmation().is_none(),
        "read-only must leave nothing to confirm"
    );
    assert_eq!(
        status(&app),
        Some("Describe web-01 is unavailable: read-only mode cannot prove this only reads"),
        "the refusal names the label the operator was shown"
    );
}

/// Every service can carry a plugin, so every service's keyed press must reach
/// its confirmation. Walks [`ResourceServiceKind::ALL`] rather than naming
/// services: naming them is exactly how a new service ships with a validated
/// plugin whose key dispatches nothing.
///
/// The sibling of
/// `app::state::plugins::tests::every_service_resolves_a_name_only_plugin_against_its_own_row`,
/// one layer up: that one proves the resolver answers for every service, this
/// one proves the *keyed press* does.
#[test]
fn every_service_opens_a_confirmation_from_a_keyed_press() {
    for kind in ResourceServiceKind::ALL {
        let mut app = root_with_plugin(kind, 'a', &["describe", "{name}"], "row-01");

        arm(&mut app);
        assert!(
            app.root.plugin_prefix_pending.is_some(),
            "{kind:?}: `,` must arm where a plugin is scoped"
        );
        press(&mut app, 'a');

        let confirmation = app
            .root
            .resources
            .plugin_confirmation()
            .unwrap_or_else(|| panic!("{kind:?}: a keyed press opened no confirmation"));
        assert_eq!(
            confirmation.argv(),
            ["--configuration", "dev", "describe", "row-01"],
            "{kind:?}: the argv must carry this service's own row"
        );
    }
}

/// The keyed press reaches a plugin from an open detail pane, where the palette
/// deliberately offers none.
///
/// Not an oversight on either side. `plugin_commands`
/// (`app::state::command`) withholds every plugin while a detail pane is open,
/// because a palette entry is a row of a list and the pane is not that list;
/// the keymap binds `,` in the detail view anyway, because a key pressed
/// against the resource *on screen* is unambiguous about which row it means —
/// `selected_fields` resolves the detail target, not the list cursor.
///
/// The divergence is worth a test of its own because it is the one claim the
/// README's "open detail pane" paragraph could be read as denying, and because
/// a later slice tidying the two paths into one guard would silently take the
/// keyed press with it.
#[test]
fn a_keyed_press_reaches_a_plugin_from_an_open_detail_pane() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'a',
        DESCRIBE_ARGS,
        "web-01",
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(
        app.root.resources.is_showing_detail(),
        "the fixture must actually be in the state the test is about"
    );

    arm(&mut app);
    // The status area is the frame's, rendered for every workspace and both
    // resource views (`ui::render`, ahead of any overlay), which is why the
    // prompt lives in the frame projection rather than in the table tools line
    // the filter draft uses — that line is the list view's, and the gesture is
    // reachable from here. A later guard scoping the prompt to the list would
    // pass every other test in this file.
    assert_eq!(
        project_app_frame(&app.root).operation.message.as_deref(),
        Some("Plugin key: a (Esc to cancel)"),
        "the prompt has to reach the surface the gesture is reachable from"
    );
    press(&mut app, 'a');

    let confirmation = app
        .root
        .resources
        .plugin_confirmation()
        .expect("the key must reach the plugin the palette withholds here");
    assert_eq!(
        confirmation.argv(),
        [
            "--configuration",
            "dev",
            "compute",
            "instances",
            "describe",
            "web-01"
        ],
        "the argv names the resource the pane is showing"
    );
}

/// Two plugins in one scope, keyed in the order given.
///
/// The status-line hint's only interesting property is *which* order it lists
/// them in, so the caller passes keys that are not in alphabetical order: a hint
/// that sorted, or that read the keys off some map's iteration order, would be
/// indistinguishable from one that preserved the file's order if the fixture's
/// two orders agreed.
fn two_plugins_scoped_to(kind: ResourceServiceKind, keys: [char; 2]) -> Plugins {
    let scope = scope_id(kind);
    let body = keys
        .iter()
        .enumerate()
        .map(|(index, key)| {
            format!(
                "[[plugins]]\nid = \"describe-{index}\"\nscope = {scope:?}\nkey = {key:?}\n\
                 label = \"Describe {{name}} {index}\"\nargs = {DESCRIBE_ARGS:?}\n"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let plugins = load(&format!("version = 1\n\n{body}"));
    assert_eq!(plugins.len(), 2, "both fixture plugins must load");
    plugins
}

/// The armed prefix has to say which keys it will accept: the keys are runtime
/// data from the operator's own plugin file, so nothing static — not the footer,
/// not the help catalog — can name them, and a prefix that armed silently would
/// leave the operator guessing at the file they wrote weeks ago.
///
/// Both directions matter. That the hint appears while armed is the feature;
/// that it is *gone* otherwise is what keeps the status line free for the
/// refusals and operation messages every other test here reads, and a prompt
/// still on screen after the press would claim nuvem is waiting for a key it
/// has already consumed.
#[test]
fn the_status_line_prompts_with_the_declared_keys_only_while_armed() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'a',
        DESCRIBE_ARGS,
        "web-01",
    );
    // Declared 'b' first, so a sorted or hash-ordered hint reads "a, b" and
    // fails.
    app.root.plugins = two_plugins_scoped_to(ResourceServiceKind::ComputeInstances, ['b', 'a']);

    let idle = project_app_frame(&app.root).operation;
    assert_eq!(
        idle.message, app.root.system.status_message,
        "an unarmed frame must pass the operation's own message through untouched"
    );

    arm(&mut app);

    let armed = project_app_frame(&app.root).operation;
    assert_eq!(
        armed.message.as_deref(),
        Some("Plugin key: b, a (Esc to cancel)"),
        "the armed prompt must list this scope's declared keys in file order"
    );
    assert_eq!(
        armed.message_label, None,
        "the prompt is not an operation's status, so it must not be prefixed with \
         an operation label"
    );

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::CancelPluginPrefix),
    );

    assert_eq!(
        project_app_frame(&app.root).operation.message,
        app.root.system.status_message,
        "a disarmed prefix leaves no prompt behind"
    );
}

/// Re-arming after a refused press prompts as a prompt, not as an error.
///
/// `status_phase` is the phase *of the status message*, and a refusal sets it to
/// `Failed`. The prompt displaces that message, so carrying its phase over would
/// paint the next `,` in the error style and mark it `[ERR]` — telling the
/// operator something has gone wrong at the exact moment nuvem is waiting on
/// them. The sequence is the ordinary one: mistype the key, read the refusal,
/// press `,` again.
#[test]
fn re_arming_after_a_refusal_does_not_inherit_the_failed_phase() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'a',
        DESCRIBE_ARGS,
        "web-01",
    );

    arm(&mut app);
    press(&mut app, 'z');
    let refused = project_app_frame(&app.root).operation;
    assert_eq!(
        refused.phase,
        OperationPhase::Failed,
        "the refusal itself is an error, and the fixture must reach that state \
         or this test pins nothing"
    );

    arm(&mut app);

    let armed = project_app_frame(&app.root).operation;
    assert_eq!(
        armed.message.as_deref(),
        Some("Plugin key: a (Esc to cancel)"),
        "the prompt displaced the refusal"
    );
    assert_ne!(
        armed.phase,
        OperationPhase::Failed,
        "a prompt is not a failure, and the phase it displaced belonged to the \
         message it displaced"
    );
}

/// The other half of the same defect: a *completed operation* that failed must
/// not style the prompt either.
///
/// `status_phase` (the previous test) is not the only road to `[ERR]`.
/// `operation.phase` reaches `Failed` on ten effect results —
/// `runtime::session`'s completion table, where a GCE detail load, a GCE action,
/// a GKE/Cloud SQL/Cloud Storage/Cloud Run detail and the rest each complete as
/// `(OperationPhase::Failed, label)`. Open a detail pane, let the load fail,
/// press `,`: with the operation's phase carried through, `render_status`
/// paints `[ERR] Plugin key: a (Esc to cancel)` in the error style.
///
/// The prompt is not a report on anything that already finished, so a finished
/// phase is neutralised. Work still *in flight* underneath is the exception:
/// that is a true statement about right now, and the operator wants to know a
/// load is still running while they pick a key.
#[test]
fn a_failed_operation_does_not_style_the_prompt_as_an_error() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'a',
        DESCRIBE_ARGS,
        "web-01",
    );
    // The state a failed detail load leaves behind, seeded through the same
    // call `runtime::session` makes on `ComputeInstanceDetailLoadFailed`.
    app.root
        .system
        .complete_operation(OperationPhase::Failed, "GCE detail");
    assert_eq!(
        project_app_frame(&app.root).operation.phase,
        OperationPhase::Failed,
        "the fixture must provably reach the failed phase before arming, or this \
         test pins nothing"
    );

    arm(&mut app);

    let armed = project_app_frame(&app.root).operation;
    assert_eq!(
        armed.message.as_deref(),
        Some("Plugin key: a (Esc to cancel)"),
        "the prompt is on the line"
    );
    assert_ne!(
        armed.phase,
        OperationPhase::Failed,
        "a failed load that already finished must not paint the prompt red"
    );
}

/// Work still running underneath keeps its phase while the prefix is armed: the
/// prompt neutralises phases that *describe something finished*, not a true
/// statement about what is happening right now.
#[test]
fn an_operation_still_in_flight_keeps_its_phase_under_the_prompt() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'a',
        DESCRIBE_ARGS,
        "web-01",
    );
    app.root
        .system
        .begin_operation(OperationPhase::Loading, "GCE instances");

    arm(&mut app);

    assert_eq!(
        project_app_frame(&app.root).operation.phase,
        OperationPhase::Loading,
        "a load still running is still running, and the prompt does not deny it"
    );
}

/// The hint is about the scope in view, not the file: a plugin keyed in another
/// service must not be offered here, because pressing its key would land on the
/// "no plugin bound" refusal.
#[test]
fn the_prompt_lists_only_this_scopes_keys() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'a',
        DESCRIBE_ARGS,
        "web-01",
    );
    let compute = scope_id(ResourceServiceKind::ComputeInstances);
    let sql = scope_id(ResourceServiceKind::CloudSql);
    app.root.plugins = load(&format!(
        "version = 1\n\n\
         [[plugins]]\nid = \"describe\"\nscope = {compute:?}\nkey = \"a\"\n\
         label = \"Describe {{name}}\"\nargs = {DESCRIBE_ARGS:?}\n\n\
         [[plugins]]\nid = \"describe\"\nscope = {sql:?}\nkey = \"z\"\n\
         label = \"Describe {{name}}\"\nargs = {DESCRIBE_ARGS:?}\n"
    ));

    arm(&mut app);

    assert_eq!(
        project_app_frame(&app.root).operation.message.as_deref(),
        Some("Plugin key: a (Esc to cancel)"),
        "the Cloud SQL plugin's key is not pressable from the Compute list"
    );
}

/// Esc cancels without refusing anything: an armed prefix the operator thinks
/// better of is not an error.
#[test]
fn esc_clears_the_prefix_and_sets_no_error() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'a',
        DESCRIBE_ARGS,
        "web-01",
    );
    let before = status(&app).map(str::to_string);

    arm(&mut app);
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::CancelPluginPrefix),
    );

    assert!(
        app.root.plugin_prefix_pending.is_none(),
        "cancelling disarms the prefix"
    );
    assert_eq!(
        status(&app).map(str::to_string),
        before,
        "thinking better of `,` is not an error and must not write the status line"
    );
}

/// The armed prefix owns the keyboard, so the footer must stop advertising the
/// view's own keys: `Enter Open`, `/ Filter` and `J Jumps` are all swallowed
/// while it is armed, and a footer that still lists them is telling the
/// operator to press keys that do nothing.
///
/// `Esc` is the only key here a static footer can name — which characters mean
/// anything is the operator's plugin file, and the status-line prompt lists
/// those. It is also the entry that gives `HelpContext::PluginPrefix`'s catalog
/// a reader in the running app.
#[test]
fn the_armed_prefix_replaces_the_views_footer_with_its_own_one_key() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'a',
        DESCRIBE_ARGS,
        "web-01",
    );
    let unarmed = footer_keys(&app);
    assert!(
        unarmed.len() > 1,
        "the resource list's own footer is the thing being displaced: {unarmed:?}"
    );

    arm(&mut app);

    assert_eq!(
        footer_keys(&app),
        vec![("Esc".to_string(), "Cancel")],
        "an armed prefix swallows every key but Esc, and the footer has to say so"
    );

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::CancelPluginPrefix),
    );

    assert_eq!(
        footer_keys(&app),
        unarmed,
        "disarming gives the view its own footer back"
    );
}

fn footer_keys(app: &AppState) -> Vec<(String, &'static str)> {
    project_app_frame(&app.root)
        .footer_hints
        .into_iter()
        .map(|hint| (hint.key, hint.label))
        .collect()
}

/// `Degraded` is the one phase that survives the prompt without being work in
/// flight, and it has to: it is not a verdict on a finished job but a standing
/// claim about the freshness of the table on screen right now. Neutralised, a
/// `[STALE]` line turns green `[OK]` for as long as the prefix stays armed —
/// the prompt would be repainting a fact it has no opinion about.
#[test]
fn a_degraded_table_stays_degraded_under_the_prompt() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'a',
        DESCRIBE_ARGS,
        "web-01",
    );
    app.root
        .system
        .complete_operation(OperationPhase::Degraded, "GCE instances");
    assert_eq!(
        project_app_frame(&app.root).operation.phase,
        OperationPhase::Degraded,
        "the fixture must provably reach the degraded phase before arming"
    );

    arm(&mut app);

    let armed = project_app_frame(&app.root).operation;
    assert_eq!(
        armed.message.as_deref(),
        Some("Plugin key: a (Esc to cancel)"),
        "the prompt is on the line"
    );
    assert_eq!(
        armed.phase,
        OperationPhase::Degraded,
        "the table is still stale while the operator picks a key"
    );
}

/// A second configuration, activated under the operator's armed prefix. The
/// activation of a gcloud configuration is an async `gcloud` call, so the
/// window between pressing `,` and this result landing is real time, not a
/// theoretical interleaving.
fn other_configuration() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("prod"),
        is_active: true,
        account: Some("sre@example.com".to_string()),
        project: Some("prod-project".to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

/// Activates `prod` the way the runtime does, which lands the remembered — here
/// the default — service for that configuration and so changes the scope under
/// an armed prefix. Not a keyboard action: `reduce` runs `sync_remembered_view`
/// on *every* action, and this one is an `EffectResult`.
fn activate_prod_landing_on(app: &mut AppState, service: ResourceServiceKind) {
    app.root.default_service = service;
    reduce(
        app,
        &Action::EffectResult(EffectResult::ConfigurationActivated {
            configuration: other_configuration(),
        }),
    );
    assert_eq!(
        app.root.resources.active_service(),
        service,
        "the fixture must actually switch the service, or these tests pin nothing"
    );
}

/// The prompt describes the scope the prefix was armed over, so a scope change
/// underneath it leaves no prompt at all — never one advertising a key list
/// that is empty because this scope declares nothing.
///
/// Every reader before this one reasoned about the flag over the *keyboard*:
/// `,` arms, a character or `Esc` clears, everything else is swallowed. That is
/// complete for key-driven actions and silent about the effect results
/// interleaved with them.
#[test]
fn an_effect_driven_service_switch_leaves_no_prompt_for_the_new_scope() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'd',
        DESCRIBE_ARGS,
        "web-01",
    );

    arm(&mut app);
    assert_eq!(
        project_app_frame(&app.root).operation.message.as_deref(),
        Some("Plugin key: d (Esc to cancel)"),
        "the fixture must reach the armed prompt before the switch"
    );

    activate_prod_landing_on(&mut app, ResourceServiceKind::GkeClusters);

    assert_eq!(
        project_app_frame(&app.root).operation.message,
        app.root.system.status_message,
        "GKE declares no plugin here, and `,` would have refused to arm over it: \
         the prompt must be gone rather than advertising an empty key list"
    );
}

/// The other half: the press must not resolve the *new* scope's plugin.
///
/// One character legally keys a plugin in two scopes — uniqueness is per scope,
/// which is why `one_character_keyed_in_two_scopes_answers_with_each_scopes_own_plugin`
/// exists — so an armed prefix that survives the switch answers with a plugin
/// the operator was never offered. Here the operator armed `,` over the Compute
/// list meaning `d` = describe, and the surviving flag would hand them GKE's
/// `d` = delete.
#[test]
fn an_effect_driven_service_switch_stops_the_press_answering_for_the_new_scope() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        'd',
        DESCRIBE_ARGS,
        "web-01",
    );
    let compute = scope_id(ResourceServiceKind::ComputeInstances);
    let gke = scope_id(ResourceServiceKind::GkeClusters);
    app.root.plugins = load(&format!(
        "version = 1\n\n\
         [[plugins]]\nid = \"describe\"\nscope = {compute:?}\nkey = \"d\"\n\
         label = \"Describe {{name}}\"\nargs = {DESCRIBE_ARGS:?}\n\n\
         [[plugins]]\nid = \"delete\"\nscope = {gke:?}\nkey = \"d\"\ndangerous = true\n\
         label = \"Delete {{name}}\"\n\
         args = [\"container\", \"clusters\", \"delete\", \"{{name}}\"]\n"
    ));

    arm(&mut app);
    activate_prod_landing_on(&mut app, ResourceServiceKind::GkeClusters);
    // The switch reloads, and that load lands too: the new scope has a row for
    // the wrong plugin to compose against, so a surviving prefix reaches a
    // confirmation rather than stopping at "nothing selected".
    let context = app
        .root
        .current_context()
        .expect("prod is the active context now");
    reduce(
        &mut app,
        &Action::EffectResult(row_loaded(
            ResourceServiceKind::GkeClusters,
            context,
            "prod-cluster",
        )),
    );

    press(&mut app, 'd');

    assert_eq!(
        app.root
            .resources
            .plugin_confirmation()
            .map(|confirmation| confirmation.argv().to_vec()),
        None,
        "the press must not compose GKE's delete from a prefix armed over Compute"
    );
    assert_eq!(
        status(&app),
        Some("Plugin not run: the view changed from GCE instances to GKE clusters"),
        "the keystroke was consumed by a scope the operator did not arm, and \
         silence there is indistinguishable from a plugin that ran"
    );
    assert!(
        app.root.plugin_prefix_pending.is_none(),
        "the press consumes the prefix on this path too"
    );
}
