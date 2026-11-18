//! IAM service layer: the argv nuvem issues for each of the two calls, and what
//! it makes of the JSON that comes back.
//!
//! The policy assertions carry most of the weight. The wire shape is one
//! binding per role holding a `members` array, and the rows this table renders
//! are (role, member) pairs — so the count, the order, and which role each row
//! carries are all produced by the flattening rather than read off the
//! document. A parser that dropped the last member of every binding, or that
//! stamped the wrong role onto a row, would still return a plausible,
//! fully-populated table.

use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::resources::iam::{GcloudIamService, IamLoadError};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{IamMemberKind, IamPolicyMember, ServiceAccount};
use std::cell::RefCell;

fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".into()),
        region: None,
        zone: None,
    }
}

/// Answers every invocation with one canned payload and records the argv it was
/// asked to run, so a test can assert on both halves.
struct RecordingRunner {
    stdout: String,
    seen: RefCell<Vec<Vec<String>>>,
}

impl RecordingRunner {
    fn new(stdout: &str) -> Self {
        Self {
            stdout: stdout.to_string(),
            seen: RefCell::new(Vec::new()),
        }
    }

    fn argv(&self) -> Vec<String> {
        self.seen
            .borrow()
            .last()
            .cloned()
            .expect("a command was run")
    }
}

impl CommandRunner for RecordingRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.seen.borrow_mut().push(command.args.clone());
        Ok(CommandOutput {
            stdout: self.stdout.clone(),
            stderr: String::new(),
        })
    }
}

/// Refuses to run anything, so a test can prove a load never reached the shell.
struct RefusingRunner;

impl CommandRunner for RefusingRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        panic!("no command should have run, but got: {:?}", command.args);
    }
}

fn accounts() -> Vec<ServiceAccount> {
    let runner = RecordingRunner::new(include_str!("fixtures/iam_service_accounts.json"));
    GcloudIamService::new(runner)
        .load_service_accounts(&ctx(), None)
        .expect("the fixture parses")
}

fn members() -> Vec<IamPolicyMember> {
    let runner = RecordingRunner::new(include_str!("fixtures/iam_policy.json"));
    GcloudIamService::new(runner)
        .load_policy_members(&ctx())
        .expect("the fixture parses")
}

fn account_by_email<'a>(accounts: &'a [ServiceAccount], email: &str) -> &'a ServiceAccount {
    accounts
        .iter()
        .find(|account| account.email == email)
        .unwrap_or_else(|| panic!("no service account {email:?} in {accounts:?}"))
}

// ---------------------------------------------------------------- accounts

/// The four fields the table renders, off the wire. `disabled` is the one that
/// is not a string: it distinguishes an account that cannot authenticate from
/// one that can, and it still holds every role either way, so it is not
/// derivable from the policy table and nothing else on the row implies it.
#[test]
fn a_service_account_row_carries_its_email_name_disabled_state_and_unique_id() {
    let accounts = accounts();
    let vault = account_by_email(&accounts, "sa-vault@dev-project.iam.gserviceaccount.com");
    assert_eq!(
        vault.display_name.as_deref(),
        Some("SA used by the vault node pool")
    );
    assert!(!vault.disabled);
    assert_eq!(vault.unique_id, "110727432291788759739");

    let retired = account_by_email(
        &accounts,
        "old-deployer@dev-project.iam.gserviceaccount.com",
    );
    assert!(
        retired.disabled,
        "a disabled account must not read as enabled — it is the one column \
         saying this identity cannot authenticate"
    );
    assert_eq!(retired.unique_id, "104405669981102690742");
}

/// `disabled` is a proto3 boolean, so proto3 JSON omits it when false: the
/// absence of the key *is* the value `false`. A row that dropped or failed on
/// the missing key would either lose the account or mark a working identity
/// disabled.
#[test]
fn an_absent_disabled_key_reads_as_enabled() {
    let accounts = accounts();
    let no_key = account_by_email(
        &accounts,
        "no-display-name@dev-project.iam.gserviceaccount.com",
    );
    assert!(!no_key.disabled);
}

/// An absent `displayName` and an empty one are the same fact — the account has
/// no name — and both must read as `None`. A `Some("")` paints a blank cell
/// while every `is_some()` downstream believes there is a name to show.
#[test]
fn an_absent_and_an_empty_display_name_are_both_no_name() {
    let accounts = accounts();
    assert_eq!(
        account_by_email(
            &accounts,
            "no-display-name@dev-project.iam.gserviceaccount.com"
        )
        .display_name,
        None
    );
    assert_eq!(
        account_by_email(
            &accounts,
            "empty-display-name@dev-project.iam.gserviceaccount.com"
        )
        .display_name,
        None,
        "an empty displayName is no display name, not a name that is empty"
    );
}

/// A row missing every field parses into blanks rather than failing the
/// document. An empty table is a claim that the project has no service
/// accounts, and that must not be how one unreadable row renders.
#[test]
fn a_row_missing_every_field_still_parses() {
    let accounts = accounts();
    assert_eq!(accounts.len(), 5, "every fixture row is listed");
    let blank = account_by_email(&accounts, "");
    assert_eq!(blank.display_name, None);
    assert!(!blank.disabled);
    assert_eq!(blank.unique_id, "");
}

/// The subcommand and the flags that bound the answer.
#[test]
fn the_accounts_argv_names_the_configuration_project_and_bounded_json() {
    let service = GcloudIamService::new(RecordingRunner::new("[]"));
    service
        .load_service_accounts(&ctx(), None)
        .expect("empty list parses");
    let argv = service.runner().argv();

    assert_eq!(
        argv,
        vec![
            "--configuration",
            "dev",
            "--project",
            "dev-project",
            "iam",
            "service-accounts",
            "list",
            "--format=json",
            "--limit=500",
        ]
    );
}

/// Nothing is ever pushed, in any filter mode. The service's searchable values
/// include the rendered `Disabled` state, which is in no raw field, so a
/// pushed expression could narrow away a row the client filter would keep.
#[test]
fn no_filter_mode_reaches_the_server_for_service_accounts() {
    for filter in [None, Some("vault"), Some("-l env=prod"), Some("~vau.t")] {
        let runner = RecordingRunner::new("[]");
        let service = GcloudIamService::new(runner);
        service
            .load_service_accounts(&ctx(), filter)
            .expect("empty list parses");
        let argv = service.runner().argv();
        assert!(
            !argv.iter().any(|arg| arg.starts_with("--filter")),
            "filter {filter:?} was pushed down as {argv:?}"
        );
    }
}

// ------------------------------------------------------------------ policy

/// The flattening, end to end. Eight fixture bindings hold eleven members
/// between them, and the binding with an empty `members` array contributes no
/// row — the pair is the unit this table renders, and a role nobody holds is
/// not a pair.
#[test]
fn every_binding_member_becomes_one_row_and_an_empty_binding_becomes_none() {
    let members = members();
    assert_eq!(
        members.len(),
        12,
        "one row per (role, member) pair, and none for the memberless binding: {members:#?}"
    );
    assert!(
        !members
            .iter()
            .any(|row| row.role == "roles/orphanedRoleWithNoMembers"),
        "a role with no members contributes no row"
    );
}

/// Each row carries the role of the binding it came from. A binding holding
/// three members produces three rows naming that same role — get the pairing
/// wrong and the table still fills, with every grant attributed to the wrong
/// principal.
#[test]
fn each_row_carries_the_role_of_its_own_binding() {
    let members = members();
    let admins = members
        .iter()
        .filter(|row| row.role == "roles/artifactregistry.admin")
        .map(|row| row.principal.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        admins,
        vec![
            "sa-gke@dev-project.iam.gserviceaccount.com",
            "alice@example.com",
            "platform@example.com",
        ],
        "all three members of the one binding, in wire order"
    );

    // The custom role shares its trailing segment with the predefined one
    // above, and must not have been folded into it.
    let custom = members
        .iter()
        .filter(|row| row.role == "projects/dev-project/roles/artifactregistry.admin")
        .collect::<Vec<_>>();
    assert_eq!(custom.len(), 2, "the custom role keeps its own rows");
    assert_eq!(custom[0].kind, IamMemberKind::PrincipalSet);

    // A federated subject is an arbitrary IdP string and may contain colons of
    // its own, so only the first one separates the kind. Splitting from the
    // back would truncate this identity to `erin@example.com` — a different,
    // and real, principal.
    assert_eq!(custom[1].kind, IamMemberKind::Principal);
    assert_eq!(
        custom[1].principal,
        "//iam.googleapis.com/locations/global/workforcePools/wf/subject/user:erin@example.com"
    );
}

/// The public bindings, off the wire. These are the two rows a policy audit
/// exists to find, and both are prefixless — read by a `split(':')` they become
/// rows with an empty principal.
#[test]
fn the_unbounded_population_rows_are_flagged_and_keep_their_identity() {
    let members = members();
    let public = members
        .iter()
        .filter(|row| row.is_public())
        .map(|row| (row.role.as_str(), row.principal.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        public,
        vec![
            ("roles/storage.objectViewer", "allUsers"),
            ("roles/viewer", "allAuthenticatedUsers"),
        ]
    );
    assert!(
        members
            .iter()
            .filter(|row| !row.is_public())
            .all(|row| !row.principal.is_empty()),
        "no row may have an empty principal"
    );
}

/// A deleted principal keeps its kind, its `?uid=` disambiguator, and its
/// marker. The uid is the only thing telling two deleted accounts at the same
/// address apart, and the marker is what makes a stale grant findable.
#[test]
fn a_deleted_principal_keeps_its_kind_uid_and_marker() {
    let members = members();
    let deleted = members.iter().filter(|row| row.deleted).collect::<Vec<_>>();
    assert_eq!(deleted.len(), 2);
    assert_eq!(deleted[0].kind, IamMemberKind::ServiceAccount);
    assert_eq!(
        deleted[0].principal,
        "gone@dev-project.iam.gserviceaccount.com?uid=104405669981102690742"
    );
    assert_eq!(deleted[1].kind, IamMemberKind::User);
    assert_eq!(deleted[1].principal, "bob@example.com?uid=9");
    assert!(
        members
            .iter()
            .filter(|row| !row.deleted)
            .all(|row| !row.raw_member.starts_with("deleted:")),
        "no undeleted row may carry the marker"
    );
}

/// A conditional binding is a *narrower* grant, so the condition reaches the
/// row; every other row must read as unconditional. A parser that stamped the
/// last binding's condition onto every row, or dropped it, would overstate or
/// understate what a principal can do.
#[test]
fn only_the_conditional_binding_carries_a_condition() {
    let members = members();
    let conditional = members
        .iter()
        .filter(|row| row.condition.is_some())
        .collect::<Vec<_>>();
    assert_eq!(conditional.len(), 1);
    assert_eq!(conditional[0].role, "roles/compute.admin");
    assert_eq!(conditional[0].principal, "carol@example.com");
    assert_eq!(conditional[0].condition.as_deref(), Some("expires-2027"));
}

/// A binding with no `role` key still lists its members rather than failing the
/// document, for the same reason a service account row missing every field
/// parses: an empty policy table is a claim that nothing is granted.
#[test]
fn a_binding_missing_its_role_still_lists_its_members() {
    let members = members();
    let roleless = members
        .iter()
        .filter(|row| row.role.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(roleless.len(), 1);
    assert_eq!(roleless[0].principal, "dave@example.com");
}

/// A policy document with no `bindings` key at all is an empty table, not a
/// parse failure: gcloud omits empty arrays.
#[test]
fn a_policy_with_no_bindings_key_is_an_empty_table() {
    let runner = RecordingRunner::new(r#"{"etag":"abc","version":1}"#);
    let members = GcloudIamService::new(runner)
        .load_policy_members(&ctx())
        .expect("a bindings-less policy parses");
    assert!(members.is_empty());
}

/// The project is a *positional* argument to `get-iam-policy`, not the
/// `--project` flag, and the `--limit` every other list carries is absent: the
/// answer is one policy object, so a limit would cap policies rather than
/// bindings.
#[test]
fn the_policy_argv_names_the_project_positionally_and_passes_no_limit() {
    let runner = RecordingRunner::new(r#"{"bindings":[]}"#);
    let service = GcloudIamService::new(runner);
    service
        .load_policy_members(&ctx())
        .expect("empty policy parses");
    let argv = service.runner().argv();

    assert_eq!(
        argv,
        vec![
            "--configuration",
            "dev",
            "--project",
            "dev-project",
            "projects",
            "get-iam-policy",
            "dev-project",
            "--format=json",
        ]
    );
    assert!(
        !argv.iter().any(|arg| arg.starts_with("--limit")),
        "a limit would cap the number of policies, not the bindings inside one"
    );
}

/// With no project there is no positional argument to pass, so the load refuses
/// rather than shelling out to a command that cannot succeed.
#[test]
fn a_context_with_no_project_never_runs_the_policy_command() {
    let context = GcloudContext {
        project: None,
        ..ctx()
    };
    let error = GcloudIamService::new(RefusingRunner)
        .load_policy_members(&context)
        .expect_err("no project means no call");
    assert!(matches!(error, IamLoadError::MissingProject), "{error:?}");
}

/// Malformed JSON fails the load rather than returning an empty table, and each
/// call says which document it could not read.
#[test]
fn each_load_reports_its_own_parse_failure() {
    let accounts = GcloudIamService::new(RecordingRunner::new("not json"))
        .load_service_accounts(&ctx(), None)
        .expect_err("garbage does not parse");
    assert!(
        accounts.to_string().contains("service accounts"),
        "{accounts}"
    );

    let policy = GcloudIamService::new(RecordingRunner::new("not json"))
        .load_policy_members(&ctx())
        .expect_err("garbage does not parse");
    assert!(policy.to_string().contains("IAM policy"), "{policy}");
}
