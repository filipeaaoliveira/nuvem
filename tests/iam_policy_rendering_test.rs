//! The project's IAM policy reaches the screen.
//!
//! A service can be in `SERVICE_REGISTRY`, satisfy every `ResourceServiceKind`
//! walk, hold rows in state, and still never paint one — every one of those
//! tests passes and the tab is blank. So these assertions are made on the
//! terminal buffer, through the real reducer and the real renderer, not on the
//! projection model.
//!
//! The service is list-only: one row per (role, member) pair, no detail pane
//! and no lifecycle action. What is asserted here is the table — that it
//! paints, that the identity column shows the member, that a role is carried
//! verbatim, and — the assertions this file exists for — that a public grant
//! and a grant to a deleted principal wear their badges on their own rows
//! without those badges costing the reader the member they mark or the role
//! beside it.

use nuvem::app::projection::columns::catalog_meta;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{IamPolicyMember, ResourceServiceKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// A terminal an operator plausibly runs: the table shares the frame with the
/// navigator and the preview, so this is where the column plan actually has to
/// make a choice about which column gets the slack.
const FRAME_WIDTH: u16 = 200;
const FRAME_HEIGHT: u16 = 40;

/// Wide enough that every column reaches its target width, so an assertion
/// about *what* a cell contains is not really an assertion about elision.
const WIDE_FRAME_WIDTH: u16 = 320;

fn configuration() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

/// Five bindings: an ordinary one, a public one, a deleted principal, one that
/// is both, and a conditional grant. All but the first are rows this project's
/// live policy cannot produce, which is exactly why they are fixtures.
///
/// `deleted:allUsers` is a member the wire permits and `from_wire` reads as
/// both facts at once — the row whose cell has to compose two badges rather
/// than choose between them.
fn bindings() -> Vec<IamPolicyMember> {
    vec![
        IamPolicyMember::from_wire(
            "roles/storage.admin",
            "serviceAccount:sa-vault@dev-project.iam.gserviceaccount.com",
            None,
        ),
        IamPolicyMember::from_wire("roles/storage.objectViewer", "allUsers", None),
        IamPolicyMember::from_wire(
            "roles/viewer",
            "deleted:serviceAccount:old@dev-project.iam.gserviceaccount.com?uid=1",
            None,
        ),
        IamPolicyMember::from_wire("roles/editor", "deleted:allUsers", None),
        IamPolicyMember::from_wire(
            "projects/dev-project/roles/custom.auditor",
            "user:auditor@example.com",
            Some("expires-2027"),
        ),
    ]
}

fn app_with_context() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration()],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::IamPolicy,
        )),
    );
    app.root.workspace = Workspace::Resources;
    app
}

fn app_showing_bindings() -> AppState {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::IamPolicyLoaded {
            context,
            members: bindings(),
        }),
    );
    app
}

/// The five fixture bindings with `filter` applied through the real filter
/// actions.
fn app_with_filter(filter: &str) -> AppState {
    let mut app = app_showing_bindings();
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for character in filter.chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(character)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));
    app
}

/// The rendered frame as one string, joined by lines.
fn render(app: &AppState, width: u16) -> String {
    let session = RuntimeSession::new_for_test(app.clone());
    let backend = TestBackend::new(width, FRAME_HEIGHT);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            let mut line = String::new();
            for x in 0..buffer.area.width {
                line.push_str(buffer.cell((x, y)).expect("cell").symbol());
            }
            line.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The rows of the table pane alone.
///
/// The navigator, the table and the preview are painted side by side, so one
/// buffer line carries a slice of all three — and the preview shows the
/// selected binding's own facts. A `contains` over the whole line therefore
/// passes on a table that paints nothing at all, which is the failure these
/// tests exist to catch.
fn table_lines(frame: &str) -> Vec<&str> {
    frame
        .lines()
        .filter_map(|line| line.split("\u{2502}\u{2502}").nth(1))
        .collect()
}

fn table_text(frame: &str) -> String {
    table_lines(frame).join("\n")
}

/// The single rendered *table* row containing `needle`, so a per-row assertion
/// cannot be satisfied by a match in the navigator or the preview pane.
///
/// Panics when two rows match, because this returns one row and a silent first
/// match is how "every row wears the badge" passes a per-row assertion.
fn row_containing<'f>(frame: &'f str, needle: &str) -> &'f str {
    let matches: Vec<&str> = table_lines(frame)
        .into_iter()
        .filter(|line| line.contains(needle))
        .collect();
    match matches.as_slice() {
        [row] => row,
        [] => panic!("no rendered table row contains {needle:?} in:\n{frame}"),
        rows => panic!(
            "{} rendered table rows contain {needle:?}, so a per-row assertion \
             would be answered by whichever came first:\n{frame}",
            rows.len()
        ),
    }
}

/// How many table rows carry `needle`.
fn rows_containing(frame: &str, needle: &str) -> usize {
    table_lines(frame)
        .into_iter()
        .filter(|line| line.contains(needle))
        .count()
}

#[test]
fn the_identity_column_is_keyed_name_and_shows_the_member() {
    // `IDENTITY_COLUMN` is the literal "name", and `config::columns` rejects a
    // configured list that lacks it, so the key must be "name" even though the
    // header reads MEMBER and the value is the member.
    let keys: Vec<&str> = catalog_meta(ResourceServiceKind::IamPolicy)
        .iter()
        .map(|meta| meta.key)
        .collect();
    assert_eq!(keys, vec!["name", "role", "kind", "condition"]);

    let table = table_text(&render(&app_showing_bindings(), WIDE_FRAME_WIDTH));
    assert!(table.contains("MEMBER"), "no MEMBER header:\n{table}");
    // The type prefix and the local part, not the whole address: a member
    // longer than the column's preferred width elides its tail with an
    // ellipsis, which is correct behaviour and useless to assert a full
    // address against.
    assert!(
        table.contains("serviceAccount:sa-vault@dev-project"),
        "the identity column does not show the member:\n{table}"
    );
}

#[test]
fn a_role_renders_verbatim_so_a_custom_role_is_distinguishable() {
    // The custom role must paint its `projects/<project>/roles/<name>`
    // spelling from the left. Shortened to `custom.auditor` it would be
    // indistinguishable from a predefined role of the same trailing name —
    // two different grants, one string. (The last character is the column's
    // ellipsis, not the projection's doing: the role is carried verbatim and
    // it is the *cell* that runs out of room.)
    let table = table_text(&render(&app_showing_bindings(), WIDE_FRAME_WIDTH));

    assert!(
        table.contains("projects/dev-project/roles/custom.audit"),
        "the custom role was shortened:\n{table}"
    );
    assert!(
        table.contains("roles/storage.admin"),
        "the predefined role is missing:\n{table}"
    );
}

#[test]
fn the_role_still_reads_as_a_custom_role_on_a_real_terminal() {
    // `plan_table` grows priority 0 to its whole target before priority 1 gets
    // any slack, so every character MEMBER's preferred width gains is taken
    // from ROLE at the widths an operator actually runs. At MEMBER 76 — enough
    // to hold `DELETED ` plus a fixture member in full — the role column here
    // collapses toward its 6-cell minimum and an audit loses the half of the
    // row that says *what* was granted. This is the test that fails when
    // someone widens the identity column to make a wide-frame assertion pass.
    let frame = render(&app_showing_bindings(), FRAME_WIDTH);
    let table = table_text(&frame);

    assert!(
        table.contains("projects/dev-project/roles/"),
        "at a realistic width the role column no longer distinguishes a \
         custom role from a predefined one:\n{table}"
    );
    assert!(
        table.contains("roles/storage.admin"),
        "at a realistic width a predefined role no longer paints in full:\n{table}"
    );
    // And the identity column still carries a badge plus the start of the
    // member it marks, rather than being squeezed to the badge alone.
    assert!(
        row_containing(&frame, "old@").contains("DELETED deleted:serviceAccount:old@"),
        "at a realistic width the badged member is not readable:\n{table}"
    );
}

#[test]
fn an_unconditional_binding_paints_a_dash_not_none() {
    // Four of the five fixtures are unconditional, and `Cell::optional` is how
    // every other table in this codebase paints an absent value. Rust's `None`
    // reaching a cell is what this catches; that the absent value paints as a
    // *dash* rather than as a blank is the next test.
    let table = table_text(&render(&app_showing_bindings(), WIDE_FRAME_WIDTH));

    assert!(
        !table.contains("None"),
        "a raw Option reached a cell:\n{table}"
    );
    assert!(
        table.contains("expires-2027"),
        "the one conditional grant does not show its title:\n{table}"
    );
}

#[test]
fn the_condition_cell_of_an_unconditional_binding_is_a_dash() {
    // CONDITION is the rightmost column, so the dash is the last thing a
    // trimmed row carries. An empty cell passes a `!contains("None")` check
    // and still tells the operator nothing.
    let frame = render(&app_showing_bindings(), WIDE_FRAME_WIDTH);

    let unconditional = row_containing(&frame, "serviceAccount:sa-vault@dev-project").trim_end();
    assert!(
        unconditional.ends_with('-'),
        "an unconditional binding must paint a dash in its condition cell, \
         not leave it blank: {unconditional:?}"
    );

    let conditional = row_containing(&frame, "user:auditor@example.com").trim_end();
    assert!(
        conditional.ends_with("expires-2027"),
        "the conditional grant must paint its title in the same cell: \
         {conditional:?}"
    );
}

#[test]
fn a_public_binding_carries_a_badge() {
    // allUsers is the finding a policy audit exists to surface, and a badge is
    // how this codebase already says "this row is the one to look at" (the
    // ORPHAN precedent on persistent disks).
    let frame = render(&app_showing_bindings(), WIDE_FRAME_WIDTH);
    let row = row_containing(&frame, "PUBLIC allUsers");

    assert!(
        row.contains("roles/storage.objectViewer"),
        "the public badge is not on the public grant's own row:\n{row}"
    );
}

#[test]
fn a_deleted_principal_carries_a_badge() {
    let frame = render(&app_showing_bindings(), WIDE_FRAME_WIDTH);
    let row = row_containing(&frame, "old@dev-project");

    assert!(
        row.contains("DELETED"),
        "the stale binding carries no badge:\n{row}"
    );
}

#[test]
fn a_binding_that_is_both_public_and_deleted_wears_both_badges() {
    // `deleted:allUsers` is a member the wire permits, and it is the worst row
    // in a policy: an internet-wide grant that also names a principal nobody
    // is maintaining. A cell that showed one badge would hide whichever fact
    // it dropped, and the row would still look badged.
    let frame = render(&app_showing_bindings(), WIDE_FRAME_WIDTH);
    let row = row_containing(&frame, "deleted:allUsers");

    // One assertion on the composed text: it pins that both badges are
    // present, that they are separated from each other and from the member by
    // a space, and the order they compose in.
    assert!(
        row.contains("PUBLIC DELETED deleted:allUsers"),
        "a binding that is both public and deleted must wear both badges, \
         spaced, ahead of its member:\n{row}"
    );
}

#[test]
fn a_badge_is_worn_only_by_the_row_whose_fact_it_states() {
    // The two badges are separate facts about separate rows. A renderer that
    // badged every row, or that read `deleted` where it meant `is_public`,
    // would satisfy the per-row tests above and tell an auditor that a live
    // service account is open to the internet.
    let frame = render(&app_showing_bindings(), WIDE_FRAME_WIDTH);

    // Two of the five fixtures are public and two name a deleted principal,
    // and they overlap in exactly one row.
    let public_rows = rows_containing(&frame, "PUBLIC");
    assert_eq!(
        public_rows, 2,
        "two fixtures are public; PUBLIC appears on {public_rows} rows:\n{frame}"
    );
    let deleted_rows = rows_containing(&frame, "DELETED");
    assert_eq!(
        deleted_rows, 2,
        "two fixtures name a deleted principal; DELETED appears on \
         {deleted_rows} rows:\n{frame}"
    );

    // The two rows that are neither wear neither: an ordinary service account
    // grant and a conditional user grant.
    for member in [
        "serviceAccount:sa-vault@dev-project",
        "user:auditor@example.com",
    ] {
        let row = row_containing(&frame, member);
        assert!(
            !row.contains("PUBLIC") && !row.contains("DELETED"),
            "{member} is neither public nor deleted and must wear no badge:\n{row}"
        );
    }
}

#[test]
fn a_badge_does_not_displace_the_member_it_marks() {
    // The badge annotates the identity cell. One that overwrote the member
    // would cost the reader the only column that tells rows apart — and the
    // badged rows are precisely the ones an auditor came to read.
    let frame = render(&app_showing_bindings(), WIDE_FRAME_WIDTH);

    assert!(
        row_containing(&frame, "PUBLIC allUsers").contains("allUsers"),
        "the PUBLIC badge displaced its member:\n{frame}"
    );
    assert!(
        row_containing(&frame, "old@dev-project")
            .contains("deleted:serviceAccount:old@dev-project"),
        "the DELETED badge displaced its member:\n{frame}"
    );
}

#[test]
fn every_binding_paints_its_own_row() {
    // Five fixtures, five rows: the pair-per-row shape is the whole reason
    // this table exists, and a renderer that grouped by role would show four.
    let frame = render(&app_showing_bindings(), WIDE_FRAME_WIDTH);

    for member in [
        "serviceAccount:sa-vault@dev-project",
        "PUBLIC allUsers",
        "old@dev-project",
        "deleted:allUsers",
        "user:auditor@example.com",
    ] {
        row_containing(&frame, member);
    }
    assert_eq!(
        table_lines(&frame)
            .into_iter()
            .filter(|line| line.contains("roles/") || line.contains("projects/"))
            .count(),
        5,
        "five bindings must paint five rows:\n{frame}"
    );
}

#[test]
fn a_badge_an_operator_can_read_is_a_badge_they_can_filter_by() {
    // The `ORPHAN` rule: the cell text and the searchable field are one
    // string, so `/PUBLIC` isolates exactly the rows wearing the badge. A
    // badge that is only paintable would leave an auditor scrolling 188 rows
    // looking for the one the table already knows how to find.
    //
    // PUBLIC and not DELETED, because no fixture field spells "public" while
    // every deleted member's `raw_member` is prefixed `deleted:` and the
    // filter is case-insensitive — `/DELETED` finds those rows whether or not
    // the badge is searchable, so asserting it would assert nothing.
    let table = table_text(&render(&app_with_filter("PUBLIC"), WIDE_FRAME_WIDTH));

    assert!(
        table.contains("PUBLIC allUsers"),
        "filtering by the badge must keep the row wearing it:\n{table}"
    );
    assert!(
        !table.contains("sa-vault@dev-project"),
        "filtering by the badge must hide the rows that do not wear it:\n{table}"
    );
}

// --- The preview ---

/// The preview pane's policy arm, asserted on the projection.
///
/// Every assertion above goes through `table_lines`, which splits the preview
/// off the buffer line and throws it away — so this harness structurally
/// cannot see the preview, and the arm could return nothing at all while the
/// whole file stayed green. The pane going blank on every binding is exactly
/// the kind of failure a table test cannot notice.
///
/// The member titles the pane and the three lines are the binding's own facts:
/// the mechanically transformed arm read the service accounts' fields here.
#[test]
fn the_preview_shows_the_selected_bindings_own_facts() {
    let app = app_showing_bindings();
    // The default sort is Member ascending, so the selected first row is
    // `allUsers`, not the fixture's first element.
    let preview = nuvem::app::projection::project_resource_preview(&app.root.resources)
        .expect("a selected binding previews");

    assert_eq!(
        preview.title, "allUsers",
        "the preview titles the selected binding by its member"
    );
    assert_eq!(
        preview.lines,
        vec![
            ("Role".to_string(), "roles/storage.objectViewer".to_string()),
            ("Type".to_string(), "All users".to_string()),
            ("Condition".to_string(), "-".to_string()),
        ],
        "the preview lines carry the binding's role, type and condition — \
         an unconditional grant reads `-`, not a blank"
    );
}

#[test]
fn the_preview_names_the_condition_that_narrows_a_grant() {
    // The one fixture binding a live policy here cannot produce. A preview
    // that dropped the title would overstate what the principal can do, on
    // the pane an operator opens precisely to check that.
    let app = app_with_filter("expires-2027");
    let preview = nuvem::app::projection::project_resource_preview(&app.root.resources)
        .expect("the conditional binding previews");

    assert_eq!(preview.title, "user:auditor@example.com");
    assert_eq!(
        preview.lines,
        vec![
            (
                "Role".to_string(),
                "projects/dev-project/roles/custom.auditor".to_string()
            ),
            ("Type".to_string(), "User".to_string()),
            ("Condition".to_string(), "expires-2027".to_string()),
        ],
        "the condition title must reach the preview, or a narrowed grant \
         reads as an unconditional one"
    );
}

// --- The empty states ---

/// The whole frame, not the table pane: the empty states render as a `List`,
/// not a table, so `table_lines` — which splits on the table's own borders —
/// does not see them at all.
fn painted(app: AppState) -> String {
    render(&app, FRAME_WIDTH)
}

/// The three status-selected empty branches of `render_iam_policy_list`.
///
/// Every rendering assertion above paints a populated table, so each of these
/// status comparisons could be inverted with the file still green. A wrong
/// branch tells an operator the fetch is loading when it has failed, or paints
/// a bare list when the rows are merely stale — and on this table "no rows"
/// reads as "nobody has access to this project".
#[test]
fn the_policy_list_paints_a_distinct_empty_state_per_status() {
    // Loading: a fetch in flight against a collection with no rows yet.
    let mut loading = app_with_context();
    loading
        .root
        .resources
        .begin_load(ResourceServiceKind::IamPolicy, std::time::Instant::now());
    let frame = painted(loading);
    assert!(
        frame.contains("Loading IAM policy"),
        "an in-flight policy fetch must paint the loading state, naming what \
         it is loading:\n{frame}"
    );
    assert!(
        !frame.contains("No policy bindings found for"),
        "a policy still loading must never claim the project has no \
         bindings:\n{frame}"
    );

    // Failed: the load-failed result for the active context.
    let mut failed = app_with_context();
    let context = failed.root.current_context().expect("active context");
    reduce(
        &mut failed,
        &Action::EffectResult(EffectResult::IamPolicyLoadFailed {
            context,
            error: nuvem::app::AppError::gcloud("permission denied"),
        }),
    );
    let frame = painted(failed);
    assert!(
        frame.contains("Press r to retry."),
        "a failed policy fetch must paint the retry affordance, which is what \
         distinguishes it from an empty or stale list:\n{frame}"
    );
    assert!(
        !frame.contains("No policy bindings found for"),
        "a policy nobody could read must not be reported as a policy granting \
         nothing:\n{frame}"
    );

    // Stale: rows cleared pending a background refresh, with no load phase.
    let mut stale = app_showing_bindings();
    stale
        .root
        .resources
        .mark_active_stale_and_clear(ResourceServiceKind::IamPolicy);
    let frame = painted(stale);
    assert!(
        frame.contains("Resource rows are stale. Press r to refresh."),
        "stale policy rows must say they are stale, not read as a project \
         whose policy binds nobody:\n{frame}"
    );
}

/// The three *non*-status empty branches: a project whose policy is genuinely
/// empty, a list nobody has fetched, and a filter that matches nothing. They
/// read as opposite facts — "this project grants nothing", "nothing has been
/// fetched yet", and "your filter hid them all" — and the third is the one an
/// audit must never confuse with the first.
#[test]
fn the_policy_list_distinguishes_no_bindings_from_not_loaded_from_filtered_out() {
    // Ready with zero rows: the fetch completed and returned no bindings.
    let mut empty = app_with_context();
    let context = empty.root.current_context().expect("active context");
    reduce(
        &mut empty,
        &Action::EffectResult(EffectResult::IamPolicyLoaded {
            context,
            members: Vec::new(),
        }),
    );
    let frame = painted(empty);
    assert!(
        frame.contains("No policy bindings found for"),
        "a completed fetch that returned nothing must say the configuration \
         has no bindings:\n{frame}"
    );
    assert!(
        !frame.contains("Press r to load the IAM policy."),
        "a completed fetch must not invite a load it has already done:\n{frame}"
    );
    assert!(
        !frame.contains("No policy bindings match filter"),
        "an empty policy is not a hidden-by-filter list:\n{frame}"
    );

    // Never loaded: the service is selected but no fetch has completed.
    let frame = painted(app_with_context());
    assert!(
        frame.contains("Press r to load the IAM policy."),
        "a list nobody has fetched must offer the load, not report a project \
         that grants nothing:\n{frame}"
    );
    assert!(
        !frame.contains("No policy bindings found for"),
        "nothing is known about the project's bindings before a fetch:\n{frame}"
    );

    // Filtered out: rows exist, none of them survive the applied filter.
    let frame = painted(app_with_filter("zzz-matches-nothing"));
    assert!(
        frame.contains("No policy bindings match filter \"zzz-matches-nothing\""),
        "a filter that hides every row must name itself, so the operator knows \
         the bindings are hidden rather than absent:\n{frame}"
    );
    assert!(
        !frame.contains("No policy bindings found for"),
        "bindings that exist behind a filter must not be reported as \
         absent — that is an access review reading a filtered view as \
         evidence:\n{frame}"
    );
}

/// The list-tools header on the empty-state path: the line that tells an
/// operator staring at "no matches" what filter is applied and how many rows
/// it hid.
#[test]
fn a_filtered_empty_policy_list_keeps_its_filter_and_row_count_header() {
    let frame = painted(app_with_filter("zzz-matches-nothing"));

    assert!(
        frame.contains("Filter: zzz-matches-nothing"),
        "the empty-state list must still carry the applied filter in its \
         tools header:\n{frame}"
    );
    assert!(
        frame.contains("Rows: 0/5"),
        "the tools header must report how many of the loaded bindings \
         survived the filter, which is what distinguishes a hiding filter \
         from a project that grants nothing:\n{frame}"
    );
}
