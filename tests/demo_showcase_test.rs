//! The showcase corpus: what `nuvem --demo` lists.
//!
//! Three properties. Every backend paints the same rows from it, as the
//! module documentation of `services::demo::sdk` requires of every demo
//! document: the native stubs and the CLI parsers read the same bytes. A
//! demo session over it lists a credible project: several rows per service and
//! no row a reader cannot name. And it names nothing real: every project,
//! email and address in it is a fixture's.

use nuvem::app::AppState;
use nuvem::app::projection::project_resource_list;
use nuvem::app::{Action, ResourcesAction, WorkspaceAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::resource::ResourceServiceKind;

const PROJECT: &str = "dev-project";

macro_rules! showcase {
    ($file:literal) => {
        include_str!(concat!("fixtures/demo/", $file))
    };
}

/// Every showcase document, by file name.
const DOCUMENTS: &[(&str, &str)] = &[
    ("cloud_functions.json", showcase!("cloud_functions.json")),
    (
        "cloud_run_revisions_v2.json",
        showcase!("cloud_run_revisions_v2.json"),
    ),
    (
        "cloud_run_services_v2.json",
        showcase!("cloud_run_services_v2.json"),
    ),
    ("gce_instances.json", showcase!("gce_instances.json")),
    ("gcs_buckets_v2.json", showcase!("gcs_buckets_v2.json")),
    ("gke_clusters.json", showcase!("gke_clusters.json")),
    (
        "iam_service_accounts.json",
        showcase!("iam_service_accounts.json"),
    ),
    ("persistent_disks.json", showcase!("persistent_disks.json")),
    (
        "pubsub_subscriptions.json",
        showcase!("pubsub_subscriptions.json"),
    ),
    ("pubsub_topics.json", showcase!("pubsub_topics.json")),
    ("secrets.json", showcase!("secrets.json")),
    (
        "vpc_firewall_rules.json",
        showcase!("vpc_firewall_rules.json"),
    ),
    ("vpc_networks.json", showcase!("vpc_networks.json")),
    ("vpc_subnets.json", showcase!("vpc_subnets.json")),
];

/// The list above is the directory, so a showcase file cannot be added
/// without this test reading it.
#[test]
fn every_showcase_file_is_checked() {
    let mut on_disk: Vec<String> = std::fs::read_dir("tests/fixtures/demo")
        .expect("the showcase directory exists")
        .map(|entry| {
            entry
                .expect("readable")
                .file_name()
                .into_string()
                .expect("utf-8")
        })
        .collect();
    on_disk.sort();
    let listed: Vec<String> = DOCUMENTS.iter().map(|(name, _)| name.to_string()).collect();
    assert_eq!(on_disk, listed);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_native_and_cli_backends_paint_the_same_showcase_rows() {
    use nuvem::services::resources::{compute, disks, functions, gke, iam, pubsub, secrets, vpc};
    use nuvem::services::sdk;

    let mut native = sdk::compute::SdkComputeService::from_showcase()
        .load_instances(PROJECT)
        .await
        .expect("answers");
    let mut cli = compute::parse_instances_json(showcase!("gce_instances.json")).expect("parses");
    native.sort_by(|a, b| a.name.cmp(&b.name));
    cli.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(native, cli, "GCE instances");

    let native = sdk::gke::SdkGkeService::from_showcase()
        .load_clusters(PROJECT)
        .await
        .expect("answers");
    assert_eq!(
        native,
        gke::parse_clusters_json(showcase!("gke_clusters.json")).expect("parses"),
        "GKE clusters"
    );

    let native = sdk::secrets::SdkSecretsService::from_showcase()
        .load_secrets(PROJECT)
        .await
        .expect("answers");
    assert_eq!(
        native,
        secrets::parse_secrets_json(showcase!("secrets.json")).expect("parses"),
        "secrets"
    );

    let native = sdk::functions::SdkFunctionsService::from_showcase()
        .load_functions(PROJECT)
        .await
        .expect("answers");
    assert_eq!(
        native,
        functions::parse_functions_json(showcase!("cloud_functions.json")).expect("parses"),
        "Cloud Functions"
    );

    let mut native = sdk::disks::SdkDisksService::from_showcase()
        .load_disks(PROJECT)
        .await
        .expect("answers");
    let mut cli = disks::parse_disks_json(showcase!("persistent_disks.json")).expect("parses");
    native.sort_by(|a, b| a.name.cmp(&b.name));
    cli.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(native, cli, "persistent disks");

    let service = sdk::pubsub::SdkPubSubService::from_showcase();
    let mut topics = pubsub::parse_topics_json(showcase!("pubsub_topics.json")).expect("parses");
    let subscriptions =
        pubsub::parse_subscriptions_json(showcase!("pubsub_subscriptions.json")).expect("parses");
    pubsub::apply_subscription_counts(&mut topics, &subscriptions);
    assert_eq!(
        service.load_topics(PROJECT).await.expect("answers"),
        topics,
        "topics"
    );
    assert_eq!(
        service.load_subscriptions(PROJECT).await.expect("answers"),
        subscriptions,
        "subscriptions"
    );

    let native = sdk::iam::SdkIamService::from_showcase()
        .load_service_accounts(PROJECT)
        .await
        .expect("answers");
    assert_eq!(
        native,
        iam::parse_service_accounts_json(showcase!("iam_service_accounts.json")).expect("parses"),
        "service accounts"
    );

    let service = sdk::vpc::SdkVpcService::from_showcase();
    assert_eq!(
        service.load_networks(PROJECT).await.expect("answers"),
        vpc::parse_networks_json(showcase!("vpc_networks.json")).expect("parses"),
        "VPC networks"
    );
    assert_eq!(
        service.load_firewall_rules(PROJECT).await.expect("answers"),
        vpc::parse_firewall_rules_json(showcase!("vpc_firewall_rules.json")).expect("parses"),
        "firewall rules"
    );
    let mut native = service.load_subnets(PROJECT).await.expect("answers");
    let mut cli = vpc::parse_subnets_json(showcase!("vpc_subnets.json")).expect("parses");
    native.sort_by(|a, b| (&a.region, &a.name).cmp(&(&b.region, &b.name)));
    cli.sort_by(|a, b| (&a.region, &a.name).cmp(&(&b.region, &b.name)));
    assert_eq!(native, cli, "VPC subnets");
}

/// Each Cloud Run service lists its own revisions, not another service's.
#[tokio::test(flavor = "multi_thread")]
async fn each_showcase_service_lists_its_own_revisions() {
    let service = nuvem::services::sdk::cloud_run::SdkCloudRunService::from_showcase();
    let services = service.load_services(PROJECT).await.expect("answers");
    assert!(services.len() >= 6, "a credible number of services");
    for run in &services {
        let revisions = service
            .load_revisions(PROJECT, &run.region, &run.name)
            .await
            .expect("answers");
        assert!(!revisions.is_empty(), "{} lists its revisions", run.name);
        for revision in revisions {
            assert!(
                revision.name.starts_with(&format!("{}-", run.name)),
                "{} lists {}, another service's revision",
                run.name,
                revision.name
            );
        }
    }
}

/// A demo session over the showcase lists a credible project: every service
/// has several rows, and no row is missing the value that names it.
///
/// Walks `ResourceServiceKind::ALL`. A service whose first column is blank on
/// some row — a service account with no email, a disk with no name — is the
/// row a reader cannot name, and the thing that made the old demo look broken.
#[tokio::test(flavor = "multi_thread")]
async fn a_demo_session_lists_several_named_rows_for_every_service() {
    use clap::Parser;
    for kind in ResourceServiceKind::ALL {
        let cli =
            nuvem::Cli::parse_from(["nuvem", "--demo", "--config", "/nonexistent/nuvem.toml"]);
        let config = nuvem::Config::resolve(&cli).config;
        let state_path = nuvem::config::state::state_path_for(&config.config_path);
        let mut session = RuntimeSession::new(config, cli, AppState::new(), state_path);
        session.initialize().await;
        settle(&mut session).await;
        for action in [
            Action::Workspace(WorkspaceAction::Resources),
            Action::Resources(ResourcesAction::SelectService(kind)),
        ] {
            session.dispatch_action(action).await;
            settle(&mut session).await;
        }
        let table = project_resource_list(&session.app().root.resources)
            .table
            .unwrap_or_else(|| panic!("{kind:?} paints no table"));
        assert!(
            (4..=12).contains(&table.rows.len()),
            "{kind:?} lists {} rows; a credible demo project has several and a \
             screenful at most",
            table.rows.len()
        );
        for row in &table.rows {
            assert!(
                !row.cells[0].text.trim().is_empty(),
                "{kind:?} lists a row with no name: {:?}",
                row.cells
                    .iter()
                    .map(|cell| cell.text.as_str())
                    .collect::<Vec<_>>()
            );
        }
    }
}

async fn settle(session: &mut RuntimeSession) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let in_flight = session.effect_tasks_in_flight();
        let progressed = matches!(
            tokio::time::timeout(
                std::time::Duration::from_millis(5),
                session.process_next_effect_result()
            )
            .await,
            Ok(true)
        );
        if !progressed && in_flight == 0 {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "the session settles");
    }
}

/// Nothing in the showcase names a real project, account or address.
///
/// Every project path names `dev-project` (or the `_` Storage uses, or a
/// project number); every email is at `example.com` or is a `dev-project`
/// service account; every IPv4 address is private, from the documentation
/// ranges, the whole internet (`0.0.0.0/0`), or one of the ranges Google
/// documents for its own health checks, IAP and APIs.
#[test]
fn the_showcase_names_only_fixtures() {
    let allowed_public: [[u8; 2]; 4] = [[35, 235], [130, 211], [35, 191], [199, 36]];
    for (file, text) in DOCUMENTS {
        for (at, _) in text.match_indices("projects/") {
            let project: String = text[at + "projects/".len()..]
                .chars()
                .take_while(|c| *c != '/' && *c != '"')
                .collect();
            assert!(
                project == PROJECT || project == "_" || project.chars().all(|c| c.is_ascii_digit()),
                "{file} names project {project:?}"
            );
        }
        for word in text.split(['"', ' ', ',', '\'']) {
            if let Some((_, domain)) = word.split_once('@') {
                assert!(
                    domain == "example.com"
                        || domain == format!("{PROJECT}.iam.gserviceaccount.com")
                        || domain == "developer.gserviceaccount.com",
                    "{file} names an address at {domain}"
                );
            }
            let address = word.split('/').next().unwrap_or("");
            let octets: Vec<u8> = address.split('.').filter_map(|o| o.parse().ok()).collect();
            if octets.len() == 4 && address.split('.').count() == 4 {
                let ok = matches!(octets[..], [10, ..] | [192, 168, ..] | [0, 0, 0, 0])
                    || (octets[0] == 172 && (16..32).contains(&octets[1]))
                    || matches!(octets[..3], [192, 0, 2] | [198, 51, 100] | [203, 0, 113])
                    || allowed_public.contains(&[octets[0], octets[1]]);
                assert!(ok, "{file} names the address {address}");
            }
        }
    }
}
