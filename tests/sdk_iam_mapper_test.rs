//! The IAM mappers, which a mutation sweep reported as untested:
//! `map_service_account` and `map_policy` in `src/services/sdk/iam.rs`. Every
//! field is planted with a distinct non-default value and the whole row is
//! asserted, so a body replaced wholesale with `Default::default()` cannot
//! pass; every branch — the non-empty display name, the present/absent binding
//! condition, the blank and whitespace title, the `deleted:` prefix and each
//! member kind including the prefixless and unrecognised ones — is exercised
//! from both sides with inputs that map to different rows.

use google_cloud_iam_admin_v1::model as admin;
use google_cloud_iam_v1::model as iam;

use nuvem::services::sdk::iam::{map_policy, map_service_account};
use nuvem_domain::gcp::resource::{IamMemberKind, IamPolicyMember, ServiceAccount};

fn binding(role: &str, members: &[&str]) -> iam::Binding {
    iam::Binding::new()
        .set_role(role)
        .set_members(members.iter().copied())
}

/// Attaches an IAM condition to a binding.
///
/// The condition's type (`google_cloud_type::model::Expr`) is reachable only
/// through `google-cloud-iam-v1`'s own dependency, which this crate does not
/// name, so the value is deserialised into the public field rather than built
/// with a setter. The field is the same one the transport fills.
///
/// `description` and `location` are planted with distinct non-default values
/// too: the row carries the *title*, and a mapper that read one of the other
/// three strings must produce a visibly wrong cell rather than an empty one
/// that an emptiness assertion would happily accept.
fn conditional(mut binding: iam::Binding, title: &str, expression: &str) -> iam::Binding {
    binding.condition = serde_json::from_value(serde_json::json!({
        "title": title,
        "expression": expression,
        "description": "Does not grant access after the cutover",
        "location": "policy.yaml:41",
    }))
    .expect("the generated model reads a condition object");
    binding
}

fn policy(bindings: impl IntoIterator<Item = iam::Binding>) -> iam::Policy {
    iam::Policy::new().set_version(3).set_bindings(bindings)
}

/// `(kind, principal)` for each row, the pair the member string is split into.
fn principals(rows: &[IamPolicyMember]) -> Vec<(IamMemberKind, &str)> {
    rows.iter()
        .map(|row| (row.kind.clone(), row.principal.as_str()))
        .collect()
}

// ---------------------------------------------------------------------------
// map_service_account
// ---------------------------------------------------------------------------

#[test]
fn a_disabled_account_carries_its_email_unique_id_and_display_name() {
    let account = admin::ServiceAccount::new()
        .set_email("billing-exporter@acme-prod.iam.gserviceaccount.com")
        .set_unique_id("114857392017465820193")
        .set_display_name("Billing exporter")
        .set_disabled(true);

    assert_eq!(
        map_service_account(&account),
        ServiceAccount {
            email: "billing-exporter@acme-prod.iam.gserviceaccount.com".to_string(),
            display_name: Some("Billing exporter".to_string()),
            disabled: true,
            unique_id: "114857392017465820193".to_string(),
        }
    );
}

#[test]
fn an_enabled_account_maps_the_disabled_flag_as_false() {
    let account = admin::ServiceAccount::new()
        .set_email("terraform@acme-prod.iam.gserviceaccount.com")
        .set_unique_id("102938475610293847561")
        .set_display_name("Terraform runner")
        .set_disabled(false);

    assert_eq!(
        map_service_account(&account),
        ServiceAccount {
            email: "terraform@acme-prod.iam.gserviceaccount.com".to_string(),
            display_name: Some("Terraform runner".to_string()),
            disabled: false,
            unique_id: "102938475610293847561".to_string(),
        }
    );
}

#[test]
fn an_empty_display_name_becomes_no_display_name() {
    let account = admin::ServiceAccount::new()
        .set_email("compute-default@acme-prod.iam.gserviceaccount.com")
        .set_unique_id("998877665544332211009")
        .set_display_name("")
        .set_disabled(true);

    assert_eq!(
        map_service_account(&account),
        ServiceAccount {
            email: "compute-default@acme-prod.iam.gserviceaccount.com".to_string(),
            // Not `Some("")`: a blank cell that every `is_some()` downstream
            // would believe in.
            display_name: None,
            disabled: true,
            unique_id: "998877665544332211009".to_string(),
        }
    );
}

#[test]
fn a_blank_but_non_empty_display_name_is_kept_verbatim() {
    // `non_empty` tests emptiness, not blankness — unlike the condition title,
    // which is trimmed. A name of spaces is a name the API holds.
    let account = admin::ServiceAccount::new()
        .set_email("spaced@acme-prod.iam.gserviceaccount.com")
        .set_unique_id("111222333444555666777")
        .set_display_name("  ")
        .set_disabled(false);

    assert_eq!(
        map_service_account(&account).display_name,
        Some("  ".to_string())
    );
}

#[test]
fn the_fields_the_row_does_not_carry_cannot_be_mistaken_for_the_ones_it_does() {
    // `name`, `project_id`, `description` and `oauth2_client_id` are all
    // distinct here: a mapper reading any of them in place of the four the row
    // holds produces a different row.
    let account = admin::ServiceAccount::new()
        .set_name("projects/acme-prod/serviceAccounts/other@acme-prod.iam.gserviceaccount.com")
        .set_project_id("acme-staging")
        .set_description("Reads the billing export dataset nightly")
        .set_oauth2_client_id("555555555555555555555")
        .set_email("nightly@acme-prod.iam.gserviceaccount.com")
        .set_unique_id("123456789012345678901")
        .set_display_name("Nightly export")
        .set_disabled(true);

    assert_eq!(
        map_service_account(&account),
        ServiceAccount {
            email: "nightly@acme-prod.iam.gserviceaccount.com".to_string(),
            display_name: Some("Nightly export".to_string()),
            disabled: true,
            unique_id: "123456789012345678901".to_string(),
        }
    );
}

// ---------------------------------------------------------------------------
// map_policy — flattening and the role
// ---------------------------------------------------------------------------

#[test]
fn every_member_of_every_binding_becomes_its_own_row_in_order() {
    let mapped = map_policy(&policy([
        binding(
            "roles/storage.admin",
            &[
                "user:ada@acme.example",
                "serviceAccount:etl@acme-prod.iam.gserviceaccount.com",
            ],
        ),
        binding("roles/viewer", &["group:auditors@acme.example"]),
    ]));

    assert_eq!(
        mapped,
        vec![
            IamPolicyMember {
                role: "roles/storage.admin".to_string(),
                kind: IamMemberKind::User,
                principal: "ada@acme.example".to_string(),
                deleted: false,
                condition: None,
                raw_member: "user:ada@acme.example".to_string(),
            },
            IamPolicyMember {
                role: "roles/storage.admin".to_string(),
                kind: IamMemberKind::ServiceAccount,
                principal: "etl@acme-prod.iam.gserviceaccount.com".to_string(),
                deleted: false,
                condition: None,
                raw_member: "serviceAccount:etl@acme-prod.iam.gserviceaccount.com".to_string(),
            },
            IamPolicyMember {
                role: "roles/viewer".to_string(),
                kind: IamMemberKind::Group,
                principal: "auditors@acme.example".to_string(),
                deleted: false,
                condition: None,
                raw_member: "group:auditors@acme.example".to_string(),
            },
        ]
    );
}

#[test]
fn a_custom_project_role_is_carried_verbatim_rather_than_shortened() {
    let mapped = map_policy(&policy([binding(
        "projects/acme-prod/roles/customAuditor",
        &["user:ada@acme.example"],
    )]));

    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].role, "projects/acme-prod/roles/customAuditor");
}

#[test]
fn a_binding_with_no_members_contributes_no_rows() {
    let mapped = map_policy(&policy([
        binding("roles/owner", &[]),
        binding("roles/viewer", &["user:ada@acme.example"]),
    ]));

    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].role, "roles/viewer");
    assert_eq!(mapped[0].principal, "ada@acme.example");
}

#[test]
fn a_policy_with_no_bindings_maps_to_no_rows() {
    assert_eq!(map_policy(&policy([])), Vec::<IamPolicyMember>::new());
}

// ---------------------------------------------------------------------------
// map_policy — the binding condition
// ---------------------------------------------------------------------------

#[test]
fn a_conditional_binding_puts_its_title_on_every_one_of_its_members() {
    let mapped = map_policy(&policy([conditional(
        binding(
            "roles/storage.objectViewer",
            &["user:ada@acme.example", "group:auditors@acme.example"],
        ),
        "expirable access",
        "request.time < timestamp('2026-10-01T00:00:00.000Z')",
    )]));

    assert_eq!(
        mapped
            .iter()
            .map(|row| row.condition.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("expirable access"), Some("expirable access")]
    );
}

#[test]
fn a_condition_title_does_not_leak_from_one_binding_to_the_next() {
    let mapped = map_policy(&policy([
        conditional(
            binding("roles/storage.admin", &["user:ada@acme.example"]),
            "only during the migration",
            "request.time < timestamp('2026-10-01T00:00:00.000Z')",
        ),
        binding("roles/viewer", &["user:grace@acme.example"]),
        conditional(
            binding("roles/logging.viewer", &["user:linus@acme.example"]),
            "prod logs only",
            "resource.name.startsWith('projects/acme-prod')",
        ),
    ]));

    assert_eq!(
        mapped
            .iter()
            .map(|row| (row.role.as_str(), row.condition.as_deref()))
            .collect::<Vec<_>>(),
        vec![
            ("roles/storage.admin", Some("only during the migration")),
            ("roles/viewer", None),
            ("roles/logging.viewer", Some("prod logs only")),
        ]
    );
}

#[test]
fn the_title_is_the_field_carried_and_not_the_expression_or_description() {
    // All four strings of the condition are distinct, so a mapper reading
    // `expression`, `description` or `location` produces a different cell
    // rather than an empty one that a "no condition" assertion would accept.
    let mapped = map_policy(&policy([conditional(
        binding("roles/storage.admin", &["user:ada@acme.example"]),
        "expirable access",
        "request.time < timestamp('2026-10-01T00:00:00.000Z')",
    )]));

    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].condition.as_deref(), Some("expirable access"));
}

#[test]
fn a_condition_with_a_blank_title_is_reported_as_no_condition() {
    // The binding *is* conditional — there is an expression — but a title-less
    // condition has nothing to paint, and an empty cell must not claim one.
    let mapped = map_policy(&policy([conditional(
        binding("roles/storage.admin", &["user:ada@acme.example"]),
        "",
        "request.time < timestamp('2026-10-01T00:00:00.000Z')",
    )]));

    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].condition, None);
}

#[test]
fn a_whitespace_only_condition_title_is_reported_as_no_condition() {
    let mapped = map_policy(&policy([conditional(
        binding("roles/storage.admin", &["user:ada@acme.example"]),
        "   ",
        "request.time < timestamp('2026-10-01T00:00:00.000Z')",
    )]));

    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].condition, None);
}

#[test]
fn a_padded_condition_title_is_trimmed_rather_than_dropped() {
    let mapped = map_policy(&policy([conditional(
        binding("roles/storage.admin", &["user:ada@acme.example"]),
        "  expires after the cutover  ",
        "request.time < timestamp('2026-10-01T00:00:00.000Z')",
    )]));

    assert_eq!(mapped.len(), 1);
    assert_eq!(
        mapped[0].condition.as_deref(),
        Some("expires after the cutover")
    );
}

// ---------------------------------------------------------------------------
// map_policy — member kinds
// ---------------------------------------------------------------------------

#[test]
fn the_prefixed_member_kinds_are_read_from_the_prefix() {
    let mapped = map_policy(&policy([binding(
        "roles/viewer",
        &[
            "user:ada@acme.example",
            "serviceAccount:etl@acme-prod.iam.gserviceaccount.com",
            "group:auditors@acme.example",
            "domain:acme.example",
        ],
    )]));

    assert_eq!(
        principals(&mapped),
        vec![
            (IamMemberKind::User, "ada@acme.example"),
            (
                IamMemberKind::ServiceAccount,
                "etl@acme-prod.iam.gserviceaccount.com"
            ),
            (IamMemberKind::Group, "auditors@acme.example"),
            (IamMemberKind::Domain, "acme.example"),
        ]
    );
}

#[test]
fn the_prefixless_public_kinds_keep_their_whole_token_as_the_principal() {
    let mapped = map_policy(&policy([binding(
        "roles/storage.objectViewer",
        &["allUsers", "allAuthenticatedUsers"],
    )]));

    assert_eq!(
        principals(&mapped),
        vec![
            (IamMemberKind::AllUsers, "allUsers"),
            (
                IamMemberKind::AllAuthenticatedUsers,
                "allAuthenticatedUsers"
            ),
        ]
    );
    assert_eq!(mapped[0].raw_member, "allUsers");
    assert_eq!(mapped[1].raw_member, "allAuthenticatedUsers");
}

#[test]
fn federated_members_keep_the_colons_inside_their_identifier() {
    let mapped = map_policy(&policy([binding(
        "roles/iam.workloadIdentityUser",
        &[
            "principal://iam.googleapis.com/locations/global/workforcePools/acme/subject/ada",
            "principalSet://iam.googleapis.com/projects/17/locations/global/workloadIdentityPools/gh/*",
        ],
    )]));

    assert_eq!(
        principals(&mapped),
        vec![
            (
                IamMemberKind::Principal,
                "//iam.googleapis.com/locations/global/workforcePools/acme/subject/ada"
            ),
            (
                IamMemberKind::PrincipalSet,
                "//iam.googleapis.com/projects/17/locations/global/workloadIdentityPools/gh/*"
            ),
        ]
    );
}

#[test]
fn an_unfamiliar_prefix_names_the_kind_verbatim_and_keeps_its_identity() {
    let mapped = map_policy(&policy([binding(
        "roles/editor",
        &["projectEditor:acme-prod", "notAKind"],
    )]));

    assert_eq!(
        principals(&mapped),
        vec![
            (
                IamMemberKind::Unrecognized("projectEditor".to_string()),
                "acme-prod"
            ),
            // No colon at all: the whole string is both the kind's name and
            // the only identity there is to show.
            (
                IamMemberKind::Unrecognized("notAKind".to_string()),
                "notAKind"
            ),
        ]
    );
}

#[test]
fn a_deleted_member_is_flagged_while_the_live_one_beside_it_is_not() {
    let mapped = map_policy(&policy([binding(
        "roles/pubsub.subscriber",
        &[
            "deleted:serviceAccount:gone@acme-prod.iam.gserviceaccount.com?uid=1029384756",
            "serviceAccount:here@acme-prod.iam.gserviceaccount.com",
        ],
    )]));

    assert_eq!(
        mapped,
        vec![
            IamPolicyMember {
                role: "roles/pubsub.subscriber".to_string(),
                kind: IamMemberKind::ServiceAccount,
                principal: "gone@acme-prod.iam.gserviceaccount.com?uid=1029384756".to_string(),
                deleted: true,
                condition: None,
                raw_member:
                    "deleted:serviceAccount:gone@acme-prod.iam.gserviceaccount.com?uid=1029384756"
                        .to_string(),
            },
            IamPolicyMember {
                role: "roles/pubsub.subscriber".to_string(),
                kind: IamMemberKind::ServiceAccount,
                principal: "here@acme-prod.iam.gserviceaccount.com".to_string(),
                deleted: false,
                condition: None,
                raw_member: "serviceAccount:here@acme-prod.iam.gserviceaccount.com".to_string(),
            },
        ]
    );
}

#[test]
fn a_deleted_user_keeps_its_kind_rather_than_becoming_one_of_its_own() {
    let mapped = map_policy(&policy([binding(
        "roles/viewer",
        &["deleted:user:ada@acme.example?uid=7788"],
    )]));

    assert_eq!(
        principals(&mapped),
        vec![(IamMemberKind::User, "ada@acme.example?uid=7788")]
    );
    assert!(mapped[0].deleted);
}

#[test]
fn only_the_unbounded_populations_are_reported_as_public() {
    let mapped = map_policy(&policy([binding(
        "roles/storage.objectViewer",
        &[
            "allUsers",
            "allAuthenticatedUsers",
            "user:ada@acme.example",
            "domain:acme.example",
            "deleted:user:grace@acme.example?uid=42",
        ],
    )]));

    assert_eq!(
        mapped
            .iter()
            .map(IamPolicyMember::is_public)
            .collect::<Vec<_>>(),
        vec![true, true, false, false, false]
    );
}

#[test]
fn every_kind_a_policy_can_produce_paints_its_own_label() {
    let mapped = map_policy(&policy([binding(
        "roles/viewer",
        &[
            "user:ada@acme.example",
            "serviceAccount:etl@acme-prod.iam.gserviceaccount.com",
            "group:auditors@acme.example",
            "domain:acme.example",
            "allUsers",
            "allAuthenticatedUsers",
            "principal://iam.googleapis.com/subject/ada",
            "principalSet://iam.googleapis.com/pools/gh/*",
            "projectEditor:acme-prod",
        ],
    )]));

    assert_eq!(
        mapped
            .iter()
            .map(|row| row.kind.label())
            .collect::<Vec<_>>(),
        vec![
            "User",
            "Service account",
            "Group",
            "Domain",
            "All users",
            "All authenticated users",
            "Principal",
            "Principal set",
            // The fallback shows what the API sent rather than a catch-all.
            "projectEditor",
        ]
    );
}
