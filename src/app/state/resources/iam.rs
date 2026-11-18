//! IAM in application state: the service accounts table and the project's IAM
//! policy.
//!
//! Two services in one module because they share a loader
//! (`crate::services::resources::iam`) and change together.
//!
//! # List-only, both of them
//!
//! Neither table has a detail pane: `Detail` and `DetailTab` are `()`, so
//! `Enter` on a row does nothing at all. That is the shape Cloud Functions
//! shipped in and Pub/Sub topics shipped in, and it is deliberate rather than
//! unfinished — a pane that opens on nothing is worse than one not offered yet,
//! which is also why there is no detail tab enum here carrying variants nothing
//! renders. The policy is browsed, never edited: a slice that can change an IAM
//! policy is a different feature with a different threat model.
//!
//! # Why the policy is its own service rather than a column here
//!
//! The policy's row is a (role, member) pair, not a service account. Its 188
//! members in a representative project include `user:` and `group:`
//! principals, and those are exactly what a project audit is looking for. A
//! roles column on the *accounts* table could only show the service-account
//! subset, so it would quietly answer "who has access" with the humans left out
//! — an answer that invites the conclusion that nobody else has any.
//!
//! There is deliberately no "roles granted" enrichment column on the accounts
//! table, and it is not the `SubscriptionCount` situation it resembles.
//! Pub/Sub's count answered a question no other view could answer; this one is
//! answered by filtering the policy table for the account's email.
//!
//! # Nothing here parses a member
//!
//! `IamPolicyMember` arrives already flattened, with its kind and principal
//! read off the wire by `IamPolicyMember::from_wire`. Re-deriving either from
//! `raw_member` in this layer would be a second, divergent parser for the
//! member grammar.
//!
//! # No label filtering
//!
//! Neither a service account nor a policy binding carries user labels — the IAM
//! API has no field for them — so `gcp::filter::label_filter_field` returns
//! `None` for both kinds and `-l k=v` narrows nothing rather than silently
//! matching every row.
use super::*;

/// Marker type unifying the IAM service accounts service behind
/// [`ResourceService`]. Never instantiated.
pub enum IamServiceAccounts {}

impl ResourceService for IamServiceAccounts {
    type Row = ServiceAccount;
    type Detail = ();
    type SortField = ServiceAccountSortField;
    type DetailTab = ();

    const KIND: ResourceServiceKind = ResourceServiceKind::IamServiceAccounts;

    fn collection(
        state: &ResourcesState,
    ) -> &ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab> {
        &state.service_accounts
    }

    fn collection_mut(
        state: &mut ResourcesState,
    ) -> &mut ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab>
    {
        &mut state.service_accounts
    }

    fn matches_filter(row: &Self::Row, filter: &CompiledFilter) -> bool {
        service_account_matches_filter(row, filter)
    }

    /// The needle sits in the email, which is a service account's identity
    /// column and the string every other resource names it by.
    fn sample_row() -> Self::Row {
        ServiceAccount {
            email: format!("{SAMPLE_ROW_NEEDLE}@sample-project.iam.gserviceaccount.com"),
            display_name: None,
            disabled: false,
            unique_id: "100000000000000000001".to_string(),
        }
    }

    fn compare(
        left: &Self::Row,
        right: &Self::Row,
        sort_field: Self::SortField,
        direction: SortDirection,
    ) -> std::cmp::Ordering {
        compare_service_accounts(left, right, sort_field, direction)
    }

    /// The email is the identity: it is unique within the project and is what
    /// every other resource names a service account by. A service account is
    /// global within its project, so there is no location to qualify it with —
    /// the Secret Manager and Pub/Sub shape, not the Cloud Functions one.
    fn row_id(row: &Self::Row) -> ResourceRowId {
        ResourceRowId {
            name: row.email.clone(),
            location: String::new(),
        }
    }
}

/// The sortable columns of the service accounts table. Every catalogue column
/// sorts: unlike the tables that carry labels, none of these cells is a
/// flattened map whose ordering would mean nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAccountSortField {
    Email,
    DisplayName,
    Disabled,
    UniqueId,
}

impl ServiceAccountSortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Email => "Email",
            Self::DisplayName => "Display name",
            Self::Disabled => "State",
            Self::UniqueId => "Unique ID",
        }
    }

    pub const COLUMN_COUNT: usize = 4;

    pub fn from_column(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Email),
            1 => Some(Self::DisplayName),
            2 => Some(Self::Disabled),
            3 => Some(Self::UniqueId),
            _ => None,
        }
    }

    pub fn column(self) -> usize {
        match self {
            Self::Email => 0,
            Self::DisplayName => 1,
            Self::Disabled => 2,
            Self::UniqueId => 3,
        }
    }
}

/// The state cell an operator reads, and the value the filter searches.
///
/// Two words rather than a badge on one of them: unlike the disks table's
/// `ORPHAN`, where the absence of the badge is the ordinary case and needs no
/// word, "this identity cannot authenticate" is a fact worth reading directly
/// off every row of an access review.
pub fn service_account_state_label(account: &ServiceAccount) -> &'static str {
    if account.disabled {
        "Disabled"
    } else {
        "Enabled"
    }
}

impl ResourcesState {
    pub fn service_accounts(&self) -> &[ServiceAccount] {
        &self.service_accounts.rows
    }

    pub fn visible_service_accounts(&self) -> Vec<&ServiceAccount> {
        let filter = self.service_accounts.filter_applied.filter();
        let mut rows = self
            .service_accounts
            .rows
            .iter()
            .filter(|account| service_account_matches_filter(account, filter))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            compare_service_accounts(
                left,
                right,
                self.service_accounts.sort_field,
                self.service_accounts.sort_direction,
            )
        });
        rows
    }

    pub fn visible_service_account_count(&self) -> usize {
        let filter = self.service_accounts.filter_applied.filter();
        self.service_accounts
            .rows
            .iter()
            .filter(|account| service_account_matches_filter(account, filter))
            .count()
    }

    pub fn selected_service_account(&self) -> Option<&ServiceAccount> {
        self.visible_service_accounts()
            .get(self.service_accounts.selected_visible)
            .copied()
    }

    /// Installs a freshly loaded list, keeping the cursor on the account it was
    /// already on where that account is still listed.
    pub fn set_service_accounts(&mut self, context: GcloudContext, accounts: Vec<ServiceAccount>) {
        let previously_selected = self
            .selected_service_account()
            .map(|account| account.email.clone());

        self.service_accounts.context = Some(context);
        self.service_accounts.rows = accounts;
        self.service_accounts.stale = false;
        self.service_accounts.last_problem = None;
        self.service_accounts.last_diagnostic = None;
        self.service_accounts.clear_load_phase();

        self.service_accounts.selected_visible = previously_selected
            .as_ref()
            .and_then(|email| {
                self.visible_service_accounts()
                    .iter()
                    .position(|account| &account.email == email)
            })
            .unwrap_or_else(|| {
                self.service_accounts
                    .selected_visible
                    .min(self.visible_service_accounts().len().saturating_sub(1))
            });
    }

    pub fn mark_service_accounts_stale_and_clear(&mut self) {
        self.service_accounts.filter_draft =
            self.service_accounts.filter_applied.source().to_string();
        self.service_accounts.filter_input_active = false;
        self.service_accounts.rows.clear();
        self.service_accounts.context = None;
        self.service_accounts.selected_visible = 0;
        self.service_accounts.stale = true;
    }

    pub fn service_accounts_problem(&self) -> Option<&DiagnosticProblem> {
        self.service_accounts.last_problem.as_ref()
    }
}

/// The values a service accounts filter searches.
///
/// Every cell the table paints, because `/` narrows what is on screen and a
/// term an operator can read in a column but cannot filter by is the filter
/// lying about the rows it hid. That includes the *state label* — `Disabled` is
/// the word on screen, and an access review filters for exactly it.
///
/// The never-drop-a-row premise holds trivially here because this service
/// pushes no clause down at all (see `crate::services::resources::iam`), so
/// every one of these values is matched client-side against a row that was
/// never narrowed server-side.
fn service_account_searchable_fields(account: &ServiceAccount) -> Vec<String> {
    vec![
        account.email.clone(),
        account.display_name.clone().unwrap_or_default(),
        service_account_state_label(account).to_string(),
        account.unique_id.clone(),
    ]
}

fn service_account_matches_filter(account: &ServiceAccount, filter: &CompiledFilter) -> bool {
    filter.matches(&service_account_searchable_fields(account))
}

fn compare_service_accounts(
    left: &ServiceAccount,
    right: &ServiceAccount,
    sort_field: ServiceAccountSortField,
    direction: SortDirection,
) -> std::cmp::Ordering {
    let primary = match sort_field {
        ServiceAccountSortField::Email => left.email.cmp(&right.email),
        // An account with no display name sorts as if the name were empty, so
        // those rows group at one end rather than scattering.
        ServiceAccountSortField::DisplayName => left
            .display_name
            .as_deref()
            .unwrap_or_default()
            .cmp(right.display_name.as_deref().unwrap_or_default()),
        // Disabled accounts sort together, and — because `false < true` —
        // ascending puts the enabled ones first, so one keypress brings the
        // disabled tail into view.
        ServiceAccountSortField::Disabled => left.disabled.cmp(&right.disabled),
        // Lexicographic, not numeric: a unique id is a 21-digit opaque token
        // that no operator reads as a quantity, and every id is the same
        // length, so the two orderings agree anyway.
        ServiceAccountSortField::UniqueId => left.unique_id.cmp(&right.unique_id),
    };
    // The email breaks every tie and is unique within the project, so this is
    // a total order over the row set.
    let ordering = primary.then_with(|| left.email.cmp(&right.email));
    match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    }
}

/// The text the member cell paints on a grant held by an unbounded population,
/// and the term an operator types into `/` to isolate the bindings an audit
/// exists to find.
///
/// One constant because the cell and the searchable field must be the same
/// string: a badge an operator can read but cannot filter by is the filter
/// lying about the rows it hid. The same reason [`ORPHAN_BADGE`] is one.
///
/// [`ORPHAN_BADGE`]: super::disks::ORPHAN_BADGE
pub const PUBLIC_BADGE: &str = "PUBLIC";

/// The text the member cell paints on a grant to a principal that has since
/// been deleted. A constant for the same reason as [`PUBLIC_BADGE`], though
/// this one needs no searchable field of its own: see
/// `policy_member_searchable_fields`.
pub const DELETED_BADGE: &str = "DELETED";

/// Marker type unifying the project IAM policy behind [`ResourceService`].
/// Never instantiated.
pub enum IamPolicy {}

impl ResourceService for IamPolicy {
    type Row = IamPolicyMember;
    type Detail = ();
    type SortField = IamPolicySortField;
    type DetailTab = ();

    const KIND: ResourceServiceKind = ResourceServiceKind::IamPolicy;

    fn collection(
        state: &ResourcesState,
    ) -> &ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab> {
        &state.policy_members
    }

    fn collection_mut(
        state: &mut ResourcesState,
    ) -> &mut ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab>
    {
        &mut state.policy_members
    }

    fn matches_filter(row: &Self::Row, filter: &CompiledFilter) -> bool {
        filter.matches(&policy_member_searchable_fields(row))
    }

    /// The needle sits in the member — both the raw string and the principal
    /// parsed out of it — because a binding is identified by who it grants
    /// to, not by the role, which repeats across rows.
    fn sample_row() -> Self::Row {
        let principal = format!("{SAMPLE_ROW_NEEDLE}@sample-project.iam.gserviceaccount.com");
        IamPolicyMember {
            role: "roles/viewer".to_string(),
            kind: IamMemberKind::ServiceAccount,
            raw_member: format!("serviceAccount:{principal}"),
            principal,
            deleted: false,
            condition: None,
        }
    }

    fn compare(
        left: &Self::Row,
        right: &Self::Row,
        sort_field: Self::SortField,
        direction: SortDirection,
    ) -> std::cmp::Ordering {
        compare_policy_members(left, right, sort_field, direction)
    }

    /// The pair is the identity, and the condition is part of the pair: two
    /// bindings can carry the same role and the same member and differ only by
    /// their condition, and those are two distinct grants. Without the
    /// condition here, marking one would mark both.
    fn row_id(row: &Self::Row) -> ResourceRowId {
        ResourceRowId {
            name: row.raw_member.clone(),
            location: match &row.condition {
                Some(title) => format!("{}\u{1f}{title}", row.role),
                None => row.role.clone(),
            },
        }
    }
}

/// The sortable columns of the policy table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IamPolicySortField {
    Member,
    Role,
    Kind,
    Condition,
}

impl IamPolicySortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Member => "Member",
            Self::Role => "Role",
            Self::Kind => "Type",
            Self::Condition => "Condition",
        }
    }

    pub const COLUMN_COUNT: usize = 4;

    pub fn from_column(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Member),
            1 => Some(Self::Role),
            2 => Some(Self::Kind),
            3 => Some(Self::Condition),
            _ => None,
        }
    }

    pub fn column(self) -> usize {
        match self {
            Self::Member => 0,
            Self::Role => 1,
            Self::Kind => 2,
            Self::Condition => 3,
        }
    }
}

impl ResourcesState {
    pub fn policy_members(&self) -> &[IamPolicyMember] {
        &self.policy_members.rows
    }

    pub fn visible_policy_members(&self) -> Vec<&IamPolicyMember> {
        let filter = self.policy_members.filter_applied.filter();
        let mut rows = self
            .policy_members
            .rows
            .iter()
            .filter(|member| policy_member_matches_filter(member, filter))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            compare_policy_members(
                left,
                right,
                self.policy_members.sort_field,
                self.policy_members.sort_direction,
            )
        });
        rows
    }

    pub fn visible_policy_member_count(&self) -> usize {
        let filter = self.policy_members.filter_applied.filter();
        self.policy_members
            .rows
            .iter()
            .filter(|member| policy_member_matches_filter(member, filter))
            .count()
    }

    pub fn selected_policy_member(&self) -> Option<&IamPolicyMember> {
        self.visible_policy_members()
            .get(self.policy_members.selected_visible)
            .copied()
    }

    /// Installs a freshly loaded policy, keeping the cursor on the binding it
    /// was already on where that binding is still granted.
    pub fn set_policy_members(&mut self, context: GcloudContext, members: Vec<IamPolicyMember>) {
        let previously_selected = self.selected_policy_member().map(IamPolicy::row_id);

        self.policy_members.context = Some(context);
        self.policy_members.rows = members;
        self.policy_members.stale = false;
        self.policy_members.last_problem = None;
        self.policy_members.last_diagnostic = None;
        self.policy_members.clear_load_phase();

        self.policy_members.selected_visible = previously_selected
            .as_ref()
            .and_then(|id| {
                self.visible_policy_members()
                    .iter()
                    .position(|member| &IamPolicy::row_id(member) == id)
            })
            .unwrap_or_else(|| {
                self.policy_members
                    .selected_visible
                    .min(self.visible_policy_members().len().saturating_sub(1))
            });
    }

    pub fn mark_policy_members_stale_and_clear(&mut self) {
        self.policy_members.filter_draft = self.policy_members.filter_applied.source().to_string();
        self.policy_members.filter_input_active = false;
        self.policy_members.rows.clear();
        self.policy_members.context = None;
        self.policy_members.selected_visible = 0;
        self.policy_members.stale = true;
    }

    pub fn policy_members_problem(&self) -> Option<&DiagnosticProblem> {
        self.policy_members.last_problem.as_ref()
    }
}

fn compare_policy_members(
    left: &IamPolicyMember,
    right: &IamPolicyMember,
    sort_field: IamPolicySortField,
    direction: SortDirection,
) -> std::cmp::Ordering {
    let primary = match sort_field {
        IamPolicySortField::Member => left.raw_member.cmp(&right.raw_member),
        // Verbatim, so a custom role sorts as the distinct grant it is rather
        // than colliding with the predefined role of the same trailing name.
        IamPolicySortField::Role => left.role.cmp(&right.role),
        IamPolicySortField::Kind => left.kind.label().cmp(right.kind.label()),
        // Unconditional rows sort as if the title were empty, so they group at
        // one end instead of scattering through the conditional ones.
        IamPolicySortField::Condition => left
            .condition
            .as_deref()
            .unwrap_or_default()
            .cmp(right.condition.as_deref().unwrap_or_default()),
    };
    // (member, role, condition) is the row identity, so breaking every tie with
    // it in that order makes this a total order over the row set — which is
    // what keeps a sort stable across refreshes.
    let ordering = primary
        .then_with(|| left.raw_member.cmp(&right.raw_member))
        .then_with(|| left.role.cmp(&right.role))
        .then_with(|| left.condition.cmp(&right.condition));
    match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    }
}

/// Every value a filter may match against. `raw_member`, so that filtering by
/// `allUsers` or by a `deleted:` prefix finds the rows an audit is looking for,
/// and the role, so that filtering by a role answers "who holds this role".
///
/// `principal` is always a substring of `raw_member`, so it changes nothing in
/// substring or fuzzy mode; it earns its place in regex mode, where an anchored
/// `^a@x\.c$` names one identity exactly and would otherwise match nothing —
/// the wire prefix is part of `raw_member` and an anchored pattern cannot see
/// past it. That is the query an audit reaches for when a lookalike address
/// makes an unanchored search ambiguous.
///
/// [`PUBLIC_BADGE`] contributes its own text, for the badged rows only, so
/// `/PUBLIC` isolates exactly the rows wearing it — the same rule the disks
/// table follows for `ORPHAN`: a badge an operator can read but cannot filter
/// by is the filter lying about the rows it hid. [`DELETED_BADGE`] needs no
/// entry of its own, because a deleted principal's `raw_member` is literally
/// prefixed `deleted:` and the filter is case-insensitive, so `/DELETED`
/// already finds exactly those rows; pushing it would be a field no input can
/// distinguish.
fn policy_member_searchable_fields(row: &IamPolicyMember) -> Vec<String> {
    let mut fields = vec![
        row.raw_member.clone(),
        row.principal.clone(),
        row.role.clone(),
        row.kind.label().to_string(),
    ];
    if let Some(title) = &row.condition {
        fields.push(title.clone());
    }
    if row.is_public() {
        fields.push(PUBLIC_BADGE.to_string());
    }
    fields
}

fn policy_member_matches_filter(row: &IamPolicyMember, filter: &CompiledFilter) -> bool {
    filter.matches(&policy_member_searchable_fields(row))
}
