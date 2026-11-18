//! Evidence: the plugin confirmation carries what runs, and nothing to
//! re-resolve it from.
//!
//! The argv is composed at palette-submit time, against the row that was
//! selected then, and travels whole into the confirmation. That is the whole
//! promise the modal makes: the operator approved *these* elements, and these
//! are the elements the child receives. A `PluginId` riding along beside them
//! is not merely unused — it is the one field a future reader could use to look
//! the plugin up again at confirm time, which would let the command that runs
//! differ from the command that was read.
//!
//! Two tests, failing differently:
//!
//! 1. [`the_submission_hands_back_only_the_label_and_the_argv`] pins the shape
//!    at the type level. It is the one that fails the moment an identity is put
//!    back on the submission, whatever it is called, because the annotation is
//!    what the compiler checks.
//! 2. [`replacing_the_plugin_table_under_an_open_confirmation_cannot_change_what_runs`]
//!    pins the consequence. It swaps the whole plugin table out from under an
//!    open confirmation — the exact hazard `RootState::plugins` documents as the
//!    reason a reload does not replace it — and asserts the child still receives
//!    the previewed argv. A confirm arm that re-resolved by index would run the
//!    substitute's `delete` here.
//!
//! Neither subsumes the other. The type test cannot tell whether a re-resolved
//! plugin would actually reach the child; the behavioural test would still pass
//! against a dead field nobody reads yet.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::KeyCode;
use nuvem::app::state::resources::ResourcesState;
use nuvem::config::plugins::Plugins;
use nuvem::config::problems::ConfigProblems;
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use tempfile::NamedTempFile;

use crate::plugin_preview_evidence::Operator;

/// A plugin table whose one entry sits at the same index as the fixture's
/// `describe` and runs something an operator would never confirm by accident.
/// `delete` is chosen so the failure mode is legible in the assertion message:
/// if this argv ever reaches the runner, the confirm arm resolved a plugin
/// rather than running the one it was handed.
/// How long a confirmed run's result may take to reach the queue. Generous: it
/// is a deadline that turns a hang into a message, not a timing assertion.
const RESULT_TIMEOUT: Duration = Duration::from_secs(30);

const SUBSTITUTE_PLUGIN_FILE: &str = "version = 1\n\n\
     [[plugins]]\n\
     id       = \"substitute\"\n\
     scope    = \"compute\"\n\
     key      = \"d\"\n\
     label    = \"Delete {name}\"\n\
     args     = [\"compute\", \"instances\", \"delete\", \"{name}\", \"--zone\", \"{zone}\"]\n";

#[test]
fn the_submission_hands_back_only_the_label_and_the_argv() {
    let mut state = ResourcesState::default();
    state.show_plugin_confirmation(
        "Describe web-01".to_string(),
        vec![
            "--configuration".to_string(),
            "dev".to_string(),
            "compute".to_string(),
            "instances".to_string(),
            "describe".to_string(),
            "web-01".to_string(),
        ],
        false,
    );

    // The annotation is the assertion. Widening this tuple to carry an identity
    // again is a compile error here, which is the only place it can be caught
    // before a reader has already written the code that resolves from it.
    let submission: Option<(String, Vec<String>)> = state.plugin_confirmation_submission();

    let (label, argv) = submission.expect("the confirmation was open");
    assert_eq!(label, "Describe web-01");
    assert_eq!(
        argv.last().map(String::as_str),
        Some("web-01"),
        "the argv travels whole, element for element"
    );
}

#[tokio::test]
async fn replacing_the_plugin_table_under_an_open_confirmation_cannot_change_what_runs() {
    let runner = RecordingRunner::default();
    let mut operator =
        Operator::at_the_compute_list_with_runner(false, Box::new(runner.clone())).await;

    operator.press(KeyCode::Char(':')).await;
    operator.type_text("describe").await;
    operator.press(KeyCode::Enter).await;

    let previewed = operator
        .session
        .app()
        .root
        .resources
        .plugin_confirmation()
        .expect("the palette left a confirmation to answer")
        .argv()
        .to_vec();

    // The hazard, made real: the table the offer came from is gone, and the
    // index the offer was resolved at now names a different command entirely.
    // Nothing in the app does this today — reload deliberately does not replace
    // `plugins` — which is why it is done here by hand rather than waited for.
    // The temp file is bound rather than discarded: it is the guard that keeps
    // the fixture on disk for as long as anything might read it back.
    let (substitute, _substitute_file) = substitute_plugins();
    operator.session.app_mut().root.plugins = substitute;

    operator.press(KeyCode::Enter).await;

    // Bounded on both axes, and the deadline is the load-bearing half. The
    // session holds the sender, so the channel never closes and `recv()` never
    // returns `false`: a regression that seeded an entry without dispatching the
    // effect that settles it would park here forever, and the count guard below
    // would never be reached to report it. A hung test says nothing; this one
    // names what did not arrive.
    let mut drained = 0;
    while operator
        .session
        .app()
        .root
        .system
        .operations
        .pending_count()
        > 0
    {
        assert!(
            drained < 8,
            "the plugin's entry was still pending after {drained} results"
        );
        let applied = tokio::time::timeout(
            RESULT_TIMEOUT,
            operator.session.process_next_effect_result(),
        )
        .await
        .unwrap_or_else(|_| {
            panic!("no effect result arrived within {RESULT_TIMEOUT:?} with the plugin's entry still pending")
        });
        assert!(
            applied,
            "the result channel closed with the plugin's entry still pending"
        );
        drained += 1;
    }

    // Counted rather than compared against the whole call list: confirming a
    // plugin also refreshes the table it may have mutated, so the
    // runner legitimately sees a `compute instances list` alongside the plugin's
    // own command. The claim here is about the plugin's call, and both halves
    // are needed — the count alone would be satisfied by the substitute running
    // *as well*, and the absence alone by nothing running at all.
    let calls = runner.argv.lock().unwrap().clone();
    assert_eq!(
        calls.iter().filter(|argv| **argv == previewed).count(),
        1,
        "the child must receive the argv the modal held, exactly once. Calls \
         were: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|argv| argv.iter().any(|element| element == "delete")),
        "the substitute plugin's command must never reach the child, so nothing \
         re-resolved the plugin at confirm time: {calls:?}"
    );
}

fn substitute_plugins() -> (Plugins, NamedTempFile) {
    let mut file = NamedTempFile::new().expect("a temp plugin file");
    file.write_all(SUBSTITUTE_PLUGIN_FILE.as_bytes())
        .expect("the fixture writes");
    let mut problems = ConfigProblems::default();
    let plugins = Plugins::load(file.path(), &mut problems);
    assert!(
        problems.is_empty(),
        "the substitute fixture must load cleanly: {:?}",
        problems.messages()
    );
    assert_eq!(plugins.len(), 1, "the substitute plugin must load");
    (plugins, file)
}

/// Records the argv of every command it is handed. Answers with empty stdout:
/// this file asserts on what crossed the process boundary, not on parsing.
#[derive(Clone, Default)]
struct RecordingRunner {
    argv: Arc<Mutex<Vec<Vec<String>>>>,
}

impl CommandRunner for RecordingRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.argv.lock().unwrap().push(command.args.clone());
        Ok(CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}
