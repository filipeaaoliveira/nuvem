//! Demo fixture command runner.
//!
//! `DemoCommandRunner` implements [`CommandRunner`] and answers every gcloud
//! and `bq` invocation with a canned, embedded JSON fixture instead of
//! shelling out to the real CLI. Because every `Gcloud*Service` is constructed
//! generically over `CommandRunner`, swapping this runner in lets the whole
//! app render against realistic data with no live GCP project, network access,
//! or `gcloud` binary required.
//!
//! It is no longer demo's whole backend. The native clients under
//! `services/sdk/` serve the listings and detail loads they cover, and in demo
//! a session builds that cache from `sdk/` next door rather than from
//! credentials, so those reads never become a command at all. What still
//! reaches this runner is everything with no native client behind it on
//! either backend: BigQuery, which goes to `bq`; FinOps budgets, for which
//! Cloud Billing publishes no Rust client (the recommender calls beside them
//! do read natively); gcloud's own configuration and auth state; and
//! label-filtered listings, which only the pushed `--filter` expression can
//! narrow. Together with the withheld remembered-view store, this runner and
//! that cache are the demo-aware seams; nothing above the service layer knows
//! which mode it is in.
//!
//! The fixtures embedded here (`include_str!("../../../tests/fixtures/*.json")`)
//! are the exact same files the test suite parses against the real service
//! layer, so demo mode and the tests can never drift silently out of sync.
//! `nuvem --demo` itself lists the showcase under `tests/fixtures/demo/`
//! where a service has one; see [`Corpus`] for why there are two sets.
//!
//! `sdk/` is the same idea one layer down: the fixture backend for the
//! generated clients, serving those same files to a stub-backed client
//! instead of to a spawned command. See its module documentation.

pub mod sdk;

/// Which set of embedded documents a demo backend answers from.
///
/// Two sets, for two jobs. The captures under `tests/fixtures/` exist to pin
/// the parsers: they carry the edge cases a parser has to survive (a service
/// account with no fields at all, a disk with no location, a function in a
/// state no build knows) and only as many rows as each case needs. That makes
/// them right for the suite and wrong for a first impression: two instances,
/// three buckets and a blank service account look like an empty tool.
///
/// The showcase under `tests/fixtures/demo/` is what `nuvem --demo` lists: a
/// credible project with several rows per service, in the same wire shapes,
/// with fixture names only (`dev-project`, `example.com`, documentation and
/// private address ranges). A service with no showcase file answers from its
/// captures in both corpora. Every backend in a demo session reads the same
/// corpus, so the native and CLI paths still tell one story.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Corpus {
    /// The parser captures. What the suite's stub tests read.
    #[default]
    Captures,
    /// What `nuvem --demo` shows.
    Showcase,
}

impl Corpus {
    /// The document this corpus answers with, of the two given.
    pub fn pick(self, captures: &'static str, showcase: &'static str) -> &'static str {
        match self {
            Self::Captures => captures,
            Self::Showcase => showcase,
        }
    }

    /// The documents a `get` searches in this corpus, of the two sets given.
    pub fn documents(
        self,
        captures: &[&'static str],
        showcase: &[&'static str],
    ) -> Vec<&'static str> {
        match self {
            Self::Captures => captures.to_vec(),
            Self::Showcase => showcase.to_vec(),
        }
    }
}

use crate::services::gcloud::cli::{
    CommandError, CommandOutput, CommandProgram, CommandRunner, GcloudCommand,
};

/// Serves embedded JSON fixtures in place of a real `gcloud` invocation.
///
/// See the module documentation for why this is the only demo-aware seam.
#[derive(Debug, Clone, Default)]
pub struct DemoCommandRunner {
    corpus: Corpus,
    /// The showcase's gcloud configurations as this session has changed them:
    /// which one is active, and the properties set or unset on each. Held in
    /// memory for the life of the runner and never written anywhere, so a
    /// demo session can switch and edit configurations without touching the
    /// operator's own. `None` until the first change; the fixture answers
    /// until then.
    configurations: std::sync::Arc<std::sync::Mutex<Option<Vec<serde_json::Value>>>>,
}

impl DemoCommandRunner {
    /// A runner answering from the parser captures.
    pub fn new() -> Self {
        Self::default()
    }

    /// A runner answering from `corpus`.
    pub fn over(corpus: Corpus) -> Self {
        Self {
            corpus,
            ..Self::default()
        }
    }

    /// The showcase answers for gcloud's own configuration state, kept in
    /// memory: listing, activating, creating, setting and unsetting, and the
    /// account `auth list` reports for a configuration. `None` hands the argv
    /// on.
    fn configuration_state(&self, args: &[String]) -> Option<CommandOutput> {
        let named = args
            .windows(2)
            .find(|pair| pair[0] == "--configuration")
            .map(|pair| pair[1].clone());
        let positional = |after: &str| {
            args.iter()
                .skip_while(|arg| arg.as_str() != after)
                .skip(1)
                .find(|arg| !arg.starts_with('-'))
                .cloned()
        };
        let mut state = self
            .configurations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let rows = state.get_or_insert_with(|| {
            serde_json::from_str(include_str!(
                "../../../tests/fixtures/gcloud_configurations.json"
            ))
            .unwrap_or_default()
        });
        let name_of = |row: &serde_json::Value| {
            row.get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        };
        if matches(args, &["config", "configurations", "list"]) {
            return Some(fixture(&serde_json::Value::Array(rows.clone()).to_string()));
        }
        if matches(args, &["config", "configurations", "activate"]) {
            let wanted = positional("activate")?;
            for row in rows.iter_mut() {
                let active = name_of(row).as_deref() == Some(wanted.as_str());
                row["is_active"] = serde_json::Value::Bool(active);
            }
            return Some(ok_noop());
        }
        if matches(args, &["config", "configurations", "create"]) {
            let name = positional("create")?;
            rows.push(serde_json::json!({ "name": name, "is_active": false, "properties": {} }));
            return Some(ok_noop());
        }
        let property_path = |property: &str| match property.split_once('/') {
            Some((section, key)) => (section.to_string(), key.to_string()),
            None => ("core".to_string(), property.to_string()),
        };
        let target = |rows: &mut Vec<serde_json::Value>| -> Option<usize> {
            match &named {
                Some(name) => rows
                    .iter()
                    .position(|row| name_of(row).as_deref() == Some(name)),
                None => rows
                    .iter()
                    .position(|row| row["is_active"] == serde_json::Value::Bool(true)),
            }
        };
        if matches(args, &["config", "set"]) {
            let property = positional("set")?;
            let value = args
                .iter()
                .skip_while(|arg| **arg != property)
                .nth(1)?
                .clone();
            let index = target(rows)?;
            let (section, key) = property_path(&property);
            rows[index]["properties"][section.as_str()][key.as_str()] =
                serde_json::Value::String(value);
            return Some(ok_noop());
        }
        if matches(args, &["config", "unset"]) {
            let property = positional("unset")?;
            let index = target(rows)?;
            let (section, key) = property_path(&property);
            if let Some(section) = rows[index]["properties"]
                .get_mut(section.as_str())
                .and_then(serde_json::Value::as_object_mut)
            {
                section.remove(key.as_str());
            }
            return Some(ok_noop());
        }
        if matches(args, &["auth", "list"]) {
            let index = target(rows)?;
            let account = rows[index]["properties"]["core"]["account"].clone();
            return Some(fixture(
                &serde_json::json!([{ "account": account, "status": "ACTIVE" }]).to_string(),
            ));
        }
        None
    }
}

impl CommandRunner for DemoCommandRunner {
    /// Program first, argv second.
    ///
    /// Every arm below is reached only for the binary it was written for, and
    /// that is a correctness property rather than tidiness: [`matches`] is an
    /// in-order *subsequence* test, so a `bq` argv is free to satisfy a
    /// gcloud needle list that happens to appear inside it, and vice versa.
    /// Dispatching on the program up here is what makes that impossible for
    /// every arm at once, rather than for whichever arms a future author
    /// remembers to guard.
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        let args = &command.args;

        match command.program {
            CommandProgram::Gcloud => {
                if self.corpus == Corpus::Showcase
                    && let Some(answer) = self.configuration_state(args)
                {
                    return Ok(answer);
                }
                gcloud_fixture(args, self.corpus)
            }
            CommandProgram::Bq => match self.corpus {
                Corpus::Captures => bq_fixture(args),
                Corpus::Showcase => showcase_bq(args),
            },
        }
    }
}

/// The `bq` fixtures. Only reached for [`CommandProgram::Bq`].
fn bq_fixture(args: &[String]) -> Result<CommandOutput, CommandError> {
    // The dataset listing: `bq --format=prettyjson --project_id=<project> ls
    // -d -n 500`. Globals lead and the project rides as a flag rather than as
    // a positional — `bq` rejects the reverse order, which is why
    // `services/resources/bigquery.rs` builds it that way.
    if matches(args, &["ls", "-d"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/bigquery_datasets.json"
        )));
    }
    // The tables listing: the same globals, then `ls -n <cap> <dataset>` —
    // matched on that tail by [`is_bq_tables_listing`].
    //
    // One dataset's tables answer every dataset: the fixture is real output of
    // `bq ls bigquery-public-data:austin_bikeshare`, transcribed rather than
    // freshly captured (see 6153219), and drilling into any other demo dataset
    // lands it too — the tables carry `austin_bikeshare` as their own dataset,
    // which the preview pane shows.
    // Demo mode has one fixture per command, not per argument.
    if is_bq_tables_listing(args) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/bigquery_tables.json"
        )));
    }

    // `bq show <table_ref>`: the same globals, then the table — matched on that
    // shape by [`is_bq_table_show`]. Two different callers land here. Opening a
    // dataset issues one per listed row for its ROWS, BYTES and LAST MODIFIED
    // cells; entering a table issues one more for that table's column schema.
    // Their argvs are identical, so demo mode cannot serve them different
    // documents and one fixture has to answer both.
    //
    // That is why the *nested* response is the one served, and not
    // `bigquery_table_show.json`. Both carry the three numbers the fan-out
    // reads, so the choice costs the stats nothing; they differ in the half
    // only the schema view reads. The flat fixture's schema is three
    // `NULLABLE` scalars with no description among them — a schema view served
    // that shows none of the three things this level does: the dotted path a
    // `RECORD` flattens to, the `REPEATED` mode that makes one an array, and
    // the `-` an undocumented field paints. The nested fixture has all three.
    //
    // It is a recorded `bq --format=prettyjson show` response for
    // bigquery-public-data:ga4_obfuscated_sample_ecommerce.events_20210131,
    // transcribed rather than freshly captured (see 5504b90): its field names,
    // nesting and modes follow the published GA4 export schema, and its
    // counters carry the string-encoded int64 shape the API really sends
    // rather than verified numbers. So what demo mode paints in ROWS and BYTES
    // is the recorded response's, not an observed value for any table.
    //
    // One table's resource answers every table, for the same reason one
    // dataset's tables answer every dataset — demo mode has one fixture per
    // command, not per argument. So both listed demo tables paint the ga4
    // table's numbers, and both open the ga4 table's columns. The rows are
    // still named by the tables listing, which was recorded from a different
    // dataset; nothing in demo mode claims otherwise.
    if is_bq_table_show(args) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/bigquery_table_show_nested.json"
        )));
    }

    Err(no_fixture(CommandProgram::Bq, args))
}

/// True for the per-table `show` argv and nothing else: the subcommand is
/// `show`, and exactly one positional follows it — the table reference.
///
/// Keyed on its own shape rather than on "a `bq` argv that is not a listing",
/// which is the same rule the two listing arms follow. `bq show` also
/// describes datasets, jobs, models and transfer configs; none of those has a
/// fixture, and each of them names its subject with a flag this arity check
/// leaves out rather than being answered with a table's numbers.
///
/// This arm and the two listings are mutually exclusive by their predicates,
/// not by their order. When this predicate holds, every token is either a
/// flag (leading `-`), the literal `show`, or a table reference containing a
/// `.` — so no token is the bare `ls` that both listing arms require, and
/// neither of them can match. Moving any of the three arms therefore changes
/// nothing; `each_bq_argv_satisfies_exactly_one_arm_predicate` runs all three
/// argvs past all three predicates rather than checking the order.
fn is_bq_table_show(args: &[String]) -> bool {
    // The subcommand is the first token that is not a global flag: `bq`
    // requires the globals to lead, which is why the two listings build their
    // argv that way too.
    let Some(position) = args.iter().position(|arg| !arg.starts_with('-')) else {
        return false;
    };
    match &args[position..] {
        [subcommand, table_ref] => {
            subcommand == "show" && !table_ref.starts_with('-') && table_ref.contains('.')
        }
        _ => false,
    }
}

/// True for the tables listing's argv and nothing else: `ls`, then `-n <cap>`,
/// then one positional — the dataset.
///
/// Keyed on the shape it wants rather than on "an `ls` that is not the
/// datasets listing", because that negation is a catch-all. `bq ls` also takes
/// `-j`, `-m`, `-p`, `--routines` and `--transfer_config`; a future listing
/// built from any of them would be answered with tables instead of falling
/// through to [`no_fixture`], and a wrong fixture is exactly what
/// `every_service_lists_rows_through_the_demo_runner` cannot see.
///
/// The three `bq` arms are mutually exclusive by their predicates rather than
/// by their order: the datasets arm requires `-d`, and the datasets argv's
/// tail after `ls` opens with that flag where this predicate requires `-n`;
/// the `show` arm requires an argv with no bare `ls` token in it at all (see
/// [`is_bq_table_show`]). For any argv at most one of the three can match, so
/// moving any arm changes nothing — which is the claim, and it is a property
/// of the predicates rather than of a test run that failed to notice a
/// difference.
fn is_bq_tables_listing(args: &[String]) -> bool {
    let Some(position) = args.iter().position(|arg| arg == "ls") else {
        return false;
    };
    match &args[position + 1..] {
        // The cap is `-n`'s own value, so the dataset is the argument after
        // it rather than the one after the flag.
        [flag, cap, dataset] => {
            flag == "-n"
                && !cap.is_empty()
                && cap.chars().all(|c| c.is_ascii_digit())
                && !dataset.starts_with('-')
        }
        _ => false,
    }
}

/// The gcloud fixtures. Only reached for [`CommandProgram::Gcloud`].
fn gcloud_fixture(args: &[String], corpus: Corpus) -> Result<CommandOutput, CommandError> {
    if corpus == Corpus::Showcase
        && let Some(answer) = showcase_gcloud(args)
    {
        return answer;
    }
    // Specific `list`/`describe` matches are checked first so the
    // broader mutating fallbacks below never shadow them.
    if matches(args, &["config", "configurations", "list"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/gcloud_configurations.json"
        )));
    }
    if matches(args, &["projects", "list"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/gcloud_projects.json"
        )));
    }
    if matches(args, &["compute", "regions", "list"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/gcloud_compute_regions.json"
        )));
    }
    if matches(args, &["compute", "zones", "list"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/gcloud_compute_zones.json"
        )));
    }
    if matches(args, &["compute", "instances", "list"]) {
        return Ok(fixture(corpus.pick(
            include_str!("../../../tests/fixtures/gce_instances.json"),
            include_str!("../../../tests/fixtures/demo/gce_instances.json"),
        )));
    }
    if matches(args, &["compute", "instances", "describe"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/gce_instance_detail.json"
        )));
    }
    if matches(args, &["container", "clusters", "list"]) {
        return Ok(fixture(corpus.pick(
            include_str!("../../../tests/fixtures/gke_clusters.json"),
            include_str!("../../../tests/fixtures/demo/gke_clusters.json"),
        )));
    }
    // Writing a kubeconfig entry is local, but it is the user's real
    // kubeconfig, so demo mode answers it as a no-op like any other change.
    if matches(args, &["container", "clusters", "get-credentials"]) {
        return Ok(ok_noop());
    }
    if matches(args, &["container", "clusters", "describe"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/gke_cluster_detail.json"
        )));
    }
    // Secret Manager, metadata only. There is deliberately no arm for the
    // payload subcommand — demo mode cannot serve one because nothing in
    // this binary ever asks for one.
    if matches(args, &["secrets", "versions", "list"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/secret_versions.json"
        )));
    }
    if matches(args, &["secrets", "list"]) {
        return Ok(fixture(corpus.pick(
            include_str!("../../../tests/fixtures/secrets.json"),
            include_str!("../../../tests/fixtures/demo/secrets.json"),
        )));
    }
    if matches(args, &["secrets", "describe"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/secret_detail.json"
        )));
    }
    // VPC. `matches` is a subsequence test, so the networks arm below would
    // also answer `compute networks subnets list` — which is why the subnets
    // arm comes first. Reordering these two silently serves a document of
    // networks to the subnets table.
    //
    // PROVENANCE: `vpc_subnets.json` is a live capture (2026-08-25), scrubbed
    // only by replacing the source project id with `nuvem-fixture-project`,
    // and trimmed from the project's 43 rows to three. The trim removes rows
    // and changes no field of the rows it keeps. Three is what it takes to
    // show the fact the table is built around: all three are called `default`
    // and are told apart only by region.
    //
    // The fourth row (`us-west1`) is CONSTRUCTED, not captured: it is one of
    // the captured rows' shapes with `privateIpGoogleAccess` set to `true` and
    // a fresh region, range, gateway, id and fingerprint. The captured project
    // has the flag off on all 43 of its subnets, so with the capture alone the
    // `PGA` column only ever paints `Off` — and a mapping that lost the field
    // entirely would paint exactly the same table, since `false` is also what
    // an absent flag parses to. The constructed row is what makes the column
    // carry a value that has to be read to be rendered.
    if matches(args, &["compute", "networks", "subnets", "list"]) {
        return Ok(fixture(corpus.pick(
            include_str!("../../../tests/fixtures/vpc_subnets.json"),
            include_str!("../../../tests/fixtures/demo/vpc_subnets.json"),
        )));
    }
    //
    // PROVENANCE: `vpc_networks.json` is a live capture (2026-08-25), scrubbed
    // only by replacing the source project id with `nuvem-fixture-project`.
    // Its field set, key spellings and 43-entry `subnetworks` array are the
    // API's own, which is what makes demo mode's networks table a fair
    // rendering of a real project and what makes a test asserting a *missing*
    // field mean something.
    //
    // What it does NOT contain: any custom-mode network, any global-routing
    // network, and any legacy network. The account it was captured from can
    // reach none of the three, so those parser arms are exercised by rows
    // constructed from Google's published `compute/v1/networks` reference and
    // labelled CONSTRUCTED where they appear. Demo mode therefore shows the
    // `AUTO`/`REGIONAL` case only. Saying so here rather than only in the
    // commit message is the rule: a transcription that is taken for a capture is
    // a claim about evidence, and the way that regressed before was a deleted
    // sentence rather than a wrong one.
    if matches(args, &["compute", "networks", "list"]) {
        return Ok(fixture(corpus.pick(
            include_str!("../../../tests/fixtures/vpc_networks.json"),
            include_str!("../../../tests/fixtures/demo/vpc_networks.json"),
        )));
    }
    //
    // PROVENANCE: `vpc_firewall_rules.json` is the one fixture in this slice
    // that is *mixed*, and the split matters more here than anywhere else in
    // the VPC data. The four `default-allow-*` rules are a live capture
    // (2026-08-25), scrubbed only by replacing the source project id with
    // `nuvem-fixture-project`. The last three — `allow-web-to-tagged`,
    // `deny-all-egress` and `disabled-sa-scoped-ssh` — are CONSTRUCTED from
    // Google's published `compute/v1/firewalls` reference, because no project
    // this account can reach has an egress rule, a `denied` rule, a
    // target-tagged rule, a service-account-scoped rule or a disabled rule.
    //
    // So every genuinely interesting parser arm in this service rests on
    // transcription rather than on bytes gcloud produced: the `denied` half of
    // the allow/deny discrimination, EGRESS and its `destinationRanges`,
    // `targetTags`, `targetServiceAccounts`/`sourceServiceAccounts`, and
    // `disabled: true`. Two of those are the dangerous ones — a DENY rendered
    // as an ALLOW, and a disabled rule rendered as a live one — so if the wire
    // shape ever turns out to differ from the reference, those are the arms to
    // re-derive first. Saying that here rather than only in the commit
    // message is the rule.
    if matches(args, &["compute", "firewall-rules", "list"]) {
        return Ok(fixture(corpus.pick(
            include_str!("../../../tests/fixtures/vpc_firewall_rules.json"),
            include_str!("../../../tests/fixtures/demo/vpc_firewall_rules.json"),
        )));
    }
    if matches(args, &["compute", "disks", "list"]) {
        return Ok(fixture(corpus.pick(
            include_str!("../../../tests/fixtures/persistent_disks.json"),
            include_str!("../../../tests/fixtures/demo/persistent_disks.json"),
        )));
    }
    if matches(args, &["functions", "list"]) {
        return Ok(fixture(corpus.pick(
            include_str!("../../../tests/fixtures/cloud_functions.json"),
            include_str!("../../../tests/fixtures/demo/cloud_functions.json"),
        )));
    }
    // Pub/Sub. Both list calls are routed: the topics table's count column
    // is derived from the subscriptions list, so a demo serving only the
    // first would show every count as unknown and quietly demo the
    // degraded path as if it were the normal one.
    if matches(args, &["pubsub", "topics", "list"]) {
        return Ok(fixture(corpus.pick(
            include_str!("../../../tests/fixtures/pubsub_topics.json"),
            include_str!("../../../tests/fixtures/demo/pubsub_topics.json"),
        )));
    }
    if matches(args, &["pubsub", "subscriptions", "list"]) {
        return Ok(fixture(corpus.pick(
            include_str!("../../../tests/fixtures/pubsub_subscriptions.json"),
            include_str!("../../../tests/fixtures/demo/pubsub_subscriptions.json"),
        )));
    }
    // IAM service accounts, list only.
    if matches(args, &["iam", "service-accounts", "list"]) {
        return Ok(fixture(corpus.pick(
            include_str!("../../../tests/fixtures/iam_service_accounts.json"),
            include_str!("../../../tests/fixtures/demo/iam_service_accounts.json"),
        )));
    }
    // The project IAM policy. `get-iam-policy` names the project
    // positionally, so the match is on the first two words only.
    if matches(args, &["projects", "get-iam-policy"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/iam_policy.json"
        )));
    }
    if matches(args, &["run", "services", "list"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/cloud_run_services.json"
        )));
    }
    if matches(args, &["run", "services", "describe"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/cloud_run_service_detail.json"
        )));
    }
    if matches(args, &["run", "revisions", "list"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/cloud_run_revisions.json"
        )));
    }
    if matches(args, &["run", "revisions", "describe"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/cloud_run_revision_detail.json"
        )));
    }
    if matches(args, &["sql", "instances", "list"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/cloudsql_instances.json"
        )));
    }
    if matches(args, &["sql", "instances", "describe"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/cloudsql_instance_detail.json"
        )));
    }
    if matches(args, &["sql", "users", "list"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/cloudsql_users.json"
        )));
    }
    if matches(args, &["storage", "buckets", "list"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/gcs_buckets.json"
        )));
    }
    if matches(args, &["storage", "buckets", "describe"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/gcs_bucket_detail.json"
        )));
    }
    if matches(args, &["storage", "objects", "list"]) {
        // One page, because `gcloud storage objects list` emits no page token
        // and so the real runner can never ask for a second one.
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/gcs_objects.json"
        )));
    }
    if matches(args, &["storage", "objects", "describe"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/gcs_object_detail.json"
        )));
    }
    if matches(args, &["logging", "read"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/log_entries.json"
        )));
    }
    if matches(args, &["auth", "list"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/gcloud_auth.json"
        )));
    }
    if matches(args, &["recommender", "recommendations", "list"]) {
        return Ok(fixture(recommendations_fixture(args)));
    }
    // Ordered above the budgets arm only for readability; the two argvs share
    // no subsequence, so the order between them carries nothing.
    if matches(args, &["billing", "projects", "describe"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/finops_billing_project.json"
        )));
    }
    if matches(args, &["billing", "budgets", "list"]) {
        return Ok(fixture(include_str!(
            "../../../tests/fixtures/finops_budgets.json"
        )));
    }

    // Mutating commands never touch a real backend in demo mode; they
    // succeed as empty-stdout no-ops. Checked only after every
    // list/describe case above so those stay authoritative.
    //
    // gcloud's own configuration state has no API, so these three are the
    // whole mechanism for it and a demo session reaches them on every
    // activate.
    if matches(args, &["config", "configurations"]) {
        return Ok(ok_noop());
    }
    if matches(args, &["config", "set"]) {
        return Ok(ok_noop());
    }
    if matches(args, &["config", "unset"]) {
        return Ok(ok_noop());
    }
    // The two lifecycle arms are no longer on a demo session's path:
    // `run_instance_action_result` and its Cloud SQL twin take the native
    // client whenever the session holds one, and since the demo backend flip
    // it always does. They are kept rather than dropped for two reasons. The
    // no-op is a documented property of this runner, named by command in
    // `docs/guides/demo-mode.md` ("start/stop an instance ... succeed as
    // no-ops"), and two tests in `demo_runner_test` drive the CLI services
    // against this runner directly to hold it. Dropping them would turn that
    // documented no-op into the unrouted-argv error for any caller that still
    // builds a `Gcloud*Service` over this runner.
    if matches(args, &["compute", "instances"]) {
        return Ok(ok_noop());
    }
    if matches(args, &["sql", "instances"]) {
        return Ok(ok_noop());
    }

    Err(no_fixture(CommandProgram::Gcloud, args))
}

/// The showcase answers for the gcloud reads whose showcase document differs
/// from the capture by more than its rows: a `describe` answers the row it
/// names, and a listing scoped to one parent answers that parent's rows.
///
/// Reads with no showcase document in the CLI's shape — Cloud Run and Cloud
/// Storage, whose showcase is in the client library's shape — answer the
/// unrouted-argv error rather than a capture, so nothing outside the showcase
/// reaches a demo screen; the demo session reads both through their clients.
/// `None` hands the argv on to the arms shared by both corpora.
fn showcase_gcloud(args: &[String]) -> Option<Result<CommandOutput, CommandError>> {
    const INSTANCES: &str = include_str!("../../../tests/fixtures/demo/gce_instances.json");
    const CLUSTERS: &str = include_str!("../../../tests/fixtures/demo/gke_clusters.json");
    const SQL: &str = include_str!("../../../tests/fixtures/demo/cloudsql_instances.json");
    const SQL_USERS: &str = include_str!("../../../tests/fixtures/demo/cloudsql_users.json");
    const SECRETS: &str = include_str!("../../../tests/fixtures/demo/secrets.json");
    const VERSIONS: &str = include_str!("../../../tests/fixtures/demo/secret_versions.json");
    const POLICY: &str = include_str!("../../../tests/fixtures/demo/iam_policy.json");
    const ENTRIES: &str = include_str!("../../../tests/fixtures/demo/log_entries.json");

    let after = |word: &str| {
        args.iter()
            .skip_while(|arg| arg.as_str() != word)
            .skip(1)
            .find(|arg| !arg.starts_with('-'))
            .cloned()
            .unwrap_or_default()
    };
    let described = |listing: &str| -> Result<CommandOutput, CommandError> {
        let wanted = after("describe");
        let rows: Vec<serde_json::Value> = serde_json::from_str(listing).unwrap_or_default();
        rows.into_iter()
            .find(|row| {
                row.get("name")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|name| name.rsplit('/').next() == Some(wanted.as_str()))
            })
            // Indented, as `gcloud ... describe --format=json` prints it: a
            // plugin's output viewer shows this verbatim, and one minified
            // line read as a fault rather than as a document.
            .map(|row| fixture(&serde_json::to_string_pretty(&row).unwrap_or_default()))
            .ok_or_else(|| no_fixture(CommandProgram::Gcloud, args))
    };
    Some(if matches(args, &["compute", "instances", "describe"]) {
        described(INSTANCES)
    } else if matches(args, &["container", "clusters", "describe"]) {
        described(CLUSTERS)
    } else if matches(args, &["sql", "instances", "list"]) {
        Ok(fixture(SQL))
    } else if matches(args, &["sql", "instances", "describe"]) {
        described(SQL)
    } else if matches(args, &["sql", "users", "list"]) {
        Ok(fixture(SQL_USERS))
    } else if matches(args, &["secrets", "versions", "list"]) {
        let rows: Vec<serde_json::Value> = serde_json::from_str(VERSIONS).unwrap_or_default();
        let secret = format!("/secrets/{}/versions/", after("list"));
        let mine: Vec<serde_json::Value> = rows
            .into_iter()
            .filter(|row| {
                row.get("name")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|name| name.contains(&secret))
            })
            .collect();
        Ok(fixture(&serde_json::Value::Array(mine).to_string()))
    } else if matches(args, &["secrets", "describe"]) {
        described(SECRETS)
    } else if matches(args, &["projects", "get-iam-policy"]) {
        Ok(fixture(POLICY))
    } else if matches(args, &["logging", "read"]) {
        Ok(fixture(ENTRIES))
    } else if matches(args, &["run"]) || matches(args, &["storage"]) {
        Err(no_fixture(CommandProgram::Gcloud, args))
    } else {
        return None;
    })
}

/// The showcase `bq` answers: the sample project's datasets, each dataset's
/// own tables, and each table's own document, by the reference the argv
/// names. The same three argv shapes as [`bq_fixture`] and its predicates.
fn showcase_bq(args: &[String]) -> Result<CommandOutput, CommandError> {
    const DATASETS: &str = include_str!("../../../tests/fixtures/demo/bigquery_datasets.json");
    const TABLES: &str = include_str!("../../../tests/fixtures/demo/bigquery_tables.json");
    const SHOWS: &str = include_str!("../../../tests/fixtures/demo/bigquery_table_show.json");

    let tail = args.last().map(String::as_str).unwrap_or_default();
    let without_project = |reference: &str| {
        reference
            .split_once(':')
            .map_or(reference, |(_, rest)| rest)
            .to_string()
    };
    if matches(args, &["ls", "-d"]) {
        return Ok(fixture(DATASETS));
    }
    if is_bq_tables_listing(args) {
        let dataset = without_project(tail);
        let rows: Vec<serde_json::Value> = serde_json::from_str(TABLES).unwrap_or_default();
        let mine: Vec<serde_json::Value> = rows
            .into_iter()
            .filter(|row| {
                row.pointer("/tableReference/datasetId")
                    .and_then(serde_json::Value::as_str)
                    == Some(dataset.as_str())
            })
            .collect();
        return Ok(fixture(&serde_json::Value::Array(mine).to_string()));
    }
    if is_bq_table_show(args) {
        let wanted = without_project(tail);
        let rows: Vec<serde_json::Value> = serde_json::from_str(SHOWS).unwrap_or_default();
        return rows
            .into_iter()
            .find(|row| {
                row.get("id")
                    .and_then(serde_json::Value::as_str)
                    .map(without_project)
                    .as_deref()
                    == Some(wanted.as_str())
            })
            .map(|row| fixture(&row.to_string()))
            .ok_or_else(|| no_fixture(CommandProgram::Bq, args));
    }
    Err(no_fixture(CommandProgram::Bq, args))
}

/// The `recommender recommendations list` answer, chosen by the recommender the
/// argv names *and* the location it asks about.
///
/// Keyed on the pair rather than on the recommender alone, unlike the bq tables
/// listing where one fixture answers every dataset. `FinOpsState::recommendations`
/// flattens a recommender's cells across every location it asked, so a fixture
/// served for each of them would list the same recommendation once per zone and
/// the savings total would count it that many times — the one failure mode a
/// cost screen must not have.
///
/// Every other location answers `[]`, which is what the real API does and a
/// state the cards distinguish: `CellState::Empty` is "asked, nothing to save"
/// rather than "not asked".
///
/// The locations here are the ones `query_locations` derives from the demo
/// compute and disk fixtures: `europe-west1-b` for the zonal recommenders, and
/// its region for the address recommender, which is regional.
fn recommendations_fixture(args: &[String]) -> &'static str {
    const EMPTY: &str = "[]";
    let flag = |prefix: &str| -> Option<&str> {
        args.iter()
            .find_map(|arg| arg.strip_prefix(prefix))
            .filter(|value| !value.is_empty())
    };
    let (Some(recommender), Some(location)) = (flag("--recommender="), flag("--location=")) else {
        return EMPTY;
    };

    match (recommender, location) {
        ("google.compute.instance.IdleResourceRecommender", "europe-west1-b") => {
            include_str!("../../../tests/fixtures/finops_idle_vms.json")
        }
        ("google.compute.instance.MachineTypeRecommender", "europe-west1-b") => {
            include_str!("../../../tests/fixtures/finops_machine_type.json")
        }
        ("google.compute.disk.IdleResourceRecommender", "europe-west1-b") => {
            include_str!("../../../tests/fixtures/finops_idle_disks.json")
        }
        ("google.compute.address.IdleResourceRecommender", "europe-west1") => {
            include_str!("../../../tests/fixtures/finops_unused_addresses.json")
        }
        _ => EMPTY,
    }
}

/// The error an unrouted argv gets, naming the binary it was aimed at so a
/// `bq` argv that fell through cannot read as a missing gcloud fixture.
fn no_fixture(program: CommandProgram, args: &[String]) -> CommandError {
    CommandError::Failed {
        // Demo mode is not a child process: nothing wrote to a pipe, and the
        // message below is nuvem's own. It goes on the stderr side because
        // that is where a diagnostic belongs when there is no binary whose
        // habits decide otherwise — see `failure_diagnostic`.
        stdout: String::new(),
        stderr: format!(
            "demo mode: no fixture for {} command {args:?}",
            program.binary_name()
        ),
        exit_code: None,
    }
}

/// Returns true when every needle in `needles` appears in `args`, in order,
/// tolerating any number of interspersed values (flags, flag values,
/// resource names, etc.) between and around them.
fn matches(args: &[String], needles: &[&str]) -> bool {
    let mut needles = needles.iter();
    let mut current = needles.next();
    for arg in args {
        let Some(needle) = current else {
            break;
        };
        if arg == needle {
            current = needles.next();
        }
    }
    current.is_none()
}

fn fixture(json: &str) -> CommandOutput {
    CommandOutput {
        stdout: json.to_string(),
        stderr: String::new(),
    }
}

fn ok_noop() -> CommandOutput {
    CommandOutput {
        stdout: String::new(),
        stderr: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    /// A showcase describe answers one indented document, the way gcloud
    /// prints one, and it is the named resource's.
    #[test]
    fn a_showcase_describe_is_indented_like_gclouds() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/demo/gce_instances.json"
        ))
        .expect("the showcase instances parse");
        let name = rows[0]["name"].as_str().expect("a name").to_string();
        let output = showcase_gcloud(&args(&[
            "compute",
            "instances",
            "describe",
            &name,
            "--format=json",
        ]))
        .expect("answered")
        .expect("found");
        assert!(output.stdout.lines().count() > 10, "{}", output.stdout);
        assert!(output.stdout.contains("\n  \""), "{}", output.stdout);
        let parsed: serde_json::Value =
            serde_json::from_str(&output.stdout).expect("still one JSON document");
        assert_eq!(parsed["name"], rows[0]["name"]);
    }

    #[test]
    fn matches_ignores_interspersed_flag_values() {
        let command = args(&[
            "--configuration",
            "dev",
            "compute",
            "instances",
            "list",
            "--format=json",
        ]);

        assert!(matches(&command, &["compute", "instances", "list"]));
    }

    #[test]
    fn matches_requires_keyword_order() {
        let command = args(&["compute", "instances", "list"]);

        assert!(!matches(&command, &["instances", "compute", "list"]));
    }

    /// The tables arm must not become a catch-all for every other `bq ls`.
    ///
    /// `bq ls` also lists jobs (`-j`), projects (`-p`), transfer configs and
    /// routines. None of those has a fixture, and a demo build that answered
    /// one with a dataset's tables would render plausible wrong rows where the
    /// unrouted-argv error is the honest outcome — `every_service_lists_rows_
    /// through_the_demo_runner` checks that a service loads, never that it
    /// loaded the right document.
    #[test]
    fn a_bq_listing_that_is_neither_datasets_nor_tables_has_no_fixture() {
        let runner = DemoCommandRunner::new();

        let error = runner
            .run(GcloudCommand::bq(args(&[
                "--format=prettyjson",
                "ls",
                "-j",
                "-n",
                "500",
            ])))
            .expect_err("a job listing has no demo fixture");

        match error {
            CommandError::Failed { stderr, .. } => assert!(
                stderr.contains("no fixture for bq"),
                "expected the unrouted-argv error, got: {stderr}"
            ),
            other => panic!("expected CommandError::Failed, got {other:?}"),
        }
    }

    /// Every conjunct of the tables guard is load-bearing, and the cap is the
    /// one the suite could not see.
    ///
    /// `is_bq_tables_listing` survived turning either of the first two `&&`s
    /// of its chain into `||`: the only argv the suite pushed through the
    /// three-element arm was the job listing, whose second token is `-n`, and
    /// that fails `cap.chars().all(is_ascii_digit)` outright — so a weakened
    /// chain still answered `false` and the mutants lived.
    ///
    /// An empty cap excludes all three substitutions at once. `chars().all` is
    /// vacuously true on the empty string, which is exactly why
    /// `!cap.is_empty()` is in the chain at all; with any `||` in it,
    /// `bq ls -n '' <dataset>` reads as the tables listing and is answered
    /// with another dataset's tables instead of the unrouted-argv error.
    #[test]
    fn a_tables_listing_whose_cap_is_empty_is_not_the_tables_listing() {
        let runner = DemoCommandRunner::new();

        let listing = runner
            .run(GcloudCommand::bq(args(&[
                "--format=prettyjson",
                "ls",
                "-n",
                "500",
                "austin_bikeshare",
            ])))
            .expect("the real tables argv is the one this guard is keyed on");
        assert!(
            listing.stdout.contains("austin_bikeshare"),
            "the contrast case must really be served the tables fixture, or              the refusal below proves nothing: {}",
            listing.stdout
        );

        let error = runner
            .run(GcloudCommand::bq(args(&[
                "--format=prettyjson",
                "ls",
                "-n",
                "",
                "austin_bikeshare",
            ])))
            .expect_err("an empty cap is not the tables listing's argv");

        match error {
            CommandError::Failed { stderr, .. } => assert!(
                stderr.contains("no fixture for bq"),
                "expected the unrouted-argv error, got: {stderr}"
            ),
            other => panic!("expected CommandError::Failed, got {other:?}"),
        }
    }

    /// The three `bq` arms are exclusive by predicate, and this is where that
    /// claim is checked rather than asserted.
    ///
    /// Each of the three argvs the app really builds is run past all three
    /// predicates, so a widened arm fails here even when the arm it would
    /// shadow sits above it and hides the damage at run time. Reordering the
    /// arms cannot change any row of this table.
    #[test]
    fn each_bq_argv_satisfies_exactly_one_arm_predicate() {
        let datasets = args(&[
            "--format=prettyjson",
            "--project_id=bigquery-public-data",
            "ls",
            "-d",
            "-n",
            "500",
        ]);
        let tables = args(&[
            "--format=prettyjson",
            "--project_id=bigquery-public-data",
            "ls",
            "-n",
            "500",
            "austin_bikeshare",
        ]);
        let show = args(&[
            "--format=prettyjson",
            "--project_id=bigquery-public-data",
            "show",
            "bigquery-public-data:austin_bikeshare.bikeshare_stations",
        ]);

        let verdicts = |argv: &[String]| {
            (
                matches(argv, &["ls", "-d"]),
                is_bq_tables_listing(argv),
                is_bq_table_show(argv),
            )
        };

        assert_eq!(
            verdicts(&datasets),
            (true, false, false),
            "the datasets argv must satisfy the datasets predicate alone"
        );
        assert_eq!(
            verdicts(&tables),
            (false, true, false),
            "the tables argv must satisfy the tables predicate alone"
        );
        assert_eq!(
            verdicts(&show),
            (false, false, true),
            "the show argv must satisfy the show predicate alone"
        );

        // The three real argvs above agree on every conjunct of the show
        // predicate at once, so they cannot tell a conjunction from a
        // disjunction: each of them satisfies all three parts or none. The two
        // argvs below each satisfy some parts and not others, which is what
        // makes every conjunct load-bearing rather than merely present.
        let two_tokens_but_not_a_show = args(&[
            "--format=prettyjson",
            "--project_id=bigquery-public-data",
            "describe",
            "bigquery-public-data:austin_bikeshare.bikeshare_stations",
        ]);
        assert_eq!(
            verdicts(&two_tokens_but_not_a_show),
            (false, false, false),
            "a two-token tail whose subcommand is not `show` is not a table \
             show, however much its second token looks like a table \
             reference: the subcommand is what says which document comes back"
        );

        let a_show_of_something_unqualified = args(&[
            "--format=prettyjson",
            "--project_id=bigquery-public-data",
            "show",
            "austin_bikeshare",
        ]);
        assert_eq!(
            verdicts(&a_show_of_something_unqualified),
            (false, false, false),
            "and a `show` whose subject carries no `.` names a dataset, not a \
             table in one; answering it with a table's row count and byte \
             size would be a wrong document that parses"
        );
    }

    /// `bq show` also describes datasets, jobs and models, and none of those
    /// has a fixture.
    ///
    /// Those argvs name their subject with a flag (`-d`, `-j`, `-m`) ahead of
    /// the identifier, which the two-token arity check refuses — so they reach
    /// the unrouted-argv error rather than being answered with a table's row
    /// count and byte size, which would be a wrong document that parses.
    #[test]
    fn a_bq_show_that_is_not_a_table_show_has_no_fixture() {
        let runner = DemoCommandRunner::new();

        let error = runner
            .run(GcloudCommand::bq(args(&[
                "--format=prettyjson",
                "show",
                "-d",
                "bigquery-public-data:austin_bikeshare",
            ])))
            .expect_err("a dataset show has no demo fixture");

        match error {
            CommandError::Failed { stderr, .. } => assert!(
                stderr.contains("no fixture for bq"),
                "expected the unrouted-argv error, got: {stderr}"
            ),
            other => panic!("expected CommandError::Failed, got {other:?}"),
        }
    }

    #[test]
    fn matches_fails_when_a_needle_is_missing() {
        let command = args(&["compute", "instances", "describe"]);

        assert!(!matches(&command, &["compute", "instances", "list"]));
    }

    /// One listing, whatever the caller asks for: `gcloud storage objects
    /// list` emits no page token, so demo mode must not pretend otherwise.
    #[test]
    fn object_list_returns_a_single_page_in_gclouds_own_shape() {
        let runner = DemoCommandRunner::new();
        let command = GcloudCommand::new([
            "--configuration",
            "dev",
            "storage",
            "objects",
            "list",
            "gs://bucket/**",
            "--format=json",
        ]);

        let output = runner
            .run(command)
            .expect("demo object list should succeed");

        // A bare array, not the JSON API's `{"items": [...]}` envelope.
        assert!(output.stdout.trim_start().starts_with('['));
        assert!(!output.stdout.contains("nextPageToken"));
        assert!(!output.stdout.contains("\"items\""));
        assert!(output.stdout.contains("top-level.txt"));
    }

    #[test]
    fn auth_list_returns_active_account_fixture() {
        let runner = DemoCommandRunner::new();
        let command =
            GcloudCommand::new(["--configuration", "dev", "auth", "list", "--format=json"]);

        let output = runner.run(command).expect("demo auth list should succeed");

        assert!(output.stdout.contains("dev@example.com"));
        assert!(output.stdout.contains("ACTIVE"));
    }

    #[test]
    fn unknown_command_fails_with_demo_marker() {
        let runner = DemoCommandRunner::new();

        let error = runner
            .run(GcloudCommand::new(["totally", "unknown"]))
            .expect_err("unknown command should fail");

        match error {
            CommandError::Failed { stderr, .. } => assert!(stderr.contains("demo")),
            other => panic!("expected CommandError::Failed, got {other:?}"),
        }
    }
}
