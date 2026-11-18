//! VPC firewall rules in application state (slice 3).
//!
//! # The one thing this table must never get wrong
//!
//! `ACTION` says whether matching traffic is permitted or blocked, and
//! `disabled` says whether the rule is in force at all. A disabled DENY rule
//! reads exactly like a live DENY rule in every other column — same name, same
//! direction, same priority, same ranges — while the traffic an operator
//! believes is blocked is flowing. So the disabled state rides the *identity*
//! column as a badge rather than taking a column of its own: badges lead the
//! cell, and the identity column is the one the layout can never drop.
//!
//! `ACTION` itself is a single field for the same reason. `allowed` and
//! `denied` arrive as separate keys and exactly one is ever present; collapsed
//! at the parser into one [`FirewallAction`], no later reader can find the
//! pair in a state the API never sends and guess at it.
//!
//! # Global, so the name is the identity
//!
//! A firewall rule is global to its project, so the name alone identifies it —
//! the networks shape, not the subnets one. Two rules cannot share a name even
//! across networks.
//!
//! # List-only
//!
//! No detail pane, no lifecycle action. nuvem never creates, edits, enables or
//! disables a rule: this is a table for reading a project's posture, and the
//! one action worth having here would be the most dangerous button in the
//! application.

use super::*;

/// Marker type unifying the firewall rules service behind [`ResourceService`].
/// Never instantiated.
pub enum FirewallRules {}

impl ResourceService for FirewallRules {
    type Row = FirewallRule;
    type Detail = ();
    type SortField = FirewallSortField;
    type DetailTab = ();

    const KIND: ResourceServiceKind = ResourceServiceKind::FirewallRules;

    fn collection(
        state: &ResourcesState,
    ) -> &ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab> {
        &state.firewall_rules
    }

    fn collection_mut(
        state: &mut ResourcesState,
    ) -> &mut ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab>
    {
        &mut state.firewall_rules
    }

    fn matches_filter(row: &Self::Row, filter: &CompiledFilter) -> bool {
        firewall_matches_filter(row, filter)
    }

    /// The one service that narrows: with a jump's instance scope in
    /// state, only the rules that target that instance are visible.
    ///
    /// The predicate is the domain's, not a second reading of it here — see
    /// `nuvem_domain::gcp::resource::rule_targets_instance` for what "targets"
    /// means and, importantly, what it does not: this narrows by target set
    /// alone, so a DENY rule and a disabled rule both stay in the list.
    fn matches_narrowing(row: &Self::Row, state: &ResourcesState) -> bool {
        match state.firewall_narrowed_to() {
            Some(scope) => rule_targets_instance(row, scope),
            None => true,
        }
    }

    /// The needle sits in the name, the identity column of the rule table.
    /// The network stays a plain `default` for the reason the subnets fixture
    /// gives: a rule found through its network is not a rule found through
    /// itself.
    fn sample_row() -> Self::Row {
        FirewallRule {
            name: SAMPLE_ROW_NEEDLE.to_string(),
            network: "default".to_string(),
            direction: FirewallDirection::Ingress,
            priority: Some(1000),
            action: FirewallAction::Allow,
            targets: FirewallTargets::AllInstances,
            source_or_dest: vec!["10.0.0.0/8".to_string()],
            disabled: false,
        }
    }

    fn compare(
        left: &Self::Row,
        right: &Self::Row,
        sort_field: Self::SortField,
        direction: SortDirection,
    ) -> std::cmp::Ordering {
        compare_firewall_rules(left, right, sort_field, direction)
    }

    /// A firewall rule is global to its project, so the name alone is the
    /// identity — the networks shape rather than the subnets one, where the
    /// region had to qualify a name that repeated.
    fn row_id(row: &Self::Row) -> ResourceRowId {
        ResourceRowId {
            name: row.name.clone(),
            location: String::new(),
        }
    }
}

/// The sortable columns of the firewall rules table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirewallSortField {
    Name,
    Network,
    Direction,
    Priority,
    Action,
    Targets,
    SourceOrDest,
}

impl FirewallSortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Network => "Network",
            Self::Direction => "Direction",
            Self::Priority => "Priority",
            Self::Action => "Action",
            Self::Targets => "Targets",
            Self::SourceOrDest => "Source/dest",
        }
    }

    pub const COLUMN_COUNT: usize = 7;

    pub fn from_column(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Name),
            1 => Some(Self::Network),
            2 => Some(Self::Direction),
            3 => Some(Self::Priority),
            4 => Some(Self::Action),
            5 => Some(Self::Targets),
            6 => Some(Self::SourceOrDest),
            _ => None,
        }
    }

    pub fn column(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Network => 1,
            Self::Direction => 2,
            Self::Priority => 3,
            Self::Action => 4,
            Self::Targets => 5,
            Self::SourceOrDest => 6,
        }
    }
}

/// The badge a disabled rule wears on its name cell.
///
/// A badge rather than a column because the identity column is the one the
/// layout may never drop: a disabled DENY rule that lost its "disabled" on a
/// narrow terminal reads as a live DENY rule, and an operator concludes
/// traffic is blocked that is flowing. Beside [`ORPHAN_BADGE`] and the rest,
/// which mark the same kind of "this row is not what it looks like" fact.
pub const DISABLED_BADGE: &str = "DISABLED";

/// What the `TARGETS` cell paints.
///
/// An empty target list is not a blank: a rule with no tag or service-account
/// scoping applies to **every instance in the network**, which is the widest
/// possible scope and the opposite of "nothing". Saying so is the difference
/// between an operator reading a blank cell as "narrow" and reading it as what
/// it is.
pub fn firewall_targets_label(rule: &FirewallRule) -> String {
    let values = rule.targets.values();
    if values.is_empty() {
        "all instances".to_string()
    } else {
        values.join(", ")
    }
}

/// What the `SOURCE/DEST` cell paints: the ranges joined, or a dash where the
/// rule scopes by something this column does not carry.
pub fn firewall_source_or_dest_label(rule: &FirewallRule) -> Option<String> {
    (!rule.source_or_dest.is_empty()).then(|| rule.source_or_dest.join(", "))
}

impl ResourcesState {
    pub fn firewall_rules(&self) -> &[FirewallRule] {
        &self.firewall_rules.rows
    }

    /// Delegated to the generic `ResourcesState::visible_rows` rather than
    /// filtering and sorting here.
    ///
    /// It used to be a hand-rolled copy of that function — same filter, same
    /// comparator, same fields — and while the two agreed nothing was wrong.
    /// The narrowing broke the agreement: applied in `visible_rows` it reached
    /// the cursor, the mark ids and `select_visible_row`, and not the rows the
    /// table paints. That is the worst shape this could take, a selection on a
    /// row the operator cannot see, so there is now one implementation and
    /// this asks it.
    pub fn visible_firewall_rules(&self) -> Vec<&FirewallRule> {
        self.visible_rows::<FirewallRules>()
    }

    pub fn visible_firewall_rule_count(&self) -> usize {
        self.visible_firewall_rules().len()
    }

    pub fn selected_firewall_rule(&self) -> Option<&FirewallRule> {
        self.visible_firewall_rules()
            .get(self.firewall_rules.selected_visible)
            .copied()
    }

    /// Installs a freshly loaded list, keeping the cursor on the rule it was
    /// already on where that rule is still listed.
    pub fn set_firewall_rules(&mut self, context: GcloudContext, rules: Vec<FirewallRule>) {
        let previously_selected = self.selected_firewall_rule().map(|rule| rule.name.clone());

        self.firewall_rules.context = Some(context);
        self.firewall_rules.rows = rules;
        self.firewall_rules.stale = false;
        self.firewall_rules.last_problem = None;
        self.firewall_rules.last_diagnostic = None;
        self.firewall_rules.clear_load_phase();

        self.firewall_rules.selected_visible = previously_selected
            .as_ref()
            .and_then(|name| {
                self.visible_firewall_rules()
                    .iter()
                    .position(|rule| &rule.name == name)
            })
            .unwrap_or_else(|| {
                self.firewall_rules
                    .selected_visible
                    .min(self.visible_firewall_rules().len().saturating_sub(1))
            });
    }

    pub fn mark_firewall_rules_stale_and_clear(&mut self) {
        self.firewall_rules.filter_draft = self.firewall_rules.filter_applied.source().to_string();
        self.firewall_rules.filter_input_active = false;
        self.firewall_rules.rows.clear();
        self.firewall_rules.context = None;
        self.firewall_rules.selected_visible = 0;
        self.firewall_rules.stale = true;
    }

    pub fn firewall_rules_problem(&self) -> Option<&DiagnosticProblem> {
        self.firewall_rules.last_problem.as_ref()
    }
}

/// The values a firewall filter searches.
///
/// Every cell the table paints, and two of them matter more than usual here.
/// `ACTION` contributes `ALLOW`/`DENY`, which appear in no raw field — the
/// rule carries a *key*, not a word — so `/DENY` finding every blocking rule
/// depends entirely on this. The `DISABLED` badge is searchable for the same
/// reason `PUBLIC` is on the IAM table: an operator hunting rules that are not
/// in force types the word they can see.
///
/// `all instances` is searchable too, which is what lets an operator find the
/// unscoped rules — the widest ones in the project — by the words the cell
/// actually shows.
fn firewall_searchable_fields(rule: &FirewallRule) -> Vec<String> {
    let mut fields = vec![
        rule.name.clone(),
        rule.network.clone(),
        rule.direction.label().to_string(),
        rule.priority.map(|p| p.to_string()).unwrap_or_default(),
        rule.action.label().to_string(),
        firewall_targets_label(rule),
        firewall_source_or_dest_label(rule).unwrap_or_default(),
    ];
    if rule.disabled {
        fields.push(DISABLED_BADGE.to_string());
    }
    fields
}

fn firewall_matches_filter(rule: &FirewallRule, filter: &CompiledFilter) -> bool {
    filter.matches(&firewall_searchable_fields(rule))
}

fn compare_firewall_rules(
    left: &FirewallRule,
    right: &FirewallRule,
    sort_field: FirewallSortField,
    direction: SortDirection,
) -> std::cmp::Ordering {
    let primary = match sort_field {
        FirewallSortField::Name => left.name.cmp(&right.name),
        FirewallSortField::Network => left.network.cmp(&right.network),
        FirewallSortField::Direction => left.direction.label().cmp(&right.direction.label()),
        // Numerically, and this is the column an operator sorts to read the
        // rules in evaluation order: lower priority wins, so ascending is the
        // order traffic actually meets them in. A text sort would put 1000
        // ahead of 900 and misrepresent which rule fires first. A rule with no
        // reported priority sorts as absent rather than as zero, so it does
        // not claim to be evaluated before every real rule.
        FirewallSortField::Priority => left.priority.cmp(&right.priority),
        FirewallSortField::Action => left.action.label().cmp(right.action.label()),
        FirewallSortField::Targets => {
            firewall_targets_label(left).cmp(&firewall_targets_label(right))
        }
        FirewallSortField::SourceOrDest => firewall_source_or_dest_label(left)
            .unwrap_or_default()
            .cmp(&firewall_source_or_dest_label(right).unwrap_or_default()),
    };
    // The name breaks every tie and is unique within the project, so this is a
    // total order over the row set.
    let ordering = primary.then_with(|| left.name.cmp(&right.name));
    match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    }
}
