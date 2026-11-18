//! Persistent disks service layer: the argv nuvem issues, and what it makes of
//! the JSON that comes back.
//!
//! The URL-tail assertions carry most of the weight here. `zone`, `type` and
//! every entry of `users` come back as fully-qualified `googleapis.com` URLs,
//! and every one of them is a table cell — a parser that passed them through
//! would still populate every row, so nothing but an assertion on the *value*
//! catches it.

use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::resources::disks::GcloudDisksService;
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{DiskStatus, PersistentDisk};
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

fn loaded() -> Vec<PersistentDisk> {
    let runner = RecordingRunner::new(include_str!("fixtures/persistent_disks.json"));
    GcloudDisksService::new(runner)
        .load_disks(&ctx(), None)
        .expect("the fixture parses")
}

fn by_name<'a>(disks: &'a [PersistentDisk], name: &str) -> &'a PersistentDisk {
    disks
        .iter()
        .find(|disk| disk.name == name)
        .unwrap_or_else(|| panic!("the fixture holds a disk named {name}"))
}

/// Every URL-shaped field is reduced to its trailing segment, and `sizeGb` —
/// which the API returns as a JSON *string*, not a number — becomes an integer
/// the size column can right-align and the sort can order numerically.
#[test]
fn the_list_parses_urls_down_to_their_trailing_segment() {
    let disks = loaded();

    assert_eq!(disks.len(), 6, "the fixture holds six disks");

    let attached = by_name(&disks, "gke-node-a");
    assert_eq!(attached.location, "europe-west1-b");
    assert_eq!(attached.disk_type, "pd-balanced");
    assert_eq!(attached.size_gb, 100);
    assert_eq!(attached.status, DiskStatus::Ready);
    assert_eq!(
        attached.users,
        vec!["gke-node-a".to_string()],
        "a user is an instance name, not 100 characters of resource URL"
    );
    assert_eq!(
        attached
            .labels
            .iter()
            .map(|label| format!("{}={}", label.key, label.value))
            .collect::<Vec<_>>(),
        vec!["env=prod".to_string(), "team=platform".to_string()],
        "labels are sorted, so a reload does not reshuffle them under the cursor"
    );
}

/// A regional disk carries `region` where a zonal one carries `zone`.
/// `compute disks list` returns both kinds in one array, so a parser that only
/// read `zone` would locate every regional disk nowhere while still listing it.
#[test]
fn a_regional_disk_is_located_by_its_region() {
    let disks = loaded();
    let regional = by_name(&disks, "regional-share");

    assert_eq!(regional.location, "europe-west1");
    assert_eq!(
        regional.users,
        vec!["db-primary".to_string(), "db-standby".to_string()],
        "a regional disk can be attached to more than one instance"
    );
}

/// The badge, end to end from the wire. `detached-legacy` has no `users` key at
/// all — gcloud omits empty arrays — and that absence is what makes it an
/// orphan. `being-built` and `broken-disk` are equally unattached and must not
/// be badged: the first is mid-operation, the second is the knowingly
/// conservative gap recorded on `PersistentDisk::is_orphan`.
#[test]
fn only_the_unattached_ready_disk_is_badged_an_orphan() {
    let disks = loaded();

    assert!(by_name(&disks, "detached-legacy").is_orphan());
    assert!(!by_name(&disks, "gke-node-a").is_orphan());
    assert!(!by_name(&disks, "being-built").is_orphan());
    assert!(!by_name(&disks, "broken-disk").is_orphan());
    assert!(!by_name(&disks, "regional-share").is_orphan());
}

/// A row missing every optional field parses into a blank rather than failing
/// the document. An empty table is a claim that the project has no disks, and
/// that must not be how one unreadable row renders.
///
/// The blank must not be badged: its status is unreadable, so "nothing will
/// start using this on its own" is not a claim we can make about it.
#[test]
fn a_row_missing_every_optional_field_still_parses_and_is_not_badged() {
    let disks = loaded();
    let bare = by_name(&disks, "bare-row");

    assert_eq!(bare.location, "");
    assert_eq!(bare.disk_type, "");
    assert_eq!(bare.size_gb, 0);
    assert!(bare.users.is_empty());
    assert_eq!(bare.status, DiskStatus::Unrecognized(String::new()));
    assert!(
        !bare.is_orphan(),
        "a disk whose status could not be read must not be badged for deletion"
    );
}

/// Label mode is the only mode that reaches the server, and it is what makes
/// `-l k=v` narrow at all: `CompiledFilter::Label` keeps every row client-side,
/// so without the pushdown the filter would silently match everything.
#[test]
fn a_label_filter_is_pushed_down_and_a_regex_filter_is_not() {
    let runner = RecordingRunner::new("[]");
    let service = GcloudDisksService::new(runner);

    service
        .load_disks(&ctx(), Some("-l env=prod"))
        .expect("an empty list parses");
    let pushed = service.runner().argv();
    assert!(
        pushed.iter().any(|arg| arg.starts_with("--filter=")
            && arg.contains("labels.env")
            && arg.contains("prod")),
        "a label filter must reach the server: {pushed:?}"
    );

    service
        .load_disks(&ctx(), Some("web"))
        .expect("an empty list parses");
    let unpushed = service.runner().argv();
    assert!(
        !unpushed.iter().any(|arg| arg.starts_with("--filter=")),
        "a regex filter narrows client-side; pushing it could drop a row the \
         table would have matched: {unpushed:?}"
    );
}

/// The subcommand and the flags that bound the answer.
#[test]
fn the_argv_names_the_configuration_project_and_bounded_json() {
    let runner = RecordingRunner::new("[]");
    let service = GcloudDisksService::new(runner);
    service.load_disks(&ctx(), None).expect("an empty list");
    let argv = service.runner().argv();

    assert!(argv.windows(2).any(|pair| pair == ["compute", "disks"]));
    assert!(argv.contains(&"list".to_string()));
    assert!(argv.contains(&"--format=json".to_string()));
    assert!(
        argv.iter().any(|arg| arg.starts_with("--limit=")),
        "an unbounded list can hang the runtime on a large project: {argv:?}"
    );
    assert!(argv.contains(&"--configuration".to_string()));
    assert!(argv.contains(&"dev-project".to_string()));
}
