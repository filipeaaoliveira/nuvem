//! User-configurable cross-resource jumps: the `[[jumps]]` array of the config
//! file.
//!
//! A rule says "from this service, offer a jump to that detail tab, naming the
//! target with these templates". Both ends name a service out of the same
//! vocabulary, `crate::config::resolve_service_kind` — `from` directly, `to`
//! through [`DetailTab::from_config_key`], which resolves the service half of
//! its key that way and only the tab half against the tab bar's own labels.
//! Neither end is a second list of names to keep in step, which is the
//! discipline `catalog_meta` gives `[columns]`.
//!
//! Validation follows `[aliases]`/`[hotkeys]`/`[columns]`: a problem is a
//! warning, never a load failure. It differs from `[columns]` in *granularity*.
//! A column block is one coherent table specification, so it is validated
//! all-or-nothing; jump rules are independent items, and dropping one does not
//! corrupt the others. Each rejected rule gets its own warning naming its
//! index.
//!
//! That is why the rules arrive here as raw [`toml::Value`]s: the *shape* of a
//! rule is checked here too, so a type slip in one rule is one more per-rule
//! warning rather than a deserialization failure that would discard the user's
//! whole file (theme, aliases, hotkeys and all).

use nuvem_domain::gcp::resource::ResourceServiceKind;

use crate::app::state::navigation::{DetailTab, JumpTarget};
use crate::config::{config_service_id, resolve_service_kind};

/// The tail every rule-invalidating warning ends with.
const FALLBACK: &str = "ignoring this rule";

/// Every key a `[[jumps]]` rule may carry; anything else is a typo and
/// rejects the rule.
const SETTINGS: [&str; 6] = ["from", "to", "name", "location", "label", "select"];

/// One value a template may substitute. The union of every service's fields;
/// which of them a given service actually offers is [`fields_for`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JumpField {
    Name,
    Zone,
    Region,
    Location,
    MachineType,
    Status,
    InternalIp,
    ExternalIp,
    Engine,
    Tier,
    /// The VPC network a row is attached to.
    ///
    /// Read straight off the row for a subnet and a firewall rule, and off the
    /// *detail* payload for a GCE instance, which carries no network on its
    /// row at all. [`detail_backed_fields`] is what records that difference,
    /// and it is the difference a diagnostic has to know about: an absent
    /// `{network}` on an instance usually means the pane is not fetched, not
    /// that the instance has no network.
    Network,
    /// The subnet a GCE instance's primary interface sits in.
    /// Detail-backed like [`Network`](Self::Network), and offered for no other
    /// service: a subnet row's own name is `{name}` there.
    ///
    /// Not an identity on its own — see [`SubnetworkRegion`](Self::SubnetworkRegion).
    Subnetwork,
    /// The region of that subnet.
    ///
    /// Its own field rather than a flavour of `{location}`, because it is not
    /// the *instance's* location: an instance has a zone, and a zone is not a
    /// region. Naming it separately is what lets a rule key the subnets table,
    /// whose rows are identified by name and region both — an auto-mode
    /// network calls every one of its subnets `default`.
    SubnetworkRegion,
}

/// The fields of `kind` that are read from its *detail* payload rather than
/// from the list row (slice 2).
///
/// These are the only fields that can be absent while the resource plainly has
/// a value, because the pane they live on has not been fetched. Everything that
/// consults this table is telling those two absences apart; nothing else in the
/// jump layer needs to care where a value came from.
///
/// Exhaustive rather than a `_ => &[]` catch-all, so a service that grows a
/// detail-backed field has to answer here rather than inheriting "row-backed"
/// silently — which is exactly the shape of the bug this table prevents.
/// `every_detail_backed_field_is_declared_and_absent_without_its_detail` walks
/// all sixteen services against what `selected_fields` actually produces.
pub fn detail_backed_fields(kind: ResourceServiceKind) -> &'static [JumpField] {
    match kind {
        // `ComputeInstance` carries no network and no subnetwork; both live on
        // `ComputeInstanceDetail.network_interfaces`.
        ResourceServiceKind::ComputeInstances => &[
            JumpField::Network,
            JumpField::Subnetwork,
            JumpField::SubnetworkRegion,
        ],
        // Every other service's whole vocabulary is columns of its row. A VPC
        // subnet and a firewall rule *do* offer `{network}`, and theirs is a
        // row field — which is why this question is asked per service and not
        // per field.
        ResourceServiceKind::GkeClusters
        | ResourceServiceKind::CloudSql
        | ResourceServiceKind::CloudRunServices
        | ResourceServiceKind::CloudStorage
        | ResourceServiceKind::SecretManagerSecrets
        | ResourceServiceKind::CloudFunctions
        | ResourceServiceKind::PersistentDisks
        | ResourceServiceKind::PubSubTopics
        | ResourceServiceKind::PubSubSubscriptions
        | ResourceServiceKind::BigQueryDatasets
        | ResourceServiceKind::IamServiceAccounts
        | ResourceServiceKind::IamPolicy
        | ResourceServiceKind::VpcNetworks
        | ResourceServiceKind::VpcSubnets
        | ResourceServiceKind::FirewallRules => &[],
    }
}

/// The fields a rule may name for a given source service, keyed by the name a
/// user writes between braces. Defined once: [`Template::parse`] validates
/// against it and task 2's value lookup answers for exactly these.
pub fn fields_for(kind: ResourceServiceKind) -> &'static [(&'static str, JumpField)] {
    match kind {
        // `network` and `subnetwork` are the two detail-backed entries in the
        // whole table (slice 2); see `detail_backed_fields`.
        ResourceServiceKind::ComputeInstances => &[
            ("name", JumpField::Name),
            ("zone", JumpField::Zone),
            ("machine_type", JumpField::MachineType),
            ("status", JumpField::Status),
            ("internal_ip", JumpField::InternalIp),
            ("external_ip", JumpField::ExternalIp),
            ("network", JumpField::Network),
            ("subnetwork", JumpField::Subnetwork),
            ("subnetwork_region", JumpField::SubnetworkRegion),
        ],
        ResourceServiceKind::CloudSql => &[
            ("name", JumpField::Name),
            ("region", JumpField::Region),
            ("engine", JumpField::Engine),
            ("tier", JumpField::Tier),
            ("status", JumpField::Status),
        ],
        ResourceServiceKind::CloudRunServices => {
            &[("name", JumpField::Name), ("region", JumpField::Region)]
        }
        ResourceServiceKind::GkeClusters => {
            &[("name", JumpField::Name), ("location", JumpField::Location)]
        }
        ResourceServiceKind::CloudStorage => {
            &[("name", JumpField::Name), ("location", JumpField::Location)]
        }
        // A secret is a global resource with no location of its own, so `name`
        // is the only field a rule can key on.
        ResourceServiceKind::SecretManagerSecrets => &[("name", JumpField::Name)],
        // A function's name is unique within its region, not within the
        // project, so `region` is part of naming one rather than decoration.
        ResourceServiceKind::CloudFunctions => {
            &[("name", JumpField::Name), ("region", JumpField::Region)]
        }
        // `location` rather than `zone` or `region`: `compute disks list`
        // returns zonal and regional disks in one array, so one of the two
        // spellings would name nothing for half the row set.
        ResourceServiceKind::PersistentDisks => {
            &[("name", JumpField::Name), ("location", JumpField::Location)]
        }
        // A topic is global and its id is unique within the project, so `name`
        // is the only field a rule can key on — the Secret Manager shape.
        ResourceServiceKind::PubSubTopics => &[("name", JumpField::Name)],
        // A VPC network is global — that is what distinguishes it from its own
        // subnets — so `name` is the only field a rule can key on, and there
        // is no location to qualify it with. The topics shape.
        ResourceServiceKind::VpcNetworks => &[("name", JumpField::Name)],
        // A subnet is *regional*, which is the one place this service parts
        // company with the networks table beside it: the name does not
        // identify a row (an auto-mode network calls every subnet `default`),
        // so a rule keyed on `name` alone would name 43 resources. `location`
        // carries the region and is what disambiguates them.
        //
        // `network` is here and not on the compute instance table because a
        // subnet row *carries* its parent network id; an instance's lives one
        // level down, on the detail payload (gap 2).
        ResourceServiceKind::VpcSubnets => &[
            ("name", JumpField::Name),
            ("location", JumpField::Location),
            ("network", JumpField::Network),
        ],
        // A firewall rule is global to its project, so `name` is the only
        // field that *identifies* one — the networks shape, not the subnets
        // one. Two rules cannot share a name even across networks. `network`
        // is offered beside it as a jump key rather than an identity: it names
        // the network the rule is attached to, which is a different resource.
        ResourceServiceKind::FirewallRules => {
            &[("name", JumpField::Name), ("network", JumpField::Network)]
        }
        // A subscription is global and its id is unique within the project, so
        // `name` is the only field a rule can key on — the topics shape.
        ResourceServiceKind::PubSubSubscriptions => &[("name", JumpField::Name)],
        // A dataset id is unique within its project, and the dataset carries a
        // real location of its own — a multi-region or a region — so both are
        // fields a rule can key on. The GKE and Cloud Storage shape.
        ResourceServiceKind::BigQueryDatasets => {
            &[("name", JumpField::Name), ("location", JumpField::Location)]
        }
        ResourceServiceKind::IamServiceAccounts => &[("name", JumpField::Name)],
        // A policy binding is global: the member is the whole identity a rule
        // can key on, so `name` is the only field — the Secret Manager shape.
        ResourceServiceKind::IamPolicy => &[("name", JumpField::Name)],
    }
}

/// The field a service keys its zone/region/location by — what a rule's
/// `location` template selects on, and `None` for a service that has no such
/// field. Secret Manager is that case: a secret is global, so a `{location}`
/// could only ever render nothing.
///
/// The option is what lets [`JumpRules::from_file`] say so instead of silently
/// accepting a template that could never resolve;
/// `a_location_template_is_rejected_for_a_service_with_no_location` pins that
/// it does.
pub fn location_field(kind: ResourceServiceKind) -> Option<JumpField> {
    fields_for(kind).iter().find_map(|(_, field)| {
        matches!(
            field,
            JumpField::Zone | JumpField::Region | JumpField::Location
        )
        .then_some(*field)
    })
}

/// What a rule narrows its destination list to, when it narrows one.
///
/// Deliberately *not* a third [`JumpTarget`] variant. The destination really is
/// the firewall rules list — `to = "firewall-rules"` — and what is new is that
/// the landing arrives narrowed. Making it a target variant would have put a
/// filtering concept into a type whose whole job is "which view", and every
/// existing match on `JumpTarget` would have had to answer a question about
/// filtering it has no opinion on.
///
/// It is also why a narrowing rule ignores `name` and `location`: the set it
/// lands on is computed from the source resource, not rendered from templates,
/// so there is no single row for a template to name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JumpSelect {
    /// The firewall rules that target the source instance — by network, by
    /// target tag, or by target service account, and including every rule that
    /// names no target at all, since those apply to the whole network.
    ///
    /// Legal only from `compute` to `firewall-rules`; the pairing is checked
    /// at load, because a `select` naming a computation the destination cannot
    /// perform is a rule that would resolve and then narrow nothing.
    FirewallRulesTargetingInstance,
}

impl JumpSelect {
    /// How the config file spells it.
    pub fn config_key(self) -> &'static str {
        match self {
            Self::FirewallRulesTargetingInstance => "targets-this-instance",
        }
    }

    /// The `(from, to)` pair this computation is defined for.
    ///
    /// One table rather than a check written into `validate`, so the answer to
    /// "which rules may carry this" is next to the variant it is about.
    pub fn defined_for(self) -> (ResourceServiceKind, ResourceServiceKind) {
        match self {
            Self::FirewallRulesTargetingInstance => (
                ResourceServiceKind::ComputeInstances,
                ResourceServiceKind::FirewallRules,
            ),
        }
    }

    /// The select a config file's `select` names, or `None` for a spelling that
    /// names no computation.
    pub fn from_config_key(raw: &str) -> Option<Self> {
        let wanted = raw.trim().to_ascii_lowercase();
        [Self::FirewallRulesTargetingInstance]
            .into_iter()
            .find(|select| select.config_key() == wanted)
    }
}

/// One configured jump: which service offers it, where it lands, and how the
/// target resource is named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpRule {
    pub from: ResourceServiceKind,
    pub to: JumpTarget,
    /// Selects the target key's name; `"{name}"` unless configured otherwise.
    pub name: Template,
    /// Selects the target key's zone/region/location. `None` means the source
    /// row's own location is used unchanged.
    pub location: Option<Template>,
    /// The picker's wording. `None` keeps the built-in `"<tab> of <name>"`.
    pub label: Option<Template>,
    /// A computed narrowing applied to the destination list.
    /// `None` — almost always — is a jump that lands on a whole list or a
    /// detail pane.
    pub select: Option<JumpSelect>,
}

/// The rule set the jump picker is derived from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpRules {
    rules: Vec<JumpRule>,
}

impl JumpRules {
    /// The built-in jumps, used when the file has no `[[jumps]]` array.
    ///
    /// These are the original fixed jumps expressed as rules of the same type
    /// the file produces, in tab-bar order, so there is one code path rather
    /// than a default branch that rots unexercised: an unconfigured user's
    /// picker is the rule engine's output, not a bypass of it.
    pub fn defaults() -> Self {
        use crate::app::state::resources::{
            CloudRunDetailTab, CloudSqlDetailTab, ComputeDetailTab,
        };

        let rule = |from, to, location: &str| JumpRule {
            from,
            to: JumpTarget::Detail(to),
            // The default templates are the only ones not written by a user,
            // so they are parsed against the same vocabulary a user's are —
            // an unparseable default would be a bug, not a config problem.
            name: Template::parse("{name}", from).expect("built-in name template"),
            location: Some(Template::parse(location, from).expect("built-in location template")),
            label: None,
            select: None,
        };

        // The list-target shape: keyed on the row's own `{network}` field, and
        // with no `location` template at all, since the destination keys its
        // rows by name alone.
        let to_networks = |from| JumpRule {
            from,
            to: JumpTarget::List(ResourceServiceKind::VpcNetworks),
            name: Template::parse("{network}", from).expect("built-in name template"),
            location: None,
            label: None,
            select: None,
        };

        let compute = ResourceServiceKind::ComputeInstances;
        let cloud_sql = ResourceServiceKind::CloudSql;
        let cloud_run = ResourceServiceKind::CloudRunServices;
        Self {
            rules: vec![
                rule(
                    compute,
                    DetailTab::Compute(ComputeDetailTab::Network),
                    "{zone}",
                ),
                rule(
                    compute,
                    DetailTab::Compute(ComputeDetailTab::Disks),
                    "{zone}",
                ),
                rule(
                    cloud_sql,
                    DetailTab::CloudSql(CloudSqlDetailTab::Replicas),
                    "{region}",
                ),
                rule(
                    cloud_run,
                    DetailTab::CloudRun(CloudRunDetailTab::Revisions),
                    "{region}",
                ),
                // The two list-target defaults. Both land on the VPC
                // networks table with the cursor on the parent network, which
                // is the question a subnet row and a firewall rule row leave
                // an operator with: *which* network is this attached to, and
                // what else is on it.
                //
                // No `location`: a network is global, so its row identity is
                // the name alone and `keys_rows_by_location` is false for it.
                // No default the other way — networks to subnets — because
                // that is "every subnet *of* this network", a filtered list,
                // which is a separate gap and not this seam.
                to_networks(ResourceServiceKind::VpcSubnets),
                to_networks(ResourceServiceKind::FirewallRules),
                // The instance's own network, which is what the VPC slice
                // asked for as "instance detail links into its
                // network". It resolves only with the pane fetched, because
                // `{network}` is detail-backed — so it appears in the picker
                // from the detail view and not from the list, which is exactly
                // the wording of the request.
                //
                // There is deliberately no matching instance-to-*subnet* rule.
                // A subnet is keyed by name and region, an instance offers
                // `{zone}`, and a zone is not a region. The region is in the
                // subnetwork self-link, which the parser discards down to the
                // tail segment, so keying one needs a parser change this slice
                // does not make. `{subnetwork}` is still offered as a field —
                // it names the subnet in a label perfectly well.
                to_networks(ResourceServiceKind::ComputeInstances),
                // The subnet jump, and the last piece of what the VPC slice
                // asked for. Keyed on both halves of a subnet's identity, because
                // `{subnetwork}` alone names one row per region: this is the
                // rule the region was kept in the parser for.
                JumpRule {
                    from: ResourceServiceKind::ComputeInstances,
                    to: JumpTarget::List(ResourceServiceKind::VpcSubnets),
                    name: Template::parse("{subnetwork}", ResourceServiceKind::ComputeInstances)
                        .expect("built-in name template"),
                    location: Some(
                        Template::parse(
                            "{subnetwork_region}",
                            ResourceServiceKind::ComputeInstances,
                        )
                        .expect("built-in location template"),
                    ),
                    label: None,
                    select: None,
                },
                // The firewall jump, and the last of the three destinations
                // the VPC slice asked for. It carries no `name` or `location` of its
                // own: the set is computed from the instance, so the built-in
                // `"{name}"` below is parsed and then never rendered.
                JumpRule {
                    from: ResourceServiceKind::ComputeInstances,
                    to: JumpTarget::List(ResourceServiceKind::FirewallRules),
                    name: Template::parse("{name}", ResourceServiceKind::ComputeInstances)
                        .expect("built-in name template"),
                    location: None,
                    label: None,
                    select: Some(JumpSelect::FirewallRulesTargetingInstance),
                },
            ],
        }
    }

    /// Parse and validate the raw `[[jumps]]` section. Every problem drops the
    /// offending rule and pushes one warning naming its index; the other rules
    /// are untouched.
    ///
    /// A present-but-entirely-invalid array yields an *empty* rule set rather
    /// than the defaults. The user asked for a custom set, so silently
    /// restoring the built-in jumps would offer entries no line of their file
    /// describes; every rejected rule has already said why it is gone.
    ///
    /// Takes the whole section rather than its entries so that a section that
    /// is not an array — `[jumps]` written for `[[jumps]]` — is one more
    /// warning here instead of a deserialization failure that would discard
    /// the user's whole file.
    pub fn from_file(raw: &toml::Value, warnings: &mut Vec<String>) -> Self {
        let Some(entries) = raw.as_array() else {
            warnings.push(format!(
                "jumps: expected an array of jump rules ([[jumps]]), found {}; \
                 ignoring every jump rule",
                raw.type_str()
            ));
            return Self { rules: Vec::new() };
        };

        let mut rules = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            match validate(entry) {
                Ok(rule) => rules.push(rule),
                Err(problem) => warnings.push(format!("jumps[{index}]: {problem}; {FALLBACK}")),
            }
        }
        Self { rules }
    }

    /// Every rule offered from `kind`, in configured order — which is the
    /// order the picker lists them in.
    pub fn for_service(&self, kind: ResourceServiceKind) -> impl Iterator<Item = &JumpRule> {
        self.rules.iter().filter(move |rule| rule.from == kind)
    }

    /// Every rule, in configured order.
    pub fn rules(&self) -> &[JumpRule] {
        &self.rules
    }
}

/// One piece of a parsed template.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Field(JumpField),
    /// A value the operator supplies rather than the row: a plugin input, named
    /// by the `[[plugins.inputs]]` entry that declared it. A `String` rather
    /// than an index into that array, so a template stays readable on its own
    /// and cannot be paired with the wrong plugin's inputs.
    Input(String),
}

/// A `{field}` template, parsed into segments at load time.
///
/// Parsing up front is what makes rendering *structurally* single-pass: there
/// are no braces left to interpret at render time, so a substituted value
/// containing `{` or `}` is copied out literally and can never trigger a second
/// expansion. A recursive renderer would make a resource whose name happens to
/// contain `{zone}` behave differently from every other resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    segments: Vec<Segment>,
}

impl Template {
    /// Parse `raw` against the fields `kind` offers. `Err` carries the problem
    /// clause of the warning the caller builds.
    pub fn parse(raw: &str, kind: ResourceServiceKind) -> Result<Self, String> {
        Self::parse_with_inputs(raw, kind, &[])
    }

    /// Parse `raw` against the fields `kind` offers *plus* `inputs` — the names
    /// of the values a caller collects rather than reads off the row.
    ///
    /// Only [`crate::config::plugins`] has any: a plugin's `[[plugins.inputs]]`
    /// widen that plugin's own templates and nothing else's, which is why this
    /// is a second door rather than a wider vocabulary in [`fields_for`]. A jump
    /// rule naming a plugin's input still gets the unknown-field warning.
    ///
    /// A name in `inputs` that is also one of `kind`'s fields is the caller's
    /// problem to refuse before it gets here — the row's `{zone}` and an input
    /// called `zone` cannot both win, and this function would silently give it
    /// to the row.
    pub fn parse_with_inputs(
        raw: &str,
        kind: ResourceServiceKind,
        inputs: &[&str],
    ) -> Result<Self, String> {
        let known = fields_for(kind);
        let mut segments = Vec::new();
        let mut literal = String::new();
        let mut rest = raw;

        while let Some(open) = rest.find('{') {
            let (before, after_open) = rest.split_at(open);
            if before.contains('}') {
                return Err("unbalanced \"}\"".to_string());
            }
            literal.push_str(before);
            let body = &after_open[1..];
            let close = body
                .find('}')
                .ok_or_else(|| "unbalanced \"{\"".to_string())?;
            let (name, after_close) = body.split_at(close);
            if name.contains('{') {
                return Err("unbalanced \"{\"".to_string());
            }
            // The row's fields are consulted first, so a name in both
            // vocabularies resolves to the row. That is never reached in
            // practice — the plugin layer refuses the collision — and choosing
            // the row here rather than the input keeps the fallthrough the same
            // shape the message below describes.
            let segment = known
                .iter()
                .find_map(|(key, field)| (*key == name).then_some(Segment::Field(*field)))
                .or_else(|| {
                    inputs
                        .contains(&name)
                        .then(|| Segment::Input(name.to_string()))
                })
                .ok_or_else(|| format!("unknown field {name:?} for {}", config_service_id(kind)))?;
            if !literal.is_empty() {
                segments.push(Segment::Literal(std::mem::take(&mut literal)));
            }
            segments.push(segment);
            rest = &after_close[1..];
        }

        if rest.contains('}') {
            return Err("unbalanced \"}\"".to_string());
        }
        literal.push_str(rest);
        if !literal.is_empty() {
            segments.push(Segment::Literal(literal));
        }

        Ok(Self { segments })
    }

    /// Whether the first thing this template emits is a substituted value
    /// rather than text the operator wrote.
    ///
    /// A substituted value is a row field *or* a collected input — the name
    /// says field because that is all a template could hold when it was
    /// written, and renaming it would churn every call site for no behaviour.
    ///
    /// This is the distinction the plugin leading-dash rule turns on
    /// ([`crate::app::state::plugins`]): `{name}` rendering to
    /// `--project=other` is a row value being read as a flag, and `{size}`
    /// collected as `-x` is typed text being read as one, while
    /// `--zone={zone}` is an operator-declared flag and must not be refused.
    /// Covering the input is deliberate: text the operator typed reaches gcloud
    /// exactly as a row value does, so it earns the same rule.
    ///
    /// Exposed rather than inferred from the rendered string, because after
    /// rendering a literal and a substitution are indistinguishable.
    pub fn starts_with_field(&self) -> bool {
        matches!(
            self.segments.first(),
            Some(Segment::Field(_) | Segment::Input(_))
        )
    }

    /// Whether this template names `name` as an input, anywhere.
    ///
    /// Anywhere, and not just first: `--size {a} --tier {b}` holds two
    /// `Segment::Input`s, because `parse_with_inputs` pushes one segment per
    /// `{...}`, and prompting only for `a` would compose an argv the operator
    /// was never asked to complete. Which inputs to prompt for is decided over
    /// all of them (`app::state::plugins::prompted_inputs`). There used to be a
    /// second question here — which unpromptable input a refusal names — and
    /// the prompt work removed it by leaving no unpromptable kind.
    pub fn names_input(&self, name: &str) -> bool {
        self.segments
            .iter()
            .any(|segment| matches!(segment, Segment::Input(found) if found == name))
    }

    /// This template's text, when nothing in it is substituted.
    ///
    /// `None` for a template holding any `Field` or `Input`, which is what lets
    /// `app::state::plugins::declared_effect` match a command path
    /// positionally: an element whose value is only known at runtime must not
    /// match a literal table entry, and answering `Some` with the literal parts
    /// concatenated would let `{verb}` match on its surroundings.
    pub fn literal(&self) -> Option<&str> {
        match self.segments.as_slice() {
            [Segment::Literal(text)] => Some(text.as_str()),
            _ => None,
        }
    }

    /// The rendered string, or `None` when any field or input it names is
    /// missing or empty. An identity built from a blank is not an identity, so
    /// the caller drops the offer rather than showing a broken one.
    ///
    /// Two lookups rather than one map: a field is read off the selected row
    /// and an input off what the operator was asked for, and they are answered
    /// by different layers. Merging them would let a row field silently satisfy
    /// a template that asked for a value nobody typed.
    pub fn render(
        &self,
        fields: impl Fn(JumpField) -> Option<String>,
        values: impl Fn(&str) -> Option<String>,
    ) -> Option<String> {
        let mut out = String::new();
        for segment in &self.segments {
            match segment {
                Segment::Literal(text) => out.push_str(text),
                Segment::Field(field) => {
                    let value = fields(*field)?;
                    if value.is_empty() {
                        return None;
                    }
                    out.push_str(&value);
                }
                Segment::Input(name) => {
                    let value = values(name)?;
                    if value.is_empty() {
                        return None;
                    }
                    out.push_str(&value);
                }
            }
        }
        Some(out)
    }
}

/// Validate one raw rule. `Err` carries the problem clause of the warning; the
/// first problem found wins, since the rule is dropped whole either way. Both
/// the rule's shape (is it a table, is `to` a string) and its content are
/// checked here, so neither class of problem can reach beyond this rule.
fn validate(entry: &toml::Value) -> Result<JumpRule, String> {
    let table = entry.as_table().ok_or_else(|| {
        format!(
            "expected a table of jump settings, found {}",
            entry.type_str()
        )
    })?;

    if let Some(key) = table.keys().find(|key| !SETTINGS.contains(&key.as_str())) {
        return Err(format!("unknown setting {key:?}"));
    }

    // Shape first, then meaning: every key is read as the type it must have
    // before any of them is interpreted.
    let raw_from = setting_str("from", table.get("from"))?.ok_or("missing from")?;
    let raw_to = setting_str("to", table.get("to"))?.ok_or("missing to")?;
    let raw_name = setting_str("name", table.get("name"))?;
    let raw_location = setting_str("location", table.get("location"))?;
    let raw_label = setting_str("label", table.get("label"))?;
    let raw_select = setting_str("select", table.get("select"))?;

    let from = resolve_service_kind(raw_from)
        .ok_or_else(|| format!("unknown from service {raw_from:?}"))?;
    let to = JumpTarget::from_config_key(raw_to).ok_or_else(|| destination_problem(raw_to))?;
    // Resolvable but unlandable: the destination identifies a row by something
    // no template can render. Said here rather than left to fail silently at
    // `J`, where a rule that offered a row and then matched nothing would read
    // as a missing resource.
    if !to.is_landable() {
        return Err(format!(
            "{} identifies a row by more than a name and a location, \
             so to {raw_to:?} is not a destination",
            to.service().label()
        ));
    }

    // The source's field table is the template vocabulary, so `from` has to be
    // resolved before any template can be parsed.
    let template = |key: &str, raw: &str| {
        // An empty template is well-formed but renders to an empty string
        // rather than to "unresolvable", so it would reach the picker as a
        // blank identity or a blank row. Rejected here, where the warning can
        // still name the key the user has to fix.
        if raw.is_empty() {
            return Err(format!("{key} template is empty"));
        }
        Template::parse(raw, from).map_err(|problem| format!("{key} template {raw:?}: {problem}"))
    };

    let name = match raw_name {
        Some(raw) => template("name", raw)?,
        // The omitted-`name` fallback is parsed against `from` like any other
        // template. Every service has a `name` field today, so this cannot
        // fail — and it is a `Result` rather than an `expect` so that the day
        // one does not, a plain config file is one more warning instead of a
        // panic at load. That is the difference between the rule "no config
        // problem is fatal" being structural and it being test-enforced.
        None => Template::parse("{name}", from).map_err(|problem| {
            format!(
                "no name template, and the built-in \"{{name}}\" does not apply to {}: {problem}",
                config_service_id(from)
            )
        })?,
    };
    let location = match raw_location {
        Some(raw) => {
            if location_field(from).is_none() {
                return Err(format!(
                    "location template on {}, which has no location field",
                    config_service_id(from)
                ));
            }
            // The *destination* half of the same objection. A
            // service whose rows are identified by name alone — a VPC network,
            // a secret, a topic — has an empty location in every row identity,
            // so a rendered one could only ever fail to match. Refusing it
            // here means the user reads why at load; letting it through would
            // surface at `J` as "that row is no longer available", which is a
            // claim about their project rather than about their file.
            if !to.keys_rows_by_location() {
                return Err(format!(
                    "location template on a jump to {}, which keys its rows by name alone",
                    to.config_key()
                ));
            }
            Some(template("location", raw)?)
        }
        None => None,
    };
    let label = raw_label.map(|raw| template("label", raw)).transpose()?;

    // A `select` names a computation, not a template, so it is
    // resolved against its own vocabulary and checked against the pair it is
    // defined for. Both halves matter: a select on the wrong `from` has no
    // instance to compute from, and one on the wrong `to` would narrow a list
    // whose rows the computation says nothing about — either way a rule that
    // resolves and then narrows nothing, which reads as an empty table rather
    // than as a mistake.
    let select = match raw_select {
        Some(raw) => {
            let select = JumpSelect::from_config_key(raw)
                .ok_or_else(|| format!("unknown select {raw:?}"))?;
            let (wants_from, wants_to) = select.defined_for();
            if from != wants_from || to != JumpTarget::List(wants_to) {
                return Err(format!(
                    "select {raw:?} is only defined from {} to {}",
                    config_service_id(wants_from),
                    config_service_id(wants_to)
                ));
            }
            Some(select)
        }
        None => None,
    };

    Ok(JumpRule {
        from,
        to,
        name,
        location,
        label,
        select,
    })
}

/// Why a `to` names no tab. A service the vocabulary resolves is called out by
/// name because it is one a user can reasonably expect to be a destination: it
/// is in the sidebar like the others, but its detail view is bespoke or absent
/// and it owns no `DetailTab`, so no key can ever name one. Which services
/// those are is read off `DetailTab::ALL` rather than listed here, so a service
/// gaining or losing a detail pane cannot leave this message behind.
fn destination_problem(raw: &str) -> String {
    let service = raw.split(':').next().unwrap_or(raw);
    match resolve_service_kind(service) {
        Some(kind) if !DetailTab::ALL.iter().any(|tab| tab.service() == kind) => {
            format!(
                "{} has no detail tabs, so to {raw:?} is not a destination; \
                 drop the tab half to land on its list",
                kind.label()
            )
        }
        _ => format!("unknown destination tab {raw:?}"),
    }
}

/// One optional setting that must be a string, or the problem clause naming
/// the type the user wrote instead.
fn setting_str<'a>(key: &str, value: Option<&'a toml::Value>) -> Result<Option<&'a str>, String> {
    value
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| format!("{key} must be a string, found {}", value.type_str()))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPUTE: ResourceServiceKind = ResourceServiceKind::ComputeInstances;

    /// The `(to, name, location, label)` of every default rule for `kind`, in
    /// order, with templates as the strings a user would have written.
    /// A default rule flattened to the parts these pins compare: where it
    /// lands, its two templates, whether it carries a label, and whether it
    /// narrows.
    type DefaultRule = (String, Template, Option<Template>, bool, Option<JumpSelect>);

    fn defaults_for(kind: ResourceServiceKind) -> Vec<DefaultRule> {
        JumpRules::defaults()
            .for_service(kind)
            .map(|rule| {
                (
                    rule.to.config_key(),
                    rule.name.clone(),
                    rule.location.clone(),
                    rule.label.is_some(),
                    rule.select,
                )
            })
            .collect()
    }

    #[test]
    fn the_defaults_are_the_fixed_jumps_in_tab_bar_order() {
        assert_eq!(
            defaults_for(COMPUTE),
            vec![
                (
                    "compute:network".to_string(),
                    Template::parse("{name}", COMPUTE).unwrap(),
                    Some(Template::parse("{zone}", COMPUTE).unwrap()),
                    false,
                    None,
                ),
                (
                    "compute:disks".to_string(),
                    Template::parse("{name}", COMPUTE).unwrap(),
                    Some(Template::parse("{zone}", COMPUTE).unwrap()),
                    false,
                    None,
                ),
                // Last, and after the two tab rules, because
                // it is the one compute default that does not resolve from the
                // list: `{network}` is detail-backed, so this row appears in
                // the picker only once the pane is fetched. It carries no
                // `location` — a VPC network is global.
                (
                    "vpc-networks".to_string(),
                    Template::parse("{network}", COMPUTE).unwrap(),
                    None,
                    false,
                    None,
                ),
                // Both halves, unlike the network rule above: a
                // subnet is identified by name *and* region, so a rule with no
                // `location` would name one row per region and land on
                // whichever sorted first.
                (
                    "vpc-subnets".to_string(),
                    Template::parse("{subnetwork}", COMPUTE).unwrap(),
                    Some(Template::parse("{subnetwork_region}", COMPUTE).unwrap()),
                    false,
                    None,
                ),
                // The one default that narrows: no `location`, and a
                // `name` that is parsed and never rendered, because the set it
                // lands on is computed rather than named.
                (
                    "firewall-rules".to_string(),
                    Template::parse("{name}", COMPUTE).unwrap(),
                    None,
                    false,
                    Some(JumpSelect::FirewallRulesTargetingInstance),
                ),
            ]
        );
    }

    #[test]
    fn cloud_sql_and_cloud_run_each_default_to_one_rule() {
        let sql = ResourceServiceKind::CloudSql;
        assert_eq!(
            defaults_for(sql),
            vec![(
                "cloudsql:replicas".to_string(),
                Template::parse("{name}", sql).unwrap(),
                Some(Template::parse("{region}", sql).unwrap()),
                false,
                None,
            )]
        );

        let run = ResourceServiceKind::CloudRunServices;
        assert_eq!(
            defaults_for(run),
            vec![(
                "cloudrun:revisions".to_string(),
                Template::parse("{name}", run).unwrap(),
                Some(Template::parse("{region}", run).unwrap()),
                false,
                None,
            )]
        );
    }

    #[test]
    fn gke_and_cloud_storage_have_no_default_rules() {
        assert!(defaults_for(ResourceServiceKind::GkeClusters).is_empty());
        assert!(defaults_for(ResourceServiceKind::CloudStorage).is_empty());
    }

    /// Parse a `[[jumps]]` body exactly as `apply_file` does.
    fn parse(body: &str) -> (JumpRules, Vec<String>) {
        #[derive(serde::Deserialize)]
        struct Doc {
            #[serde(default = "empty_array")]
            jumps: toml::Value,
        }
        fn empty_array() -> toml::Value {
            toml::Value::Array(Vec::new())
        }
        let doc: Doc = toml::from_str(body).expect("test fixture must be valid TOML");
        let mut warnings = Vec::new();
        (JumpRules::from_file(&doc.jumps, &mut warnings), warnings)
    }

    #[test]
    fn a_valid_rule_replaces_the_defaults() {
        let (rules, warnings) = parse(
            r#"
[[jumps]]
from = "gce"
to = "compute:metadata"
name = "{name}-mirror"
location = "{zone}"
label = "Metadata of {name}"
"#,
        );

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let rule = match rules.rules() {
            [only] => only,
            other => panic!("expected exactly one rule, got {other:?}"),
        };
        assert_eq!(rule.from, COMPUTE);
        assert_eq!(rule.to.config_key(), "compute:metadata");
        assert_eq!(
            rule.name,
            Template::parse("{name}-mirror", COMPUTE).unwrap()
        );
        assert_eq!(
            rule.location,
            Some(Template::parse("{zone}", COMPUTE).unwrap())
        );
        assert_eq!(
            rule.label,
            Some(Template::parse("Metadata of {name}", COMPUTE).unwrap())
        );
    }

    #[test]
    fn a_rule_with_only_from_and_to_takes_the_built_in_name_and_wording() {
        let (rules, warnings) = parse("[[jumps]]\nfrom = \"compute\"\nto = \"compute:disks\"\n");

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let rule = &rules.rules()[0];
        assert_eq!(rule.name, Template::parse("{name}", COMPUTE).unwrap());
        assert_eq!(rule.location, None);
        assert_eq!(rule.label, None);
    }

    /// Every rule below is dropped whole, with exactly one warning, while the
    /// valid rule beside it survives.
    fn assert_rejected(rule: &str, expected_warning: &str) {
        let body = format!("{rule}\n[[jumps]]\nfrom = \"gke\"\nto = \"gke:addons\"\n");
        let (rules, warnings) = parse(&body);

        assert_eq!(warnings, vec![expected_warning.to_string()]);
        assert_eq!(rules.rules().len(), 1, "the valid rule must survive");
        assert_eq!(rules.rules()[0].to.config_key(), "gke:addons");
    }

    #[test]
    fn a_rule_that_is_not_a_table_is_rejected() {
        let (rules, warnings) =
            parse("jumps = [\"compute\", { from = \"gke\", to = \"gke:addons\" }]\n");

        assert_eq!(
            warnings,
            vec![
                "jumps[0]: expected a table of jump settings, found string; ignoring this rule"
                    .to_string()
            ]
        );
        assert_eq!(rules.rules().len(), 1, "the valid rule must survive");
        assert_eq!(rules.rules()[0].to.config_key(), "gke:addons");
    }

    #[test]
    fn a_missing_from_is_rejected() {
        assert_rejected(
            "[[jumps]]\nto = \"compute:disks\"\n",
            "jumps[0]: missing from; ignoring this rule",
        );
    }

    #[test]
    fn an_unknown_from_is_rejected() {
        assert_rejected(
            "[[jumps]]\nfrom = \"zzz\"\nto = \"compute:disks\"\n",
            "jumps[0]: unknown from service \"zzz\"; ignoring this rule",
        );
    }

    #[test]
    fn a_missing_to_is_rejected() {
        assert_rejected(
            "[[jumps]]\nfrom = \"compute\"\n",
            "jumps[0]: missing to; ignoring this rule",
        );
    }

    #[test]
    fn a_non_string_to_is_rejected() {
        assert_rejected(
            "[[jumps]]\nfrom = \"compute\"\nto = 7\n",
            "jumps[0]: to must be a string, found integer; ignoring this rule",
        );
    }

    /// `from` and `to` name services out of the same vocabulary. A user who
    /// writes `from = "gce"` on one line and `to = "gce:disks"` on the next is
    /// writing one spelling, and the rule has to mean what it says.
    #[test]
    fn a_destination_service_takes_the_same_spellings_as_from() {
        let (lenient, warnings) = parse("[[jumps]]\nfrom = \"gce\"\nto = \"gce:disks\"\n");
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

        let (canonical, _) = parse("[[jumps]]\nfrom = \"compute\"\nto = \"compute:disks\"\n");
        assert_eq!(lenient.rules(), canonical.rules());
        // And the canonical spelling is still what gets printed back.
        assert_eq!(lenient.rules()[0].to.config_key(), "compute:disks");
    }

    #[test]
    fn an_unknown_tab_on_a_known_service_is_still_rejected_by_name() {
        assert_rejected(
            "[[jumps]]\nfrom = \"compute\"\nto = \"gce:zzz\"\n",
            "jumps[0]: unknown destination tab \"gce:zzz\"; ignoring this rule",
        );
    }

    #[test]
    fn an_unknown_to_is_rejected() {
        assert_rejected(
            "[[jumps]]\nfrom = \"compute\"\nto = \"compute:zzz\"\n",
            "jumps[0]: unknown destination tab \"compute:zzz\"; ignoring this rule",
        );
    }

    #[test]
    fn a_cloud_storage_destination_is_rejected_by_name() {
        // Storage is in the sidebar like every other service, so "why not?"
        // is the question the warning has to answer.
        assert_rejected(
            "[[jumps]]\nfrom = \"compute\"\nto = \"storage:objects\"\n",
            "jumps[0]: Cloud Storage has no detail tabs, so to \"storage:objects\" is not a \
             destination; drop the tab half to land on its list; ignoring this rule",
        );
    }

    #[test]
    fn an_unknown_key_in_the_rule_is_rejected() {
        // A typo'd `labl` would otherwise change nothing and say nothing.
        assert_rejected(
            "[[jumps]]\nfrom = \"compute\"\nto = \"compute:disks\"\nlabl = \"x\"\n",
            "jumps[0]: unknown setting \"labl\"; ignoring this rule",
        );
    }

    #[test]
    fn a_non_string_template_is_rejected() {
        assert_rejected(
            "[[jumps]]\nfrom = \"compute\"\nto = \"compute:disks\"\nname = 7\n",
            "jumps[0]: name must be a string, found integer; ignoring this rule",
        );
    }

    #[test]
    fn a_template_naming_a_field_the_source_does_not_have_is_rejected() {
        assert_rejected(
            "[[jumps]]\nfrom = \"compute\"\nto = \"compute:disks\"\nname = \"{engine}\"\n",
            "jumps[0]: name template \"{engine}\": unknown field \"engine\" for compute; \
             ignoring this rule",
        );
    }

    #[test]
    fn an_unbalanced_brace_is_rejected() {
        assert_rejected(
            "[[jumps]]\nfrom = \"compute\"\nto = \"compute:disks\"\nlabel = \"Disks of {name\"\n",
            "jumps[0]: label template \"Disks of {name\": unbalanced \"{\"; ignoring this rule",
        );
    }

    #[test]
    fn an_empty_template_is_rejected_for_every_key_that_takes_one() {
        // An empty template renders to an empty string rather than to
        // "unresolvable", so nothing downstream would catch a blank label. It
        // is rejected here, where the warning can name the key to fix.
        for key in ["name", "location", "label"] {
            assert_rejected(
                &format!("[[jumps]]\nfrom = \"compute\"\nto = \"compute:disks\"\n{key} = \"\"\n"),
                &format!("jumps[0]: {key} template is empty; ignoring this rule"),
            );
        }
    }

    #[test]
    fn a_later_rules_index_is_the_one_reported() {
        let (rules, warnings) = parse(
            "[[jumps]]\nfrom = \"gke\"\nto = \"gke:addons\"\n\
             \n[[jumps]]\nfrom = \"zzz\"\nto = \"compute:disks\"\n",
        );

        assert_eq!(
            warnings,
            vec!["jumps[1]: unknown from service \"zzz\"; ignoring this rule".to_string()]
        );
        assert_eq!(rules.rules().len(), 1);
    }

    #[test]
    fn an_array_whose_every_rule_is_rejected_configures_no_jumps() {
        // Not the defaults: the user asked for a custom set, and each warning
        // has already said why theirs is empty.
        let (rules, warnings) = parse(
            "[[jumps]]\nfrom = \"zzz\"\nto = \"compute:disks\"\n\
             \n[[jumps]]\nfrom = \"compute\"\nto = \"compute:zzz\"\n",
        );

        assert_eq!(warnings.len(), 2, "one per rule: {warnings:?}");
        assert!(warnings[0].starts_with("jumps[0]:"));
        assert!(warnings[1].starts_with("jumps[1]:"));
        assert!(rules.rules().is_empty());
    }

    #[test]
    fn every_service_has_the_name_field_the_rules_assume() {
        for kind in ResourceServiceKind::ALL {
            // A rule that omits `name` falls back to `"{name}"`. A service
            // without a `name` field would make that fallback unparseable, so
            // the plainest possible rule — `from` and `to` alone — would warn
            // and be dropped for a reason nothing in the user's file names.
            assert!(
                fields_for(kind).contains(&("name", JumpField::Name)),
                "{kind:?} has no name field"
            );
        }
    }

    /// Every tabless service's own refusal message, pinned to its wording.
    ///
    /// `destination_problem`'s arm survived mutation: deleting it falls through
    /// to the generic "unknown destination tab", which tells a user their
    /// spelling is wrong when in fact the service exists and simply has no
    /// detail tabs to land on.
    ///
    /// The cases are derived from `ResourceServiceKind::ALL` filtered by owning
    /// no `DetailTab`, not hand-listed: a hand-written list of services is
    /// exactly where a newly added tabless service goes unnoticed, which is how
    /// Cloud Functions came to be missing from this branch in the first place.
    #[test]
    fn a_tabless_service_named_as_a_destination_is_told_why_not_that_it_is_unknown() {
        let tabless: Vec<ResourceServiceKind> = ResourceServiceKind::ALL
            .into_iter()
            .filter(|kind| !DetailTab::ALL.iter().any(|tab| tab.service() == *kind))
            .collect();
        assert!(
            !tabless.is_empty(),
            "no service is tabless, so this test asserts nothing"
        );

        for kind in tabless {
            let service = config_service_id(kind);
            let expected = format!("{} has no detail tabs", kind.label());
            let source = format!("[[jumps]]\nfrom = \"compute\"\nto = \"{service}:summary\"\n");
            let document: toml::Value = toml::from_str(&source).expect("valid TOML");
            let mut warnings = Vec::new();
            let rules = JumpRules::from_file(
                document.get("jumps").expect("a [[jumps]] array"),
                &mut warnings,
            );

            assert!(
                rules.rules().is_empty(),
                "{service:?} has no detail tabs, so it must not produce a rule"
            );
            assert!(
                warnings
                    .iter()
                    .any(|warning| warning.contains(expected.as_str())),
                "{service:?} must be told it has no detail tabs rather than that \
                 its destination is unknown, got {warnings:?}"
            );
            assert!(
                !warnings
                    .iter()
                    .any(|warning| warning.contains("unknown destination tab")),
                "{service:?} is a real service, so the generic unknown-tab \
                 wording would misdescribe it: {warnings:?}"
            );
        }
    }

    /// A location field is *not* universal, and this pins the consequence
    /// rather than the old assumption that it was.
    ///
    /// Every service used to be zonal or regional, so `location_field` was
    /// `Some` for all of them and the previous version of this test pinned
    /// that. Secret Manager is global — a secret has no location — which makes
    /// `None` reachable for the first time, and Pub/Sub topics are global for
    /// the same reason. The `Option` was always there for this case; what
    /// matters is that a `location` template on such a service is *rejected
    /// with a reason*, rather than accepted and left to render nothing.
    ///
    /// The VPC pair splits across this line and that is the point of keeping
    /// the list explicit: a subnet is regional, so it has a location, while a
    /// network and a firewall rule are both global — a rule lives in a
    /// network, not in a region, and the `network` field it carries is a
    /// parent rather than a place.
    #[test]
    fn a_location_template_is_rejected_for_a_service_with_no_location() {
        let locationless: Vec<ResourceServiceKind> = ResourceServiceKind::ALL
            .into_iter()
            .filter(|kind| location_field(*kind).is_none())
            .collect();
        assert_eq!(
            locationless,
            vec![
                ResourceServiceKind::SecretManagerSecrets,
                ResourceServiceKind::PubSubTopics,
                ResourceServiceKind::PubSubSubscriptions,
                ResourceServiceKind::IamServiceAccounts,
                ResourceServiceKind::IamPolicy,
                ResourceServiceKind::VpcNetworks,
                ResourceServiceKind::FirewallRules
            ],
            "when a service gains or loses a location field, this test and the              reasoning above have to be revisited together"
        );

        for kind in locationless {
            let source = format!(
                "[[jumps]]\nfrom = \"{}\"\nto = \"compute:network\"\nlocation = \"{{name}}\"\n",
                config_service_id(kind)
            );
            let document: toml::Value = toml::from_str(&source).expect("valid TOML");
            let mut warnings = Vec::new();
            let rules = JumpRules::from_file(
                document.get("jumps").expect("a [[jumps]] array"),
                &mut warnings,
            );
            assert!(
                rules.rules().is_empty(),
                "{kind:?} has no location field, so a location template must not \
                 produce a rule"
            );
            assert!(
                warnings
                    .iter()
                    .any(|warning| warning.contains("no location field")),
                "{kind:?} must be told why its location template was dropped, got \
                 {warnings:?}"
            );
        }
    }

    /// "No config problem is fatal" is the rule the whole module is built on,
    /// and `validate` is the only place a user's bytes are interpreted. As
    /// long as every failure there is a `Result`, that rule holds for inputs
    /// nobody thought to write a case for — which is the point, since the
    /// panic this replaced was on a path no config file can reach today and
    /// would have become reachable the day a service arrived without a `name`
    /// field. Asserted on the source because the property is the *absence* of
    /// a path, and an absence has no input that exercises it.
    ///
    /// `defaults()` is deliberately not covered: its templates are constants,
    /// not user input, so an unparseable one is a bug that should be loud.
    #[test]
    fn validating_a_rule_has_no_panicking_path() {
        let source = include_str!("jumps.rs");
        let start = source
            .find("\nfn validate(")
            .expect("validate is a free function in this module");
        let body = &source[start..];
        let end = body.find("\n}\n").expect("validate has a closing brace");

        for forbidden in ["expect(", "unwrap(", "panic!", "unreachable!"] {
            assert!(
                !body[..end].contains(forbidden),
                "validate must report {forbidden} as a warning instead"
            );
        }
    }

    #[test]
    fn a_template_substitutes_its_fields_and_keeps_its_literals() {
        let template = Template::parse("{name} in {zone}", COMPUTE).expect("valid template");

        let rendered = template.render(
            |field| match field {
                JumpField::Name => Some("web-01".to_string()),
                JumpField::Zone => Some("us-central1-a".to_string()),
                _ => None,
            },
            |_| None,
        );

        assert_eq!(rendered, Some("web-01 in us-central1-a".to_string()));
    }

    #[test]
    fn a_template_renders_nothing_when_a_field_is_missing_or_empty() {
        let template = Template::parse("{name}", COMPUTE).expect("valid template");

        assert_eq!(template.render(|_| None, |_| None), None);
        assert_eq!(template.render(|_| Some(String::new()), |_| None), None);
    }

    #[test]
    fn a_field_value_containing_braces_is_emitted_literally() {
        // Rendering walks segments parsed at load time, so a substituted value
        // is never scanned for further fields.
        let template = Template::parse("{name}", COMPUTE).expect("valid template");

        let rendered = template.render(|_| Some("{zone}".to_string()), |_| None);

        assert_eq!(rendered, Some("{zone}".to_string()));
    }

    #[test]
    fn parsing_rejects_an_unknown_field_and_an_unbalanced_brace() {
        assert_eq!(
            Template::parse("{zzz}", COMPUTE),
            Err("unknown field \"zzz\" for compute".to_string())
        );
        assert_eq!(
            Template::parse("{name", COMPUTE),
            Err("unbalanced \"{\"".to_string())
        );
        assert_eq!(
            Template::parse("name}", COMPUTE),
            Err("unbalanced \"}\"".to_string())
        );
    }

    #[test]
    fn a_template_knows_whether_its_first_segment_is_substituted() {
        assert!(
            Template::parse("{name}", COMPUTE)
                .unwrap()
                .starts_with_field()
        );
        assert!(
            Template::parse("{name}-suffix", COMPUTE)
                .unwrap()
                .starts_with_field()
        );
        assert!(
            !Template::parse("--zone={zone}", COMPUTE)
                .unwrap()
                .starts_with_field()
        );
        assert!(
            !Template::parse("literal", COMPUTE)
                .unwrap()
                .starts_with_field()
        );
    }

    #[test]
    fn a_field_is_only_known_where_its_service_has_it() {
        // `engine` is a Cloud SQL field; naming it in a compute rule is the
        // typo the per-service table exists to catch.
        assert!(Template::parse("{engine}", COMPUTE).is_err());
        assert!(Template::parse("{engine}", ResourceServiceKind::CloudSql).is_ok());
    }

    #[test]
    fn an_input_name_is_a_field_only_where_the_caller_declared_one() {
        // A plugin's inputs widen its own templates' vocabulary and nothing
        // else's: `Template::parse` is the jumps door and has no inputs to
        // admit, so a jump rule naming one still gets the unknown-field
        // warning.
        assert!(Template::parse("{disk_size}", COMPUTE).is_err());
        assert!(Template::parse_with_inputs("{disk_size}", COMPUTE, &[]).is_err());
        assert!(Template::parse_with_inputs("{disk_size}", COMPUTE, &["disk_size"]).is_ok());
    }

    #[test]
    fn render_substitutes_a_collected_input_value() {
        let template = Template::parse_with_inputs("--size {disk_size}", COMPUTE, &["disk_size"])
            .expect("template parses");

        let rendered = template.render(
            |_| None,
            |name| (name == "disk_size").then(|| "100".to_string()),
        );

        assert_eq!(rendered.as_deref(), Some("--size 100"));
    }

    #[test]
    fn render_interleaves_a_field_and_an_input_in_one_walk() {
        // Every other render test here answers one lookup and stubs the other,
        // so nothing pins that the two are woven into the same string in
        // segment order. A `render` that walked the fields and then the inputs,
        // or that let one lookup answer the other's segment, would pass all of
        // them and compose `web-01 --size` with the value in the wrong place.
        let template =
            Template::parse_with_inputs("{name} --size {disk_size}", COMPUTE, &["disk_size"])
                .expect("template parses");

        let rendered = template.render(
            |field| (field == JumpField::Name).then(|| "web-01".to_string()),
            |name| (name == "disk_size").then(|| "100".to_string()),
        );

        assert_eq!(rendered.as_deref(), Some("web-01 --size 100"));
    }

    #[test]
    fn render_refuses_an_input_with_no_collected_value() {
        let template = Template::parse_with_inputs("--size {disk_size}", COMPUTE, &["disk_size"])
            .expect("template parses");

        assert_eq!(template.render(|_| None, |_| None), None);
    }

    #[test]
    fn render_refuses_an_input_collected_as_empty() {
        let template = Template::parse_with_inputs("--size {disk_size}", COMPUTE, &["disk_size"])
            .expect("template parses");

        assert_eq!(
            template.render(|_| None, |_| Some(String::new())),
            None,
            "an empty input must refuse exactly as an empty field does"
        );
    }

    #[test]
    fn names_input_sees_every_input_a_template_names() {
        let template = Template::parse_with_inputs(
            "--size {disk_size} --tier {tier}",
            COMPUTE,
            &["disk_size", "tier"],
        )
        .expect("template parses");

        assert!(template.names_input("disk_size"));
        assert!(
            template.names_input("tier"),
            "a template holds one segment per {{...}}, so the second input must be \
             visible too"
        );
        assert!(!template.names_input("absent"));
    }

    /// The classifier in `app::state::plugins` matches literal command paths
    /// positionally, so it needs to know that an element is literal *and* what
    /// it says. A templated element must answer `None` rather than its literal
    /// prefix, or `{verb}` would match the table by its surrounding text.
    #[test]
    fn literal_answers_only_for_a_template_with_nothing_substituted() {
        let plain = Template::parse("describe", COMPUTE).expect("valid template");
        assert_eq!(plain.literal(), Some("describe"));

        let field = Template::parse("{name}", COMPUTE).expect("valid template");
        assert_eq!(field.literal(), None, "a row field is not literal text");

        let mixed = Template::parse("prefix-{name}", COMPUTE).expect("valid template");
        assert_eq!(
            mixed.literal(),
            None,
            "a template that is partly substituted is not a literal element"
        );

        let input =
            Template::parse_with_inputs("{verb}", COMPUTE, &["verb"]).expect("valid template");
        assert_eq!(input.literal(), None, "an input is runtime data");
    }

    #[test]
    fn an_input_is_a_substituted_first_segment_like_a_field() {
        // The leading-dash rule turns on this answer, and an input's collected
        // value is as much a substituted value as a row's is.
        assert!(
            Template::parse_with_inputs("{disk_size}", COMPUTE, &["disk_size"])
                .unwrap()
                .starts_with_field()
        );
    }
}
