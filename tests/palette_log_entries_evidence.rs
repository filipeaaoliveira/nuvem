//! Visual evidence — every service's log jump is on the palette
//! an operator actually opens.
//!
//! `tests/reducer_command_test.rs` pins the descriptors and the guards at the
//! reducer. This binary answers the other half: with each service on screen
//! and a row under the cursor, the operator types `logs` into the palette and
//! the entry for THAT service is on the list, spelled the way it has always
//! been spelled. The frames go through the real `nuvem::ui::render` path, so
//! what is asserted is what is drawn.
//!
//! The labels below are pinned as literals rather than read back from
//! `CommandDescriptor::label`, because the point of the pin is that collapsing
//! six hand-written variants into one generated from `ResourceServiceKind`
//! left the six visible strings byte-for-byte unchanged. Reading the label
//! from the code under test would agree with any rewording.
//!
//! Frames are written only when `NUVEM_EVIDENCE_DIR` is set, and are written
//! before the assertions run so a frame is captured even on a failing run.

use std::fs;
use std::path::PathBuf;

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::command::CommandDescriptor;
use nuvem::app::{
    Action, AppState, CommandAction, EffectResult, ResourcesAction, Workspace, WorkspaceAction,
};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    BigQueryDataset, CloudFunction, CloudFunctionEnvironment, CloudFunctionState, CloudRunService,
    CloudRunServiceStatus, CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, ComputeInstance,
    DiskStatus, EndpointVisibility, FirewallAction, FirewallDirection, FirewallRule,
    FirewallTargets, GkeCluster, GkeClusterStatus, IamPolicyMember, PersistentDisk, PubSubDelivery,
    PubSubSubscription, PubSubSubscriptionState, PubSubTopic, ResourceServiceKind, ResourceStatus,
    Secret, ServiceAccount, StorageBucket, SubscriptionCount, VpcNetwork, VpcRoutingMode,
    VpcSubnet, VpcSubnetMode,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use crate::support::frame_svg;

use frame_svg::buffer_to_svg;

const WIDTH: u16 = 120;
const HEIGHT: u16 = 26;

/// The palette line each service must show, as it read before this refactor,
/// or `None` for a service that has no logs to offer.
///
/// `None` is not a skip: the walk below turns it into the opposite assertion —
/// that no log row and no log descriptor appear for that service — so a
/// service wired to a query that can only ever return zero rows fails here
/// rather than passing by omission.
fn expected_label(kind: ResourceServiceKind) -> Option<&'static str> {
    Some(match kind {
        ResourceServiceKind::ComputeInstances => "Open logs for selected GCE instance",
        ResourceServiceKind::GkeClusters => "Open logs for selected GKE cluster",
        ResourceServiceKind::CloudRunServices => "Open logs for selected Cloud Run service",
        ResourceServiceKind::CloudSql => "Open logs for selected Cloud SQL instance",
        ResourceServiceKind::CloudStorage => "Open logs for selected Cloud Storage bucket",
        ResourceServiceKind::SecretManagerSecrets => "Open logs for selected secret",
        ResourceServiceKind::CloudFunctions => "Open logs for selected Cloud Function",
        // Nothing writes logs *as* a disk: a disk's lifecycle appears in the
        // audit log of the instance or operation that touched it. A
        // resource-scoped query here could only ever return zero rows, which
        // on screen reads as "this disk has no logs" rather than as "disks do
        // not have logs".
        ResourceServiceKind::PersistentDisks => return None,
        ResourceServiceKind::PubSubTopics => "Open logs for selected Pub/Sub topic",
        ResourceServiceKind::PubSubSubscriptions => "Open logs for selected Pub/Sub subscription",
        // A service account is an identity that appears *in* log entries as
        // the caller, not a resource with a log stream of its own — the disks
        // refusal, for the same reason.
        ResourceServiceKind::IamServiceAccounts => return None,
        // A policy binding is a grant, not a resource: nothing writes log
        // entries as one, so the same refusal.
        ResourceServiceKind::IamPolicy => return None,
        ResourceServiceKind::BigQueryDatasets => "Open logs for selected BigQuery dataset",
        // The entries an operator wants for a network — VPC flow logs and
        // firewall hit logs — are written against the *subnetwork*
        // (`resource.type="gce_subnetwork"`), never the network. A
        // network-scoped query could only ever return zero rows, and on screen
        // that reads as "this network has no traffic": the disks refusal, for
        // a more dangerous version of the same reason.
        ResourceServiceKind::VpcNetworks => return None,
        // The other half of that refusal: a subnet *is* where flow logs and
        // firewall hit logs live (`resource.type="gce_subnetwork"`), so this
        // is the one VPC table that offers the entry — and offering it here is
        // what makes refusing it on networks a redirection rather than a dead
        // end.
        ResourceServiceKind::VpcSubnets => "Open logs for selected VPC subnet",
        // The third refusal in this family, and the one whose redirection is
        // most likely to be wanted: a firewall rule's *hits* are logged, but
        // they are logged against the subnetwork the traffic crossed
        // (`resource.type="gce_subnetwork"`), tagged with the rule's name in
        // the payload. Nothing writes an entry whose resource *is* the rule.
        // A rule-scoped query would return zero rows, and on a DENY rule zero
        // rows reads as "nothing was blocked" — the most dangerous of the
        // three empty results this walk refuses.
        ResourceServiceKind::FirewallRules => return None,
    })
}

/// A short slug for the evidence file names.
fn slug(kind: ResourceServiceKind) -> &'static str {
    match kind {
        ResourceServiceKind::ComputeInstances => "compute",
        ResourceServiceKind::GkeClusters => "gke",
        ResourceServiceKind::CloudRunServices => "cloud-run",
        ResourceServiceKind::CloudSql => "cloud-sql",
        ResourceServiceKind::CloudStorage => "storage",
        ResourceServiceKind::SecretManagerSecrets => "secrets",
        ResourceServiceKind::CloudFunctions => "cloud-functions",
        ResourceServiceKind::PersistentDisks => "persistent-disks",
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

fn evidence_dir() -> Option<PathBuf> {
    let dir = std::env::var("NUVEM_EVIDENCE_DIR").ok()?;
    let path = PathBuf::from(dir);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

fn text(buffer: &Buffer) -> String {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_frame(name: &str, title: &str, caption: &str, buffer: &Buffer) {
    let Some(dir) = evidence_dir() else {
        eprintln!("NUVEM_EVIDENCE_DIR not set; skipping evidence render");
        return;
    };
    fs::write(
        dir.join(format!("{name}.svg")),
        buffer_to_svg(buffer, title, caption),
    )
    .expect("write svg frame");
    fs::write(
        dir.join(format!("{name}.txt")),
        format!("{title}\n{caption}\n\n{}\n", text(buffer)),
    )
    .expect("write ascii frame");
}

fn config() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

/// The state each service is in when its log jump should be on offer: the
/// service active in the Resources workspace, a context resolved, and a row
/// under the cursor. Exhaustive on purpose — a service added to
/// `ResourceServiceKind::ALL` has to say what that state is for it before this
/// walk will compile.
fn app_showing_service(kind: ResourceServiceKind) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(kind)),
    );
    let context = app.root.current_context().expect("context exists");
    let loaded = match kind {
        ResourceServiceKind::ComputeInstances => EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![ComputeInstance {
                id: "api-dev-1-id".to_string(),
                name: "api-dev-1".to_string(),
                zone: "europe-west1-b".to_string(),
                status: ResourceStatus::Running,
                machine_type: "e2-medium".to_string(),
                internal_ip: Some("10.0.0.2".to_string()),
                external_ip: None,
            }],
        },
        ResourceServiceKind::GkeClusters => EffectResult::GkeClustersLoaded {
            context,
            clusters: vec![GkeCluster {
                name: "primary".to_string(),
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
                name: "db-dev-1".to_string(),
                database_version: "POSTGRES_15".to_string(),
                engine: "PostgreSQL".to_string(),
                region: "us-central1".to_string(),
                tier: "db-custom-2-7680".to_string(),
                availability: CloudSqlAvailability::Zonal,
                status: CloudSqlStatus::Runnable,
                connectivity: EndpointVisibility::Public,
            }],
        },
        ResourceServiceKind::CloudRunServices => EffectResult::CloudRunServicesLoaded {
            context,
            services: vec![CloudRunService {
                name: "api".to_string(),
                region: "us-central1".to_string(),
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
                name: "assets-bucket".to_string(),
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
                name: "api-signing-key".to_string(),
                create_time: Some("2026-02-11T09:14:22Z".to_string()),
                labels: Vec::new(),
                replication: "Automatic".to_string(),
            }],
        },
        ResourceServiceKind::CloudFunctions => EffectResult::CloudFunctionsLoaded {
            context,
            functions: vec![CloudFunction {
                name: "thumbnailer".to_string(),
                region: "europe-west1".to_string(),
                state: CloudFunctionState::Active,
                environment: CloudFunctionEnvironment::Gen2,
                url: Some("https://europe-west1-dev.cloudfunctions.net/thumbnailer".to_string()),
                update_time: Some("2026-03-01T10:00:00Z".to_string()),
                labels: Vec::new(),
            }],
        },
        ResourceServiceKind::PersistentDisks => EffectResult::PersistentDisksLoaded {
            context,
            disks: vec![PersistentDisk {
                name: "detached-legacy".to_string(),
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
                name: "orders-worker".to_string(),
                full_name: "projects/dev-project/subscriptions/orders-worker".to_string(),
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
                name: "orders".to_string(),
                full_name: "projects/dev-project/topics/orders".to_string(),
                subscription_count: SubscriptionCount::Known(2),
                message_retention: Some("604800s".to_string()),
                kms_key: None,
                labels: Vec::new(),
            }],
        },
        ResourceServiceKind::VpcNetworks => EffectResult::VpcNetworksLoaded {
            context,
            networks: vec![VpcNetwork {
                name: "default".to_string(),
                subnet_mode: VpcSubnetMode::Auto,
                routing_mode: VpcRoutingMode::Regional,
                subnet_count: 43,
                description: None,
            }],
        },
        ResourceServiceKind::VpcSubnets => EffectResult::VpcSubnetsLoaded {
            context,
            subnets: vec![VpcSubnet {
                name: "default".to_string(),
                region: "us-central1".to_string(),
                network: "default".to_string(),
                ip_cidr_range: "10.128.0.0/20".to_string(),
                gateway_address: Some("10.128.0.1".to_string()),
                private_google_access: false,
            }],
        },
        // CONSTRUCTED, not captured: no project this account can reach has an
        // egress rule or a `denied` rule, so this row is transcribed from
        // Google's published `compute/v1/firewalls` reference. It is the DENY
        // case on purpose — a DENY rendered as an ALLOW is the misreading this
        // service most has to avoid, so the palette evidence should show one.
        ResourceServiceKind::FirewallRules => EffectResult::FirewallRulesLoaded {
            context,
            rules: vec![FirewallRule {
                name: "deny-all-egress".to_string(),
                network: "default".to_string(),
                direction: FirewallDirection::Egress,
                priority: Some(65535),
                action: FirewallAction::Deny,
                targets: FirewallTargets::AllInstances,
                source_or_dest: vec!["0.0.0.0/0".to_string()],
                disabled: false,
            }],
        },
        ResourceServiceKind::IamServiceAccounts => EffectResult::ServiceAccountsLoaded {
            context,
            accounts: vec![ServiceAccount {
                email: "deployer@dev-project.iam.gserviceaccount.com".to_string(),
                display_name: Some("Deployer".to_string()),
                disabled: false,
                unique_id: "100000000000000000001".to_string(),
            }],
        },
        ResourceServiceKind::IamPolicy => EffectResult::IamPolicyLoaded {
            context,
            members: vec![IamPolicyMember::from_wire(
                "roles/viewer",
                "user:alice@example.com",
                None,
            )],
        },
        ResourceServiceKind::BigQueryDatasets => EffectResult::BigQueryDatasetsLoaded {
            context,
            datasets: vec![BigQueryDataset {
                dataset_id: "analytics_events".to_string(),
                project_id: "dev-project".to_string(),
                location: "europe-west1".to_string(),
                friendly_name: None,
                labels: Vec::new(),
            }],
        },
    };
    reduce(&mut app, &Action::EffectResult(loaded));
    app
}

fn draw(session: &RuntimeSession) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("terminal init");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render");
    terminal.backend().buffer().clone()
}

/// The operator's route: open the palette over a live service list, type the
/// word they reach for, read the entry off the screen.
#[tokio::test]
async fn every_service_shows_its_log_entry_in_the_rendered_palette() {
    for kind in ResourceServiceKind::ALL {
        let app = app_showing_service(kind);
        assert_eq!(app.root.workspace, Workspace::Resources);
        assert_eq!(app.root.resources.active_service(), kind);

        let mut session = RuntimeSession::new_for_test(app);
        session
            .dispatch_action(Action::Command(CommandAction::Open))
            .await;
        for ch in "logs".chars() {
            session
                .dispatch_action(Action::Command(CommandAction::AppendChar(ch)))
                .await;
        }

        let frame = draw(&session);
        let Some(label) = expected_label(kind) else {
            // A service with no logs must offer no log row and no log
            // descriptor. Asserted rather than skipped: a disk wired to a
            // resource-scoped query would draw a row here and pass a `continue`.
            let screen = text(&frame);
            assert!(
                !screen.contains(&format!("Open logs for selected {}", kind.selection_noun())),
                "{kind:?} produces no logs, but the palette drew a log row for \
                 it:\n{screen}"
            );
            assert!(
                !session
                    .app()
                    .root
                    .command
                    .visible_commands(&session.app().root)
                    .contains(&CommandDescriptor::OpenLogsForSelected(kind)),
                "{kind:?} produces no logs, but offers a log descriptor"
            );
            continue;
        };
        write_frame(
            &format!("palette-{}", slug(kind)),
            &format!("Palette over {} with `logs` typed", kind.label()),
            &format!("The row an operator needs is on screen: \"{label}\"."),
            &frame,
        );

        let screen = text(&frame);
        assert!(
            screen.contains(label),
            "the palette drew no \"{label}\" row for {kind:?}, leaving the L \
             keybinding as the only route to its logs:\n{screen}"
        );
        assert!(
            session
                .app()
                .root
                .command
                .visible_commands(&session.app().root)
                .contains(&CommandDescriptor::OpenLogsForSelected(kind)),
            "{kind:?} must offer its generated log descriptor, not merely a \
             row that reads like one"
        );
    }
}
