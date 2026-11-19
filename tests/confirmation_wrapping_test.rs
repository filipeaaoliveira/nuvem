//! A destructive-action confirmation is available on an ordinary terminal.
//!
//! `paragraph_fits` required every line to fit the modal's inner width on one
//! line, and the modal is a fixed 80% x 60% of the frame. The height was never
//! the problem — ten short lines against room for twelve at 80x24 — but the
//! command line is 90 columns:
//!
//! ```text
//! Call: compute.instances.stop zones/europe-west1-b/instances/api-dev-1
//! ```
//!
//! so below a frame of about 115 columns the modal refused to draw and the
//! operator could neither confirm nor see what they would have confirmed. The
//! action was simply unavailable.
//!
//! The guard itself is right and stays: a command the operator cannot see must
//! not be confirmable. What changes is the trade it was making — wrapping keeps
//! the whole command visible *and* lets the action happen.

use nuvem::app::{Action, AppState, CommandAction, EffectResult, Workspace};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceServiceKind, ResourceStatus};
use ratatui::{Terminal, backend::TestBackend};

/// The same fixture, stopping before the palette is driven, so a caller can
/// submit a different command.
async fn session_with_pending_stop_free() -> RuntimeSession {
    let mut session = session_with_instances().await;
    session
        .dispatch_action(Action::Command(CommandAction::Cancel))
        .await;
    session
}

async fn session_with_pending_stop() -> RuntimeSession {
    let mut session = session_with_instances().await;
    session
        .dispatch_action(Action::Command(CommandAction::Open))
        .await;
    for ch in "stop".chars() {
        session
            .dispatch_action(Action::Command(CommandAction::AppendChar(ch)))
            .await;
    }
    session
        .dispatch_action(Action::Command(CommandAction::Submit))
        .await;
    session
}

async fn session_with_instances() -> RuntimeSession {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Resources;
    app.root
        .resources
        .select_service(ResourceServiceKind::ComputeInstances);
    let mut session = RuntimeSession::new_for_test(app);
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            }],
        })
        .await;
    let context = session.app().root.current_context().expect("a context");
    session.app_mut().root.resources.set_instances(
        context,
        vec![ComputeInstance {
            id: "1".into(),
            name: "api-dev-1".into(),
            zone: "europe-west1-b".into(),
            status: ResourceStatus::Running,
            machine_type: "e2-medium".into(),
            internal_ip: Some("10.0.0.4".into()),
            external_ip: None,
        }],
    );
    session
}

fn lines(session: &RuntimeSession, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer.cell((x, y)).unwrap().symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

/// The modal's own text, with whitespace normalised.
///
/// Sliced to the modal's columns rather than read off whole screen rows: the
/// modal is drawn over the sidebar and the table, so a row of the frame carries
/// their text on either side of it and a naive reassembly glues a service name
/// onto the front of a wrapped command.
///
/// Normalised rather than concatenated verbatim: `wrap_exact` keeps the space it
/// breaks after on the first row, and the terminal then pads that row out to the
/// pane's width — so the break space and the padding are the same cells and no
/// reader of the buffer can tell them apart. What the operator can read is every
/// token, in order, which is what this compares.
fn modal_text(rendered: &[String]) -> String {
    let header = rendered
        .iter()
        .find(|line| line.contains("┌confirm GCE action"))
        .unwrap_or_else(|| panic!("no modal on screen: {rendered:#?}"));
    let byte = header.find("┌confirm").expect("the modal's left border");
    let left = header[..byte].chars().count();
    let right = left
        + header
            .chars()
            .skip(left)
            .position(|character| character == '┐')
            .expect("the modal's right border");

    rendered
        .iter()
        .map(|line| {
            line.chars()
                .skip(left + 1)
                .take(right.saturating_sub(left + 1))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The whole call, as the modal renders it.
///
/// It was a gcloud command line until the preview started naming the call the
/// runtime will make. The property is unchanged and is the reason this file
/// exists: whatever the modal puts on that line has to be readable in full at
/// every width that draws, because an operator confirming a destructive
/// action from a half-shown line is the failure this guards.
const WHOLE_CALL: &str = "compute.instances.stop zones/europe-west1-b/instances/api-dev-1";

#[tokio::test]
async fn the_confirmation_renders_on_an_eighty_column_terminal() {
    let session = session_with_pending_stop().await;
    let rendered = lines(&session, 80, 24);
    let text = rendered.join("\n");

    assert!(
        !text.contains("needs a larger terminal"),
        "the confirmation refused to draw at 80x24; rendered:\n{text}"
    );
    assert!(
        text.contains("Confirm GCE action"),
        "the confirmation is not on screen; rendered:\n{text}"
    );
}

/// The safety property: the whole command is readable, wrapped or not.
#[tokio::test]
async fn the_whole_command_is_visible_at_every_width_that_draws_the_modal() {
    let session = session_with_pending_stop().await;
    for width in [80u16, 100, 132, 180] {
        let rendered = lines(&session, width, 24);
        let text = rendered.join("\n");
        assert!(
            !text.contains("needs a larger terminal"),
            "the confirmation refused to draw at {width}x24"
        );
        assert!(
            modal_text(&rendered).contains(WHOLE_CALL),
            "the command is not readable in full at {width}x24; rendered:\n{text}"
        );
        assert!(
            !text.contains("instances/api-de..."),
            "the call was elided rather than wrapped at {width}x24"
        );
    }
}

/// The guard still fires where the content genuinely cannot fit, so the safety
/// property survives the change.
///
/// 60x12 is the smallest terminal nuvem draws on at all — below it the whole app
/// shows its own "needs at least 60x12" notice and the modal guard is never
/// reached, so a smaller size would test the wrong guard.
#[tokio::test]
async fn a_terminal_too_small_for_the_command_still_refuses() {
    let session = session_with_pending_stop().await;
    let text = lines(&session, 60, 12).join("\n");
    assert!(
        text.contains("needs a larger terminal"),
        "a terminal with no room for the command must still refuse; \
         rendered:\n{text}"
    );
}

/// The k9s launch confirmation draws, and says which cluster it will open.
///
/// The other half of the pair below, and the same gap: no rendered-frame test
/// existed for it either, and the mutation gate found it by surviving `replace
/// render_k9s_launch_confirmation with ()`.
///
/// These two are the app's *launch* confirmations — the modals that precede an
/// interactive session on live infrastructure rather than a one-shot command.
/// Every other modal in `ui::mod` had rendered-frame coverage; these two, the
/// ones where drawing nothing would let an operator confirm a session they never
/// saw, did not.
#[tokio::test]
async fn the_k9s_launch_confirmation_shows_which_cluster_it_opens() {
    use nuvem_domain::gcp::k9s::{CredentialStep, K9sLaunchTarget, kube_context_name};

    let mut session = session_with_instances().await;
    let target = K9sLaunchTarget {
        configuration: GcloudConfigurationName::new("dev"),
        project: "dev-project".to_string(),
        location: "europe-west1".to_string(),
        cluster_name: "platform-prod".to_string(),
        kube_context: kube_context_name("dev-project", "europe-west1", "platform-prod"),
    };
    session
        .app_mut()
        .root
        .resources
        .open_k9s_launch_confirmation(target, CredentialStep::RunGetCredentials);

    // With a get-credentials step this modal is fourteen lines. Its 60% box
    // holds them from about twenty-seven rows; below that it takes the whole
    // body, which holds them at 80x24.
    for (width, height) in [(80u16, 24u16), (132, 30), (160, 40)] {
        let text = lines(&session, width, height).join("\n");
        assert!(
            text.contains("Open in k9s"),
            "the confirmation must be on screen at {width}x{height}; rendered:\n{text}"
        );
        assert!(
            text.contains("Cluster: platform-prod"),
            "it must name the cluster it will open at {width}x{height}; \
             rendered:\n{text}"
        );
        assert!(
            text.contains("get-credentials modifies your local kubeconfig"),
            "it must say that launching will change local state at \
             {width}x{height}; rendered:\n{text}"
        );
    }

    // And it still refuses where its lines genuinely do not fit. Wrapping moved
    // that boundary down, and so did taking the body; neither removed it, and
    // this modal is one where the guard earns its keep — the operator is about
    // to be handed a read-write Kubernetes UI. The refusal names a size that
    // holds it and the key that leaves.
    let cramped = lines(&session, 60, 12).join("\n");
    assert!(
        cramped.contains("needs a larger terminal")
            && cramped.contains("or more")
            && cramped.contains("This one is 60x12")
            && cramped.contains("Esc closes it"),
        "fourteen lines cannot be shown in six rows, and the modal must say \
         so rather than showing some of them; rendered:\n{cramped}"
    );
}

/// The SSH launch preview draws, and says what it will run.
///
/// It had no rendered-frame test at all: `ssh_launch_preview_lines` is covered
/// as a function, but nothing asserted the modal reaches the screen. The
/// mutation gate found the gap on CI — `replace render_ssh_launch_preview with
/// ()` survived — where a local run had scored it caught, which is its own
/// reminder that a single green CAUGHT is not evidence.
///
/// It matters more than an ordinary preview: this modal is the last thing
/// between `Enter` and an interactive session on a live instance, so a renderer
/// that drew nothing would leave the operator confirming a launch they never
/// saw.
#[tokio::test]
async fn the_ssh_launch_preview_shows_what_it_will_run() {
    let mut session = session_with_pending_stop_free().await;
    session
        .dispatch_action(Action::Command(CommandAction::Open))
        .await;
    for ch in "SSH to".chars() {
        session
            .dispatch_action(Action::Command(CommandAction::AppendChar(ch)))
            .await;
    }
    session
        .dispatch_action(Action::Command(CommandAction::Submit))
        .await;

    assert!(
        session.app().root.resources.ssh_launch_preview().is_some(),
        "the palette entry must open the launch preview, or this asserts nothing"
    );

    for width in [80u16, 132] {
        let rendered = lines(&session, width, 24);
        let text = rendered.join("\n");
        assert!(
            text.contains("Launch SSH"),
            "the launch preview must be on screen at {width}x24; rendered:\n{text}"
        );
        assert!(
            text.contains("Instance: api-dev-1"),
            "it must name the instance it will connect to at {width}x24; \
             rendered:\n{text}"
        );
        assert!(
            text.contains("Executable:"),
            "it must name the binary it will run at {width}x24; rendered:\n{text}"
        );
    }
}
