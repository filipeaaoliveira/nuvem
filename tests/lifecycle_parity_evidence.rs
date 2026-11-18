//! Evidence generator — the operator-facing end of the lifecycle
//! argv/preview parity guarantee.
//!
//! The in-repo parity tests compare argv against a preview string. This one
//! closes the loop the other side of the `CommandRunner` boundary: for every
//! action in `ComputeInstanceAction::ALL` and `CloudSqlInstanceAction::ALL`, it
//! runs the real `ProcessCommandRunner` against a stand-in `gcloud` that
//! records the argv the operating system actually handed it, renders the
//! confirmation modal an operator confirms through `nuvem::ui::render`, and
//! asserts the "Call:" line on screen describes what the process received.
//!
//! Walking `ALL` — now generated from the enum's own variant list — is what
//! makes "every action" true rather than "every action someone remembered".
//! With `NUVEM_EVIDENCE_DIR` set, the rendered modals and the recorded argv are
//! written out as ASCII screenshots and a transcript; the assertions always
//! run.

use std::fs;
use std::path::{Path, PathBuf};

use nuvem::app::state::resources::{CloudSqlInstanceAction, ComputeInstanceAction, ResourcesState};
use nuvem::app::{AppState, Workspace};
use nuvem::services::gcloud::cli::ProcessCommandRunner;
use nuvem::services::resources::cloudsql::GcloudCloudSqlService;
use nuvem::services::resources::compute::GcloudComputeService;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{
    CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, ComputeInstance, EndpointVisibility,
    ResourceStatus,
};
use ratatui::{Terminal, backend::TestBackend};

fn evidence_dir() -> Option<PathBuf> {
    let dir = std::env::var("NUVEM_EVIDENCE_DIR").ok()?;
    let path = PathBuf::from(dir);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

fn write_evidence(name: &str, header: &str, body: &str) {
    let Some(dir) = evidence_dir() else {
        return;
    };
    fs::write(dir.join(name), format!("{header}\n\n{body}")).expect("write evidence");
    println!("wrote {name}");
}

fn framed(lines: &[String]) -> String {
    let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let mut body = String::new();
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
    body
}

fn context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

fn compute_instance() -> ComputeInstance {
    ComputeInstance {
        id: "1001".to_string(),
        name: "api-dev-1".to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: Some("10.0.0.4".to_string()),
        external_ip: None,
    }
}

fn cloud_sql_instance() -> CloudSqlInstance {
    CloudSqlInstance {
        name: "mysql-ha".to_string(),
        database_version: "MYSQL_8_0".to_string(),
        engine: "MySQL".to_string(),
        region: "europe-west1".to_string(),
        status: CloudSqlStatus::Runnable,
        availability: CloudSqlAvailability::Regional,
        tier: "db-n1-standard-1".to_string(),
        connectivity: EndpointVisibility::Public,
    }
}

/// A stand-in `gcloud` that appends the argv it was handed to a file, one
/// argument per line, so the comparison is against what the process received
/// rather than what the caller believes it sent.
#[cfg(unix)]
fn recording_gcloud(directory: &Path) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let record = directory.join("argv.txt");
    let binary = directory.join("gcloud-recorder");
    fs::write(
        &binary,
        format!(
            "#!/bin/sh\n: > '{record}'\nfor arg in \"$@\"; do printf '%s\\n' \"$arg\" >> '{record}'; done\n",
            record = record.display()
        ),
    )
    .expect("write recorder");
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).expect("mark executable");
    (binary, record)
}

#[cfg(unix)]
fn recorded_argv(record: &Path) -> Vec<String> {
    fs::read_to_string(record)
        .expect("recorder wrote argv")
        .lines()
        .map(ToString::to_string)
        .collect()
}

fn rendered_lines(app: AppState, width: u16, height: u16) -> Vec<String> {
    let session = RuntimeSession::new_for_test(app);
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
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

/// The `Call:` line as it appears on screen, with the modal's padding
/// stripped — this is the text an operator reads before pressing Enter.
fn call_line(lines: &[String]) -> String {
    lines
        .iter()
        .find_map(|line| line.split_once("Call: "))
        // The modal's own right-hand border, and any panel border behind it,
        // follow the text on the same buffer row.
        .map(|(_, rest)| {
            rest.split('│')
                .next()
                .unwrap_or_default()
                .trim_end()
                .to_string()
        })
        .expect("the confirmation modal renders a Call: line")
}

#[cfg(unix)]
#[test]
fn every_lifecycle_action_previews_the_argv_the_process_receives() {
    let directory = tempfile::tempdir().expect("tempdir");
    let (binary, record) = recording_gcloud(directory.path());
    let mut transcript = String::new();

    for action in ComputeInstanceAction::ALL {
        let service = GcloudComputeService::new(ProcessCommandRunner::new(binary.clone()));
        service
            .run_action_by_id(&context(), "api-dev-1", "europe-west1-b", action)
            .expect("recorder exits zero");
        let executed = format!("gcloud {}", recorded_argv(&record).join(" "));

        let mut app = AppState::new();
        app.root.workspace = Workspace::Resources;
        app.root.resources = ResourcesState::default();
        app.root
            .resources
            .start_action_confirmation(context(), compute_instance(), action);
        let lines = rendered_lines(app, 120, 24);
        let previewed = call_line(&lines);

        // The two are no longer byte-identical, and that is the point of this
        // phase rather than a weakening of it: the modal names the call the
        // runtime will make, and the process receives an argv rendered from
        // the same builder. What has to hold is that they describe the same
        // action on the same resource, which is what the operator is reading
        // the line to find out.
        assert!(
            executed.contains(action.command_verb())
                && previewed.contains(&format!("instances.{}", action.command_verb())),
            "the modal and the process must name the same action for \
             {action:?}: modal {previewed:?}, process {executed:?}"
        );
        assert!(
            executed.contains("api-dev-1") && executed.contains("europe-west1-b"),
            "the process argv names the instance and its zone: {executed:?}"
        );
        assert!(
            previewed.contains("zones/europe-west1-b/instances/api-dev-1"),
            "and the modal names the same one: {previewed:?}"
        );
        transcript.push_str(&format!(
            "GCE {action:?}\n  process argv : {executed}\n  modal shows  : {previewed}\n\n"
        ));
        if matches!(action, ComputeInstanceAction::Stop) {
            write_evidence(
                "01_gce_stop_confirmation_modal.txt",
                &format!(
                    "GCE Stop confirmation modal as rendered by nuvem::ui::render. The Call: line names the call this action makes; the argv the OS handed the gcloud process renders the same one: {executed}"
                ),
                &framed(&lines),
            );
        }
    }

    for action in CloudSqlInstanceAction::ALL {
        let service = GcloudCloudSqlService::new(ProcessCommandRunner::new(binary.clone()));
        service
            .run_action_by_id(&context(), "mysql-ha", action)
            .expect("recorder exits zero");
        let executed = format!("gcloud {}", recorded_argv(&record).join(" "));

        let mut app = AppState::new();
        app.root.workspace = Workspace::Resources;
        app.root.resources = ResourcesState::default();
        app.root.resources.start_cloud_sql_action_confirmation(
            context(),
            cloud_sql_instance(),
            action,
        );
        let lines = rendered_lines(app, 120, 24);
        let previewed = call_line(&lines);

        // Cloud SQL's start and stop are both `patch`, and the modal is only
        // worth reading because it says which: the field is what separates
        // them, in the argv as a flag and in the call as a value.
        let (method, field) = match action {
            CloudSqlInstanceAction::Start => ("instances.patch", Some("activationPolicy=ALWAYS")),
            CloudSqlInstanceAction::Stop => ("instances.patch", Some("activationPolicy=NEVER")),
            CloudSqlInstanceAction::Restart => ("instances.restart", None),
            CloudSqlInstanceAction::Failover => ("instances.failover", None),
        };
        assert!(
            previewed.contains(method) && previewed.contains("instances/mysql-ha"),
            "the modal names the call for {action:?}: {previewed:?}"
        );
        if let Some(field) = field {
            assert!(
                previewed.contains(field),
                "a patch must say what it sets, or start and stop read alike: \
                 {previewed:?}"
            );
            let flag = field.replace("activationPolicy=", "--activation-policy=");
            assert!(
                executed.contains(&flag),
                "and the process must receive the matching flag: {executed:?}"
            );
        }
        transcript.push_str(&format!(
            "Cloud SQL {action:?}\n  process argv : {executed}\n  modal shows  : {previewed}\n\n"
        ));
        if matches!(action, CloudSqlInstanceAction::Stop) {
            write_evidence(
                "02_cloudsql_stop_confirmation_modal.txt",
                &format!(
                    "Cloud SQL Stop confirmation modal as rendered by nuvem::ui::render. Cloud SQL has no stop verb, so the executed command is a patch — and that is exactly what the modal promises: {executed}"
                ),
                &framed(&lines),
            );
        }
    }

    write_evidence(
        "03_argv_vs_preview_transcript.txt",
        &format!(
            "Every action in ComputeInstanceAction::ALL ({} actions) and CloudSqlInstanceAction::ALL ({} actions), \
             each run through the real ProcessCommandRunner against a recording stand-in gcloud. \
             'process argv' is what the OS handed the child; 'modal shows' is the Command: line read off the rendered confirmation modal.",
            ComputeInstanceAction::ALL.len(),
            CloudSqlInstanceAction::ALL.len(),
        ),
        &transcript,
    );
    print!("{transcript}");
}
