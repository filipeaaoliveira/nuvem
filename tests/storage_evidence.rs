//! Evidence generator — Cloud Storage folded into the generic
//! resource collection state.
//!
//! Renders the real end-user surfaces through `nuvem::ui::render` (the exact
//! terminal buffer an operator sees) and writes ASCII "screenshots" to the
//! evidence directory when `NUVEM_EVIDENCE_DIR` is set (assertions always run).
//! Demonstrates: the bucket list served by the embedded
//! `ResourceCollectionState`, the bucket-detail pane whose tabs come from the
//! merged `StorageDetailTab` and whose load failure flows through the generic
//! `detail_error` channel (with the GKE-style detail-target guard dropping
//! stale results for other buckets), and the deliberately bespoke object
//! browser — page listing, partial-page error banner, and object-detail
//! error pane — all unchanged.

use std::fs;
use std::path::PathBuf;

use nuvem::app::{Action, AppError, AppState, EffectResult, ResourcesAction, WorkspaceAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    ComputeKeyValue, ResourceServiceKind, StorageBucket, StorageBucketDetail, StorageObject,
    StorageObjectPage, StoragePrefix,
};
use ratatui::{Terminal, backend::TestBackend};

fn evidence_dir() -> Option<PathBuf> {
    let dir = std::env::var("NUVEM_EVIDENCE_DIR").ok()?;
    let path = PathBuf::from(dir);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

fn write_evidence(name: &str, header: &str, lines: &[String]) {
    let Some(dir) = evidence_dir() else {
        return;
    };
    let mut body = String::new();
    body.push_str(header);
    body.push_str("\n\n");
    let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    body.push('+');
    body.push_str(&"-".repeat(width));
    body.push_str("+\n");
    for line in lines {
        body.push('|');
        body.push_str(line);
        body.push_str("|\n");
    }
    body.push('+');
    body.push_str(&"-".repeat(width));
    body.push_str("+\n");
    fs::write(dir.join(name), body).expect("write evidence");
    println!("wrote {name}");
}

fn render_lines(session: &RuntimeSession, width: u16, height: u16) -> Vec<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render");
    let buffer = terminal.backend().buffer();
    let w = buffer.area.width as usize;
    buffer
        .content()
        .chunks(w)
        .map(|line| {
            line.iter()
                .map(|c| c.symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

async fn load_dev(session: &mut RuntimeSession) {
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: Some("us-east1".to_string()),
                zone: None,
            }],
        })
        .await;
}

fn bucket(name: &str, location: &str, storage_class: &str) -> StorageBucket {
    StorageBucket {
        name: name.to_string(),
        location: location.to_string(),
        location_type: Some("region".to_string()),
        storage_class: storage_class.to_string(),
        versioning_enabled: false,
        uniform_bucket_level_access: true,
        public_access_prevention: Some("enforced".to_string()),
        time_created: Some("2024-01-15T10:00:00Z".to_string()),
        updated: Some("2026-07-01T08:30:00Z".to_string()),
    }
}

fn bucket_detail(name: &str) -> StorageBucketDetail {
    StorageBucketDetail {
        name: name.to_string(),
        location: "us-east1".to_string(),
        location_type: Some("region".to_string()),
        storage_class: "STANDARD".to_string(),
        versioning_enabled: true,
        uniform_bucket_level_access: true,
        public_access_prevention: Some("enforced".to_string()),
        requester_pays: false,
        retention_period: None,
        default_kms_key_present: false,
        labels: vec![ComputeKeyValue {
            key: "env".to_string(),
            value: "prod".to_string(),
        }],
        time_created: Some("2024-01-15T10:00:00Z".to_string()),
        updated: Some("2026-07-01T08:30:00Z".to_string()),
        raw_json: "{\n  \"name\": \"cold-archive\"\n}".to_string(),
    }
}

async fn select_storage(session: &mut RuntimeSession, buckets: Vec<StorageBucket>) {
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::StorageBucketsLoaded { context, buckets })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
}

/// Bucket list from the embedded generic collection, the merged
/// `StorageDetailTab` tabs on the bucket-detail pane, the generic
/// `detail_error` channel (plus its stale-result target guard), and the
/// bespoke object browser with its partial-page and object-detail error
/// channels intact.
#[tokio::test]
async fn storage_surfaces_render_for_review() {
    // --- 1. Bucket list served by ResourceCollectionState ----------------
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev(&mut session).await;
    select_storage(
        &mut session,
        vec![
            bucket("prod-assets", "us-east1", "STANDARD"),
            bucket("staging-logs", "us-east1", "NEARLINE"),
            bucket("cold-archive", "us-central1", "ARCHIVE"),
        ],
    )
    .await;
    let list_lines = render_lines(&session, 190, 24);
    assert!(
        list_lines.iter().any(|l| l.contains("prod-assets"))
            && list_lines.iter().any(|l| l.contains("cold-archive")),
        "bucket rows must render from the embedded generic collection:\n{}",
        list_lines.join("\n")
    );
    write_evidence(
        "01_bucket_list.txt",
        "Cloud Storage bucket list (g -> Cloud Storage): rows now served by the embedded \
         ResourceCollectionState<StorageBucket, ...> — same list surface as Compute/GKE/CloudSql/CloudRun.",
        &list_lines,
    );

    // --- 2. Bucket detail: merged StorageDetailTab tabs -------------------
    let context = session.app().root.current_context().unwrap().clone();
    session
        .dispatch_action(Action::Resources(ResourcesAction::OpenBucketMetadata))
        .await;
    session
        .apply_effect_result(EffectResult::StorageBucketDetailLoaded {
            context: context.clone(),
            detail: bucket_detail("cold-archive"),
        })
        .await;
    let detail_lines = render_lines(&session, 190, 30);
    let joined = detail_lines.join("\n");
    for tab in ["Summary", "Metadata", "JSON"] {
        assert!(
            joined.contains(tab),
            "merged StorageDetailTab must render the {tab} tab in the bucket pane:\n{joined}"
        );
    }
    write_evidence(
        "02_bucket_detail_summary.txt",
        "Bucket-detail pane for cold-archive (the selected row under Name-ascending sort): tab strip (Summary / Metadata / JSON) is the single \
         merged StorageDetailTab enum; detail body stored in the generic collection.detail slot.",
        &detail_lines,
    );

    // --- 3. Stale detail result for another bucket is ignored -------------
    session
        .apply_effect_result(EffectResult::StorageBucketDetailLoaded {
            context: context.clone(),
            detail: bucket_detail("other-bucket"),
        })
        .await;
    let guarded = render_lines(&session, 190, 30).join("\n");
    assert!(
        guarded.contains("cold-archive") && !guarded.contains("other-bucket"),
        "a detail result for a bucket that is not the active detail_target must be dropped \
         (GKE-style target guard):\n{guarded}"
    );

    // --- 4. Bucket detail load failure -> generic detail_error channel ----
    session
        .dispatch_action(Action::Resources(ResourcesAction::CloseDetail))
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectNext))
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::OpenBucketMetadata))
        .await;
    session
        .apply_effect_result(EffectResult::StorageBucketDetailLoadFailed {
            context: context.clone(),
            bucket_name: "prod-assets".to_string(),
            error: AppError::gcloud(
                "PERMISSION_DENIED: storage.buckets.get access denied on prod-assets",
            ),
        })
        .await;
    let error_lines = render_lines(&session, 190, 30);
    let error_joined = error_lines.join("\n");
    assert!(
        error_joined.contains("Detail load failed for prod-assets")
            && error_joined.contains("PERMISSION_DENIED"),
        "bucket-detail failure must surface through the generic detail_error channel:\n{error_joined}"
    );
    write_evidence(
        "03_bucket_detail_error.txt",
        "Bucket-detail load failure for prod-assets: the old bespoke bucket_detail_error field is \
         gone — this error pane is fed by the generic collection.detail_error channel \
         (detail_error_target_name = prod-assets), matching the other resource services' UX.",
        &error_lines,
    );

    // --- 5. Bespoke object browser unchanged ------------------------------
    session
        .dispatch_action(Action::Resources(ResourcesAction::CloseDetail))
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::StorageObjectPageLoaded {
            context: context.clone(),
            bucket: "prod-assets".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: vec![StoragePrefix {
                    name: "2026/".to_string(),
                }],
                objects: vec![
                    StorageObject {
                        key: "access.log".to_string(),
                        size_bytes: Some(52_428),
                        storage_class: Some("NEARLINE".to_string()),
                        generation: Some("1712000000000001".to_string()),
                        updated: Some("2026-07-18T21:11:00Z".to_string()),
                        content_type: Some("text/plain".to_string()),
                        kms_encrypted: false,
                        temporary_hold: false,
                        event_based_hold: false,
                        has_retention: false,
                    },
                    StorageObject {
                        key: "error.log".to_string(),
                        size_bytes: Some(1_024),
                        storage_class: Some("NEARLINE".to_string()),
                        generation: Some("1712000000000002".to_string()),
                        updated: Some("2026-07-19T06:42:00Z".to_string()),
                        content_type: Some("text/plain".to_string()),
                        kms_encrypted: false,
                        temporary_hold: false,
                        event_based_hold: false,
                        has_retention: false,
                    },
                ],
                next_page_token: Some("page-2-token".to_string()),
            },
        })
        .await;
    let browser_lines = render_lines(&session, 190, 24);
    let browser_joined = browser_lines.join("\n");
    assert!(
        browser_joined.contains("2026/") && browser_joined.contains("access.log"),
        "object browser (bespoke, per ticket) must still list prefixes and objects:\n{browser_joined}"
    );
    write_evidence(
        "04_object_browser.txt",
        "Object browser inside prod-assets (Enter on the bucket row): prefix + object rows with a \
         further page available — this surface deliberately stays service-specific.",
        &browser_lines,
    );

    // --- 6. Partial-page error stays on its bespoke channel ---------------
    session
        .dispatch_action(Action::Resources(ResourcesAction::NextPage))
        .await;
    session
        .apply_effect_result(EffectResult::StorageObjectPageLoadFailed {
            context: context.clone(),
            bucket: "prod-assets".to_string(),
            prefix: String::new(),
            append: true,
            error: AppError::gcloud("503 backend error while listing objects"),
        })
        .await;
    let partial_lines = render_lines(&session, 190, 24);
    let partial_joined = partial_lines.join("\n");
    assert!(
        partial_joined.contains("503 backend error"),
        "an append-page failure must keep the loaded rows and show the bespoke \
         partial_page_error banner:\n{partial_joined}"
    );
    assert!(
        partial_joined.contains("access.log"),
        "already-loaded rows must survive a partial-page failure:\n{partial_joined}"
    );
    write_evidence(
        "05_browser_partial_page_error.txt",
        "Next-page load failure (n) inside prod-assets: first page stays on screen and the bespoke \
         partial_page_error banner reports the failed continuation — channel intentionally not \
         folded into the generic collection.",
        &partial_lines,
    );

    // --- 7. Object-detail failure stays on its bespoke channel -------------
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectNext))
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::StorageObjectDetailLoadFailed {
            context,
            object_key: "access.log".to_string(),
            error: AppError::gcloud("PERMISSION_DENIED: storage.objects.get access denied"),
        })
        .await;
    let object_error_lines = render_lines(&session, 190, 30);
    let object_error_joined = object_error_lines.join("\n");
    assert!(
        object_error_joined.contains("access.log")
            && object_error_joined.contains("PERMISSION_DENIED"),
        "object-detail failure must render through the bespoke object_detail_error channel:\n{object_error_joined}"
    );
    write_evidence(
        "06_object_detail_error.txt",
        "Object-detail load failure for access.log: object pane keeps its bespoke error channel \
         but shares the merged StorageDetailTab tab strip with the bucket pane.",
        &object_error_lines,
    );
}
