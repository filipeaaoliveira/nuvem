//! The `IamPolicy` table's identity, ordering and filtering.
//!
//! The conditional-binding cases are the ones live data cannot produce: this
//! project's policy has no conditional bindings, so every assertion about a
//! condition here rests on a synthetic fixture and nothing else.

use nuvem::app::projection::SortDirection;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::resources::{
    IamPolicy, IamPolicySortField, ResourceService, ResourcesState,
};
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction};
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::filter::{CompiledFilter, compile_filter};
use nuvem_domain::gcp::resource::{IamPolicyMember, ResourceServiceKind};

/// A binding, built the way the loader builds one.
/// `from_wire(role: &str, raw_member: &str, condition: Option<&str>)`.
fn member(role: &str, raw: &str, condition: Option<&str>) -> IamPolicyMember {
    IamPolicyMember::from_wire(role, raw, condition)
}

/// Compiles a needle the way the `visible_*` accessors do.
fn compiled(input: &str) -> CompiledFilter {
    compile_filter(input).expect("the fixture needle must compile")
}

#[test]
fn two_bindings_differing_only_by_condition_are_different_rows() {
    // The case the handover assumed away: a (role, member) pair is NOT unique.
    // Both rows are real, distinct grants — one narrower than the other — and a
    // mark on one must not toggle the other. No live policy in this project has
    // a conditional binding, so nothing but this test holds the line.
    let unconditional = member("roles/storage.admin", "user:a@b.c", None);
    let conditional = member("roles/storage.admin", "user:a@b.c", Some("expires-2027"));

    assert_ne!(
        IamPolicy::row_id(&unconditional),
        IamPolicy::row_id(&conditional),
        "a conditional grant collides with the unconditional one"
    );
}

#[test]
fn the_same_principal_under_two_roles_is_two_rows() {
    let admin = member("roles/storage.admin", "user:a@b.c", None);
    let viewer = member("roles/storage.objectViewer", "user:a@b.c", None);

    assert_ne!(IamPolicy::row_id(&admin), IamPolicy::row_id(&viewer));
}

#[test]
fn a_custom_role_is_not_the_same_row_as_the_predefined_role_it_shadows() {
    // `roles/storage.admin` and `projects/p/roles/storage.admin` are different
    // grants — one Google defines, one the project does. An id that shortened
    // either to its trailing segment would merge them.
    let predefined = member("roles/storage.admin", "user:a@b.c", None);
    let custom = member("projects/p/roles/storage.admin", "user:a@b.c", None);

    assert_ne!(IamPolicy::row_id(&predefined), IamPolicy::row_id(&custom));
}

#[test]
fn row_identity_survives_a_refresh_that_reorders_the_policy() {
    // The API makes no ordering promise across calls. An id derived from
    // position rather than content would move a mark to a different grant.
    let row = member("roles/storage.admin", "user:a@b.c", Some("expires-2027"));
    let same_row_next_refresh = member("roles/storage.admin", "user:a@b.c", Some("expires-2027"));

    assert_eq!(
        IamPolicy::row_id(&row),
        IamPolicy::row_id(&same_row_next_refresh)
    );
}

#[test]
fn the_default_sort_groups_every_grant_one_principal_holds() {
    // Member-ascending is the decision: it answers "what can this principal
    // do". Asserted as adjacency rather than as an exact vector, so the test
    // states the property the decision was about.
    let mut rows = [
        member("roles/viewer", "user:b@x.c", None),
        member("roles/editor", "user:a@x.c", None),
        member("roles/viewer", "user:a@x.c", None),
    ];
    rows.sort_by(|left, right| {
        IamPolicy::compare(
            left,
            right,
            IamPolicySortField::Member,
            SortDirection::Ascending,
        )
    });

    let members: Vec<&str> = rows.iter().map(|row| row.raw_member.as_str()).collect();
    assert_eq!(
        members,
        vec!["user:a@x.c", "user:a@x.c", "user:b@x.c"],
        "the two grants held by a@x.c are not adjacent"
    );
}

#[test]
fn sorting_is_a_total_order_so_a_refresh_cannot_reshuffle_equal_rows() {
    // Every sort field ties somewhere: many members share a role, many roles
    // share a kind. Without a full tiebreak the order of tied rows is the order
    // the API happened to return them in, and the table reshuffles under the
    // user on every poll.
    let rows = [
        member("roles/viewer", "user:a@x.c", None),
        member("roles/viewer", "user:b@x.c", None),
        member("roles/viewer", "user:a@x.c", Some("expires-2027")),
        // The pair that ties on every field but the role: one principal holding
        // two roles unconditionally is the ordinary shape of a policy, and
        // without a role tiebreak these two rows swap places on every poll.
        member("roles/editor", "user:a@x.c", None),
    ];
    for field in [
        IamPolicySortField::Member,
        IamPolicySortField::Role,
        IamPolicySortField::Kind,
        IamPolicySortField::Condition,
    ] {
        for left in &rows {
            for right in &rows {
                let forward = IamPolicy::compare(left, right, field, SortDirection::Ascending);
                if forward == std::cmp::Ordering::Equal {
                    assert_eq!(
                        IamPolicy::row_id(left),
                        IamPolicy::row_id(right),
                        "two distinct rows compare equal on {field:?}, so their order is undefined"
                    );
                }
            }
        }
    }
}

#[test]
fn sorting_by_role_orders_by_the_role_column_and_not_by_another() {
    // The tiebreak chain gives every sort a total order, so a Role sort that
    // read some other field would still look ordered. The fixture opposes the
    // role order to both the member and the kind order, so only a comparison of
    // the roles themselves produces this result.
    let mut rows = [
        member("roles/aaa.admin", "user:a@x.c", None),
        member("roles/zzz.viewer", "group:b@x.c", None),
    ];
    rows.sort_by(|left, right| {
        IamPolicy::compare(
            left,
            right,
            IamPolicySortField::Role,
            SortDirection::Ascending,
        )
    });

    let roles: Vec<&str> = rows.iter().map(|row| row.role.as_str()).collect();
    assert_eq!(roles, vec!["roles/aaa.admin", "roles/zzz.viewer"]);
}

#[test]
fn sorting_by_kind_orders_by_the_type_label_the_column_paints() {
    // `deleted:` sorts a User row ahead of a Group row by raw member, and the
    // roles put it ahead again, so a Kind sort that fell back to either would
    // put User first. Only comparing the labels puts Group first.
    let mut rows = [
        member("roles/aaa.admin", "deleted:user:a@x.c", None),
        member("roles/zzz.viewer", "group:b@x.c", None),
    ];
    rows.sort_by(|left, right| {
        IamPolicy::compare(
            left,
            right,
            IamPolicySortField::Kind,
            SortDirection::Ascending,
        )
    });

    let kinds: Vec<&str> = rows.iter().map(|row| row.kind.label()).collect();
    assert_eq!(kinds, vec!["Group", "User"]);
}

#[test]
fn unconditional_grants_group_at_one_end_of_the_condition_sort() {
    // The point of sorting a missing condition as the empty string: every
    // unconditional grant lands together at one end. Sorting it as any value
    // ordering after some titles would scatter them through the conditional
    // rows, which is the opposite of what the column is read for.
    //
    // Each row carries a different member, ordered *against* its condition, so
    // the member tiebreak cannot reproduce the expected order on its own: a
    // Condition arm that compared nothing would leave a plain member sort —
    // `a, b, c` — instead of `a, c, b`.
    let mut rows = [
        member("roles/viewer", "user:b@x.c", Some("zzz-window")),
        member("roles/viewer", "user:a@x.c", None),
        member("roles/viewer", "user:c@x.c", Some("aaa-window")),
    ];
    rows.sort_by(|left, right| {
        IamPolicy::compare(
            left,
            right,
            IamPolicySortField::Condition,
            SortDirection::Ascending,
        )
    });

    let conditions: Vec<Option<&str>> = rows.iter().map(|row| row.condition.as_deref()).collect();
    assert_eq!(
        conditions,
        vec![None, Some("aaa-window"), Some("zzz-window")],
        "the unconditional grant did not group at the head of the order"
    );
    let members: Vec<&str> = rows.iter().map(|row| row.raw_member.as_str()).collect();
    assert_eq!(
        members,
        vec!["user:a@x.c", "user:c@x.c", "user:b@x.c"],
        "the condition ordered nothing: this is the member tiebreak alone"
    );
}

#[test]
fn descending_reverses_the_order_so_the_direction_toggle_does_something() {
    // Nothing else in the suite compares policy rows descending: without this,
    // a direction toggle that changed nothing would go unnoticed.
    let fixture = || {
        [
            member("roles/viewer", "user:b@x.c", None),
            member("roles/viewer", "user:c@x.c", None),
            member("roles/viewer", "user:a@x.c", None),
        ]
    };
    let sorted = |direction| {
        let mut rows = fixture();
        rows.sort_by(|left, right| {
            IamPolicy::compare(left, right, IamPolicySortField::Member, direction)
        });
        rows.iter()
            .map(|row| row.raw_member.clone())
            .collect::<Vec<_>>()
    };

    let ascending = sorted(SortDirection::Ascending);
    let mut reversed = ascending.clone();
    reversed.reverse();
    assert_eq!(
        sorted(SortDirection::Descending),
        reversed,
        "descending did not reverse the ascending order"
    );
    assert_ne!(
        sorted(SortDirection::Descending),
        ascending,
        "the direction toggle changed nothing"
    );
}

#[test]
fn filtering_by_a_member_answers_what_that_principal_can_do() {
    let rows = [
        member("roles/storage.admin", "user:a@x.c", None),
        member("roles/viewer", "user:b@x.c", None),
    ];
    let filter = compiled("a@x.c");

    let matched: Vec<&str> = rows
        .iter()
        .filter(|row| IamPolicy::matches_filter(row, &filter))
        .map(|row| row.role.as_str())
        .collect();

    assert_eq!(matched, vec!["roles/storage.admin"]);
}

#[test]
fn filtering_by_a_role_answers_who_holds_it() {
    let rows = [
        member("roles/storage.admin", "user:a@x.c", None),
        member("roles/storage.admin", "group:g@x.c", None),
        member("roles/viewer", "user:b@x.c", None),
    ];
    let filter = compiled("storage.admin");

    let matched = rows
        .iter()
        .filter(|row| IamPolicy::matches_filter(row, &filter))
        .count();

    assert_eq!(matched, 2, "both holders of the role must match");
}

#[test]
fn a_public_binding_is_findable_by_the_name_an_auditor_types() {
    // `allUsers` carries no colon, so it is the member whose whole string is
    // the identity. An auditor types `allUsers`; the filter has to find it.
    let rows = [
        member("roles/storage.objectViewer", "allUsers", None),
        member("roles/viewer", "user:a@x.c", None),
    ];
    let filter = compiled("allUsers");

    let matched: Vec<&str> = rows
        .iter()
        .filter(|row| IamPolicy::matches_filter(row, &filter))
        .map(|row| row.raw_member.as_str())
        .collect();

    assert_eq!(matched, vec!["allUsers"]);
}

#[test]
fn a_deleted_principal_is_findable_by_its_prefix() {
    let rows = [
        member(
            "roles/viewer",
            "deleted:serviceAccount:x@y.iam.gserviceaccount.com?uid=1",
            None,
        ),
        member("roles/viewer", "user:a@x.c", None),
    ];
    let filter = compiled("deleted:");

    let matched = rows
        .iter()
        .filter(|row| IamPolicy::matches_filter(row, &filter))
        .count();

    assert_eq!(
        matched, 1,
        "a stale binding is what an audit is looking for"
    );
}

#[test]
fn filtering_by_a_condition_title_finds_the_narrowed_grant() {
    // The title appears in no other field, so this is the only assertion that
    // the condition is searchable at all — and since no live policy in this
    // project is conditional, nothing else would ever notice it going missing.
    let rows = [
        member("roles/storage.admin", "user:a@x.c", Some("expires-2027")),
        member("roles/storage.admin", "user:b@x.c", None),
    ];
    let filter = compiled("expires-2027");

    let matched: Vec<&str> = rows
        .iter()
        .filter(|row| IamPolicy::matches_filter(row, &filter))
        .map(|row| row.raw_member.as_str())
        .collect();

    assert_eq!(matched, vec!["user:a@x.c"]);
}

#[test]
fn filtering_by_the_type_label_finds_the_rows_that_column_shows() {
    // `Service account` with its space is the painted label, not the
    // `serviceAccount:` wire prefix, so a row only matches through the kind
    // label being searchable.
    let rows = [
        member(
            "roles/viewer",
            "serviceAccount:x@y.iam.gserviceaccount.com",
            None,
        ),
        member("roles/viewer", "user:a@x.c", None),
    ];
    let filter = compiled("Service account");

    let matched: Vec<&str> = rows
        .iter()
        .filter(|row| IamPolicy::matches_filter(row, &filter))
        .map(|row| row.raw_member.as_str())
        .collect();

    assert_eq!(matched, vec!["serviceAccount:x@y.iam.gserviceaccount.com"]);
}

#[test]
fn a_condition_title_cannot_be_confused_with_a_longer_role() {
    // The role and the condition share one id component, so they need a
    // separator no role or title can contain. Concatenating them plainly makes
    // `roles/viewer` + `x` collide with the unconditional `roles/viewerx` —
    // the same identity collision this file exists to prevent, by another
    // route.
    let conditional = member("roles/viewer", "user:a@x.c", Some("x"));
    let longer_role = member("roles/viewerx", "user:a@x.c", None);

    assert_ne!(
        IamPolicy::row_id(&conditional),
        IamPolicy::row_id(&longer_role),
        "a condition title concatenated onto the role collides with a longer role"
    );
}

// --- The state that holds the rows ---

fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".into()),
        region: None,
        zone: None,
    }
}

fn config() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

/// A state on the policy tab holding `rows`, with no filter applied.
fn state_with(rows: Vec<IamPolicyMember>) -> ResourcesState {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::IamPolicy);
    state.set_policy_members(ctx(), rows);
    state
}

fn visible_members(state: &ResourcesState) -> Vec<String> {
    state
        .visible_policy_members()
        .into_iter()
        .map(|row| row.raw_member.clone())
        .collect()
}

fn app_showing_policy() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::IamPolicy,
        )),
    );
    app
}

/// The default sort field the collection is constructed with, which no test
/// that passes a field explicitly can see. Member-ascending is the decision —
/// it is what makes the table answer "what can this principal do" — and
/// installing any other field here silently answers a different question.
///
/// The three rows order differently under every one of the four fields, so no
/// wrong default reproduces this order by coincidence: by role it is
/// c-then-b-then-a, by kind the group leads, by condition the unconditional
/// row leads, and descending reverses the whole thing.
#[test]
fn a_freshly_loaded_policy_opens_sorted_by_member_ascending() {
    let state = state_with(vec![
        member("roles/zzz.viewer", "deleted:user:a@x.c", Some("zzz-window")),
        member("roles/mmm.editor", "group:b@x.c", None),
        member("roles/aaa.admin", "user:c@x.c", Some("aaa-window")),
    ]);

    assert_eq!(
        visible_members(&state),
        vec!["deleted:user:a@x.c", "group:b@x.c", "user:c@x.c"],
        "the policy must open grouped by principal; any other default sort \
         answers a question the operator did not ask"
    );
}

/// `IamPolicySortField::label` and `from_column`, pinned per variant — the
/// mirror of the accounts twin.
///
/// The sort tests above drive the comparator with a field in hand and never
/// spell a column index or a label, so a swapped `from_column` arm sorts by
/// the wrong column the moment the operator moves the cursor and presses `s`,
/// and a wrong label puts the sort arrow's name on a different column than the
/// one doing the sorting.
#[test]
fn every_sortable_column_maps_to_its_field_and_carries_its_own_label() {
    let by_column = [
        (0, IamPolicySortField::Member, "Member"),
        (1, IamPolicySortField::Role, "Role"),
        (2, IamPolicySortField::Kind, "Type"),
        (3, IamPolicySortField::Condition, "Condition"),
    ];

    for (index, field, label) in by_column {
        assert_eq!(
            IamPolicySortField::from_column(index),
            Some(field),
            "column {index} must map to {field:?}; a swapped arm sorts by a \
             column other than the one the arrow sits on"
        );
        assert_eq!(
            field.label(),
            label,
            "{field:?} must carry its own label, or the sort indicator names \
             the wrong column"
        );
        assert_eq!(
            field.column(),
            index,
            "`column` and `from_column` must be inverses"
        );
    }

    // Distinct labels: a constant satisfies every assertion above if it
    // happens to equal one of them, and four columns reading the same tell
    // the operator nothing.
    let labels = by_column
        .iter()
        .map(|(_, field, _)| field.label())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        labels.len(),
        by_column.len(),
        "every sort field needs its own label"
    );

    assert_eq!(
        IamPolicySortField::from_column(4),
        None,
        "the policy catalogue has exactly four columns, every one of them \
         sortable"
    );
    assert_eq!(IamPolicySortField::COLUMN_COUNT, 4);
}

/// `set_policy_members` keeping the cursor on the binding it was already on.
///
/// This is the only production use of `row_id`, and the identity assertions at
/// the head of this file are all made on the pure function — so the state that
/// consumes it could ignore it entirely. The API promises no ordering across
/// calls, and the policy poller refetches on a ticker: an index-based cursor
/// moves to a different grant while the operator is reading it.
///
/// The list is sorted before it is shown, so a response that merely reorders
/// the same bindings cannot move the cursor and asserting against one would
/// assert nothing. What moves it is the visible set changing shape: the
/// fixture's reload grants one new binding that sorts *ahead* of the selected
/// row, which shifts every index below it by one.
///
/// The row left at the old index is the unconditional grant of the same role
/// to the same member — so this pins decision 1 as well: an identity blind to
/// the condition would restore the cursor onto the broader grant, which is the
/// worst of the wrong answers.
#[test]
fn a_reload_keeps_the_cursor_on_the_binding_it_was_already_on() {
    let bindings = || {
        vec![
            member("roles/storage.admin", "user:b@x.c", None),
            member("roles/storage.admin", "user:b@x.c", Some("expires-2027")),
            member("roles/viewer", "user:c@x.c", None),
        ]
    };
    let mut state = state_with(bindings());
    state.select_next();
    let selected = state.selected_policy_member().expect("a selected binding");
    assert_eq!(
        (selected.raw_member.as_str(), selected.condition.as_deref()),
        ("user:b@x.c", Some("expires-2027")),
        "the fixture needs the conditional grant selected"
    );

    // The next poll: one binding newly granted, and the rest returned in a
    // different order — the API promises none.
    let mut refreshed = bindings();
    refreshed.reverse();
    refreshed.push(member("roles/aaa.admin", "user:a@x.c", None));
    state.set_policy_members(ctx(), refreshed);

    let selected = state.selected_policy_member().expect("a selected binding");
    assert_eq!(
        (selected.raw_member.as_str(), selected.condition.as_deref()),
        ("user:b@x.c", Some("expires-2027")),
        "the cursor must follow the binding's identity, not its row index; \
         the row at the old index is the unconditional grant of the same role"
    );
}

#[test]
fn a_reload_that_revokes_the_selected_binding_clamps_the_cursor() {
    let mut state = state_with(vec![
        member("roles/storage.admin", "user:a@x.c", None),
        member("roles/storage.admin", "user:b@x.c", None),
    ]);
    state.select_next();

    state.set_policy_members(
        ctx(),
        vec![member("roles/storage.admin", "user:a@x.c", None)],
    );

    assert_eq!(
        state
            .selected_policy_member()
            .map(|row| row.raw_member.clone()),
        Some("user:a@x.c".to_string()),
        "a cursor past the end of the new list must not dangle"
    );
}

/// `policy_members` is the whole collection — the `total` of the tools line's
/// `n/m` and the Home inventory's count for this service — and
/// `visible_policy_member_count` is the filtered reading beside it. Every
/// assertion above walks `visible_policy_members` instead, so both accessors
/// could report an empty policy against 188 loaded bindings.
///
/// Counts that are neither `0` nor `1` on both readings, so neither constant
/// replacement passes, and a filter that narrows so the two readings cannot be
/// the same accessor twice.
#[test]
fn the_counts_report_the_loaded_rows_and_the_ones_the_filter_leaves() {
    let mut state = state_with(vec![
        member("roles/storage.admin", "user:auditor@x.c", None),
        member("roles/viewer", "user:auditor@x.c", None),
        member("roles/viewer", "group:everyone@x.c", None),
    ]);

    assert_eq!(
        state.policy_members().len(),
        3,
        "the collection must hold every loaded binding, or the row counter \
         and the Home inventory read zero against a loaded policy"
    );
    assert_eq!(
        state.visible_policy_member_count(),
        3,
        "with no filter applied every loaded binding is visible"
    );

    state.start_filter();
    for character in "auditor".chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();

    assert_eq!(
        state.visible_policy_member_count(),
        2,
        "the count must follow the filter, or the row counter contradicts \
         the rows the table paints"
    );
    assert_eq!(
        state.visible_policy_member_count(),
        state.visible_policy_members().len(),
        "the count and the list must never disagree: they are two readings \
         of one filter"
    );
    assert_eq!(
        state.policy_members().len(),
        3,
        "the filter hides rows; it does not unload them"
    );
}

/// `policy_members_problem` is what the home overview reads to show the policy
/// as degraded, and nothing else in the suite reads it — so it could report a
/// healthy service forever, which on this table means an access review reading
/// an empty policy as "nobody has access". Both transitions are pinned.
#[test]
fn a_failed_policy_load_raises_a_problem_and_a_later_success_clears_it() {
    let mut app = app_showing_policy();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::IamPolicyLoadFailed {
            context: context.clone(),
            error: nuvem::app::AppError::gcloud("permission denied"),
        }),
    );

    assert!(
        app.root.resources.policy_members_problem().is_some(),
        "a failed policy load must leave a problem for the overview to show, \
         or a policy nobody could read looks like a policy granting nothing"
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::IamPolicyLoaded {
            context,
            members: vec![member("roles/viewer", "user:a@x.c", None)],
        }),
    );

    assert!(
        app.root.resources.policy_members_problem().is_none(),
        "a successful load must clear the previous problem"
    );
}

/// `principal` in the searchable fields, which no other test can observe: it
/// is always a substring of `raw_member`, so in substring and fuzzy mode it
/// matches exactly the rows `raw_member` already matches. An anchored regex is
/// the one input that can tell them apart — `^a@x.c$` cannot match past the
/// `user:` prefix `raw_member` carries — and it is what an audit types when a
/// lookalike address makes an unanchored search ambiguous.
#[test]
fn an_anchored_pattern_matches_the_principal_the_wire_prefix_hides() {
    let rows = [
        member("roles/storage.admin", "user:a@x.c", None),
        member("roles/viewer", "user:lookalike-a@x.c", None),
    ];
    let filter = compiled(r"^a@x\.c$");

    let matched: Vec<&str> = rows
        .iter()
        .filter(|row| IamPolicy::matches_filter(row, &filter))
        .map(|row| row.raw_member.as_str())
        .collect();

    assert_eq!(
        matched,
        vec!["user:a@x.c"],
        "an anchored pattern must name one principal exactly; without the \
         principal among the searchable fields it names none"
    );
}

/// `s` on the column already sorting must reverse the direction, and `s` on a
/// different column must start that column ascending rather than inherit the
/// previous direction.
///
/// The comparator tests above pass a direction in hand and never press the
/// key, so the same-field comparison behind the toggle is unasserted: invert
/// it and the direction can never be reversed from the keyboard on this table
/// — every press of `s` on the sort column re-sets it to ascending and nothing
/// on screen moves.
///
/// The three roles order differently from the three members, so a sort that
/// stayed on Member cannot pass the second half by coincidence.
#[test]
fn sorting_the_same_column_twice_reverses_it_and_a_new_column_starts_ascending() {
    let mut state = state_with(vec![
        member("roles/mmm.editor", "user:a@x.c", None),
        member("roles/aaa.admin", "user:b@x.c", None),
        member("roles/zzz.viewer", "user:c@x.c", None),
    ]);
    assert_eq!(
        visible_members(&state),
        vec!["user:a@x.c", "user:b@x.c", "user:c@x.c"],
        "the fixture opens on the default member-ascending sort"
    );

    // The column cursor starts on `name`, which is already the sort field.
    state.sort_selected_column();
    assert_eq!(
        visible_members(&state),
        vec!["user:c@x.c", "user:b@x.c", "user:a@x.c"],
        "sorting the column already sorting must toggle the direction; \
         re-setting the same field instead leaves the table unable to reverse"
    );

    state.sort_selected_column();
    assert_eq!(
        visible_members(&state),
        vec!["user:a@x.c", "user:b@x.c", "user:c@x.c"],
        "the toggle must go both ways"
    );

    // Leave the member column descending, so "ascending" below is a reset
    // rather than the direction that happened to be in force.
    state.sort_selected_column();
    // Column 1 is `role`, a field the table is not currently sorted by.
    state.select_next_column();
    state.sort_selected_column();
    assert_eq!(
        visible_members(&state),
        vec!["user:b@x.c", "user:a@x.c", "user:c@x.c"],
        "a new sort column must start ascending, whatever direction the \
         previous column was left in"
    );
}
