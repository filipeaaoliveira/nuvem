//! BigQuery datasets, listed with one `bq ls -d`, and the tables inside one
//! dataset, listed with the same `bq ls` *without* that flag.
//!
//! # Why this service shells to `bq` and not to `gcloud`
//!
//! `gcloud` has no BigQuery surface: there is no `gcloud bigquery datasets
//! list`. The Cloud SDK ships a second binary, `bq`, and that is the only CLI
//! path to a dataset listing. Slice 1 taught the runner to spawn it;
//! this is its first caller.
//!
//! Two consequences fall out of that, and both are pinned by
//! `tests/bigquery_service_test.rs`:
//!
//! - **Global flags come before the subcommand.** `bq --format=prettyjson
//!   --project_id=p ls -d -n 500`, not `bq ls --format=prettyjson`. The real
//!   binary rejects the second spelling, and an argv assembled in gcloud's
//!   order would fail only against the live CLI.
//! - **The configuration travels in the environment.** `bq` has no
//!   `--configuration` flag; it resolves its configuration by shelling to
//!   `gcloud config config-helper`, so `CLOUDSDK_ACTIVE_CONFIG_NAME` is the
//!   only way to aim it at a named one. Every other service in this directory
//!   prefixes `--configuration <cfg>` instead.
//!
//! # Two listings, one flag apart
//!
//! `bq ls -d` lists a project's datasets; `bq ls <dataset>` lists the tables
//! inside one. Same binary, same subcommand, and `-d` is the whole of what
//! decides which of the two the API answers. `bq help ls` gives the flag as
//! `-d,--[no]datasets: Show datasets described by this identifier` — so it
//! does not ignore the positional argument, it *reinterprets* it: under `-d`
//! the identifier names the collection whose datasets are shown, and without
//! it the identifier names the dataset whose tables are shown.
//!
//! That is the reason the two argvs are built apart rather than shared behind
//! a boolean. One positional slot means two different things depending on one
//! flag, so a shared builder would turn a flag flip into a call asking a
//! different question about the same positional. It is also why the tables
//! test asserts the flag's *absence* rather than only the dataset's presence.
//!
//! # A third call, one table at a time
//!
//! Row counts, byte sizes and the last-modified time are not on `bq ls` output
//! at all, under either flag. Each is answered by `bq show <table>`, so the
//! three of them cost one call per listed row — which is why they are a
//! separate method rather than something the listing could have returned.
//! [`GcloudBigQueryService::load_table_stats`] issues one such call; nothing
//! in this module fans it out over a listing.
//!
//! `bq show` answers with a `schema` too, and
//! [`GcloudBigQueryService::load_table_stats`] reads none of it. The schema
//! drill-down re-issues its own `show` for the one table being drilled into —
//! [`GcloudBigQueryService::load_table_schema`] — rather than the fan-out
//! retaining every listed row's schema, which would trade unbounded memory for
//! a call that drill is already waiting on.
//!
//! That is why there are two parses over one document rather than one wider
//! one. [`parse_table_show_json`] answers three numbers for a row in a
//! listing; [`parse_table_schema_json`] answers a field list for one selected
//! table. They are issued at different times, for different rows, and a merged
//! parse would make each call carry the other's cost.
//!
//! # An empty listing is not a failure
//!
//! `bq ls -d` against a project with no datasets exits 0 with zero bytes of
//! stdout — not `[]`. Handing that to `serde_json` would fail the load and
//! paint an error over a project that is merely empty, so the empty document
//! is resolved to an empty list before deserialization is attempted.

use serde::Deserialize;

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::resource::{
    BigQueryDataset, BigQuerySchemaField, BigQueryTable, BigQueryTableKind, BigQueryTableStats,
    ComputeKeyValue, TableStat,
};

use crate::services::gcloud::cli::{CommandError, CommandRunner, GcloudCommand};
use crate::services::resources::list_filter::RESOURCE_LIST_LIMIT;

/// Verbatim-superset raw fields eligible for the regex pushdown — deliberately
/// none.
///
/// The never-drop-a-row invariant is a claim about *display* values: each of a
/// service's searchable fields must be a verbatim substring of a pushed raw
/// field, or be enumerated in [`OPAQUE_LABELS`]. Nothing can satisfy that here,
/// for a reason no other service has: `bq ls` accepts no server-side filter
/// expression at all. There is no `--filter`, so there is no clause to push
/// down and nothing a pushdown could be checked against. Empty degrades to full
/// client-side narrowing, which is the safe direction — it can only widen what
/// matches, never hide a row that is on screen.
pub const FILTER_FIELDS: &[&str] = &[];

/// Empty for the same reason [`FILTER_FIELDS`] is: with nothing pushed there is
/// nothing for the gate to protect.
pub const OPAQUE_LABELS: &[&str] = &[];

/// The environment variable that aims `bq` at a named gcloud configuration.
/// Named here rather than inline so the service and its test agree on one
/// spelling.
const ACTIVE_CONFIG_ENV: &str = "CLOUDSDK_ACTIVE_CONFIG_NAME";

#[derive(Debug, thiserror::Error)]
pub enum BigQueryParseError {
    #[error("invalid BigQuery JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum BigQueryLoadError {
    #[error("{0}")]
    Command(#[from] CommandError),
    #[error("{0}")]
    Parse(#[from] BigQueryParseError),
}

pub struct GcloudBigQueryService<R> {
    runner: R,
}

impl<R> GcloudBigQueryService<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn runner(&self) -> &R {
        &self.runner
    }
}

impl<R: CommandRunner> GcloudBigQueryService<R> {
    /// Every dataset in the context's project.
    ///
    /// Takes no filter argument, unlike its siblings: `bq ls` has no
    /// `--filter`, so there is nothing a filter could be pushed into and a
    /// parameter accepting one would be a promise the argv cannot keep. The
    /// table narrows wholly client-side — see [`FILTER_FIELDS`].
    pub fn load_datasets(
        &self,
        context: &GcloudContext,
    ) -> Result<Vec<BigQueryDataset>, BigQueryLoadError> {
        // Global flags first, then the subcommand and its own flags: `bq`
        // parses in that order and rejects the reverse.
        let mut args = vec!["--format=prettyjson".to_string()];
        if let Some(project) = context.project.as_deref() {
            // Only when there is one. `--project_id=` with nothing after it
            // names a project called the empty string; omitting the flag lets
            // `bq` fall back to the configuration's own project, which is what
            // the environment variable below selects.
            args.push(format!("--project_id={project}"));
        }
        args.extend([
            "ls".to_string(),
            "-d".to_string(),
            "-n".to_string(),
            RESOURCE_LIST_LIMIT.to_string(),
        ]);

        let command =
            GcloudCommand::bq(args).with_env(ACTIVE_CONFIG_ENV, context.configuration.as_str());
        let output = self.runner.run(command)?;
        Ok(parse_datasets_json(&output.stdout)?)
    }

    /// Every table inside one dataset of the context's project.
    ///
    /// The same `bq ls` the datasets call issues, minus `-d` and plus the
    /// dataset as a positional argument. Both halves of that matter together:
    /// `-d` is documented as "show datasets described by this identifier", so
    /// adding it here would not ignore `dataset_id` but reread it as the
    /// identifier whose *datasets* to list — a different question about the
    /// same positional, which is why the two argvs are built apart.
    pub fn load_tables(
        &self,
        context: &GcloudContext,
        dataset_id: &str,
    ) -> Result<Vec<BigQueryTable>, BigQueryLoadError> {
        // Globals first, exactly as in `load_datasets` — `bq` rejects the
        // reverse order.
        let mut args = vec!["--format=prettyjson".to_string()];
        if let Some(project) = context.project.as_deref() {
            args.push(format!("--project_id={project}"));
        }
        args.extend([
            "ls".to_string(),
            // No `-d` here. See the module note: it would reread the
            // positional below as a collection to list datasets from.
            "-n".to_string(),
            RESOURCE_LIST_LIMIT.to_string(),
            // Last, and positional: `-n` consumes the argument after it, so
            // the dataset has to follow the cap's value rather than sit
            // between the flag and its number.
            dataset_id.to_string(),
        ]);

        let command =
            GcloudCommand::bq(args).with_env(ACTIVE_CONFIG_ENV, context.configuration.as_str());
        let output = self.runner.run(command)?;
        Ok(parse_tables_json(&output.stdout)?)
    }

    /// The row count, byte size and last-modified time of one table.
    ///
    /// A third argv, and the first that is not an `ls`: `bq show` takes one
    /// table identifier and answers with the table resource, which is where
    /// these three numbers live. It is one call per table, so a listing of N
    /// tables costs N of these — the fan-out, and the decision of when to pay
    /// it, belongs to the caller.
    ///
    /// The identifier is fully qualified, `project:dataset.table`, whenever
    /// the context names a project. Unqualified, `bq` would resolve the
    /// dataset against the configuration's own project, which is the same
    /// project the listing came from only by coincidence.
    pub fn load_table_stats(
        &self,
        context: &GcloudContext,
        dataset_id: &str,
        table_id: &str,
    ) -> Result<BigQueryTableStats, BigQueryLoadError> {
        let stdout = self.show_table(context, dataset_id, table_id)?;
        Ok(parse_table_show_json(&stdout)?)
    }

    /// The column schema of one table, flattened to one row per field.
    ///
    /// The same `bq show` [`Self::load_table_stats`] issues, reading a
    /// different part of the answer. It is a second call rather than a second
    /// read of the first one because the two happen at different times and
    /// about different tables: the stats fan out over every row of a listing,
    /// this fires once for the row an operator drilled into. Keeping every
    /// listed row's schema against the chance one of them is drilled into
    /// would hold an unbounded amount of it to save a call that drill is
    /// already waiting on.
    pub fn load_table_schema(
        &self,
        context: &GcloudContext,
        dataset_id: &str,
        table_id: &str,
    ) -> Result<Vec<BigQuerySchemaField>, BigQueryLoadError> {
        let stdout = self.show_table(context, dataset_id, table_id)?;
        Ok(parse_table_schema_json(&stdout)?)
    }

    /// The one `bq show` both per-table calls issue, and their whole argv.
    ///
    /// Shared rather than written twice so that "the schema load asks the same
    /// question the stats load asks" is a fact about the code instead of a
    /// claim about two argvs that could drift apart. What the caller does with
    /// the answer is where the two part.
    fn show_table(
        &self,
        context: &GcloudContext,
        dataset_id: &str,
        table_id: &str,
    ) -> Result<String, CommandError> {
        // Globals first, exactly as in the two listings — `bq` rejects the
        // reverse order regardless of the subcommand.
        let mut args = vec!["--format=prettyjson".to_string()];
        if let Some(project) = context.project.as_deref() {
            args.push(format!("--project_id={project}"));
        }
        // With no project in the context the identifier stays `dataset.table`
        // rather than gaining a `:` with nothing before it, which `bq` reads
        // as a project named the empty string.
        let table_ref = match context.project.as_deref() {
            Some(project) => format!("{project}:{dataset_id}.{table_id}"),
            None => format!("{dataset_id}.{table_id}"),
        };
        // No `-n` cap: `show` answers about exactly one table, so there is no
        // listing to bound.
        args.extend(["show".to_string(), table_ref]);

        let command =
            GcloudCommand::bq(args).with_env(ACTIVE_CONFIG_ENV, context.configuration.as_str());
        Ok(self.runner.run(command)?.stdout)
    }
}

/// The wire shape of a dataset as `bq ls -d --format=prettyjson` reports it.
///
/// Every field defaults, so a row missing any of them still parses; only a
/// document that is not the expected JSON fails the load, which is the same
/// bargain the sibling services strike. The fixture really needs this: one of
/// the captured datasets carries `friendlyName` and the next omits the key.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDataset {
    #[serde(default)]
    dataset_reference: RawDatasetReference,
    #[serde(default)]
    friendly_name: Option<String>,
    #[serde(default)]
    location: String,
    #[serde(default)]
    labels: std::collections::BTreeMap<String, String>,
}

/// The nested object carrying the dataset's identity. `bq` reports the id and
/// the owning project here rather than at the top level; the top-level `id` is
/// the two joined by a colon, which is a display string rather than either
/// field.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDatasetReference {
    #[serde(default)]
    dataset_id: String,
    #[serde(default)]
    project_id: String,
}

/// Parses a `bq ls -d --format=prettyjson` document.
///
/// Crate-visible and taking a `&str` rather than living behind the runner, so
/// the parse is testable without spawning anything.
pub fn parse_datasets_json(stdout: &str) -> Result<Vec<BigQueryDataset>, BigQueryParseError> {
    // An empty project exits 0 with no stdout at all — not `[]`. Resolved
    // before deserialization rather than by rescuing a parse error, so a
    // genuinely malformed document still fails the load.
    if stdout.trim().is_empty() {
        return Ok(Vec::new());
    }
    let raw: Vec<RawDataset> = serde_json::from_str(stdout)?;
    Ok(raw.into_iter().map(dataset_from_raw).collect())
}

fn dataset_from_raw(raw: RawDataset) -> BigQueryDataset {
    BigQueryDataset {
        dataset_id: raw.dataset_reference.dataset_id,
        project_id: raw.dataset_reference.project_id,
        location: raw.location,
        // `"friendlyName": ""` is a shape the captured fixture really has, and
        // it means the same as the key being absent. Folded to `None` here so
        // the preview paints its dash rather than a blank line that reads as a
        // rendering fault.
        friendly_name: raw.friendly_name.filter(|name| !name.is_empty()),
        // `BTreeMap` rather than a `HashMap`: the labels reach the row as an
        // ordered list, and a listing's object order is not something the API
        // promises, so the key order is what makes a `k=v` filter match the
        // same way twice.
        labels: raw
            .labels
            .into_iter()
            .map(|(key, value)| ComputeKeyValue { key, value })
            .collect(),
    }
}

/// The wire shape of a table as `bq ls <dataset> --format=prettyjson` reports
/// it.
///
/// Every field defaults for the same reason [`RawDataset`]'s do: the fixture's
/// first row omits `labels` entirely while its second carries six, and a row
/// missing an optional key must parse rather than fail the whole listing.
///
/// `creation_time` is a `String`, not an `i64`. BigQuery serialises int64
/// fields as JSON strings (`"1495683814344"`), so deserializing it as a number
/// would fail on every real row.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTable {
    #[serde(default)]
    table_reference: RawTableReference,
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    creation_time: Option<String>,
    #[serde(default)]
    labels: std::collections::BTreeMap<String, String>,
}

/// The nested object carrying the table's identity. As with datasets, the
/// top-level `id` is a display string joining project, dataset and table, so
/// the individual ids are read from here.
///
/// `projectId` is on the wire and deliberately not read: a table's project is
/// the context's project, and a second copy of it on every row would be a
/// field the table could disagree with itself about. Unknown keys are ignored
/// by serde, so its absence here costs nothing.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTableReference {
    #[serde(default)]
    dataset_id: String,
    #[serde(default)]
    table_id: String,
}

/// Parses a `bq ls <dataset> --format=prettyjson` document.
///
/// Crate-visible and taking a `&str` for the same reason
/// [`parse_datasets_json`] is: the parse is worth testing without spawning
/// anything.
pub fn parse_tables_json(stdout: &str) -> Result<Vec<BigQueryTable>, BigQueryParseError> {
    // An empty dataset exits 0 with no stdout at all, exactly as an empty
    // project does for the datasets listing.
    if stdout.trim().is_empty() {
        return Ok(Vec::new());
    }
    let raw: Vec<RawTable> = serde_json::from_str(stdout)?;
    Ok(raw.into_iter().map(table_from_raw).collect())
}

fn table_from_raw(raw: RawTable) -> BigQueryTable {
    BigQueryTable {
        dataset_id: raw.table_reference.dataset_id,
        table_id: raw.table_reference.table_id,
        kind: table_kind_from_raw(&raw.r#type),
        // Every way this can fail resolves the same way: the key absent, the
        // value an empty string, a value that is not a number at all, and a
        // value too large for an i64 all end at `None`. `parse` rejects the
        // last three and `and_then` short-circuits the first, so none of them
        // is a reason to drop a table an operator asked to see — the row
        // renders a placeholder instead.
        created: raw
            .creation_time
            .and_then(|millis| millis.trim().parse().ok()),
        // Sorted by key, like the datasets' labels: a listing's object order
        // is not something the API promises.
        labels: raw
            .labels
            .into_iter()
            .map(|(key, value)| ComputeKeyValue { key, value })
            .collect(),
        // The listing carries none of the three, under either flag. They stay
        // unavailable until this row's own `bq show` answers, which is a
        // separate call — and unavailable is not zero, so a row waiting on its
        // `show` never claims to be an empty table.
        rows: TableStat::Unavailable,
        bytes: TableStat::Unavailable,
        last_modified: TableStat::Unavailable,
    }
}

/// Maps BigQuery's `type` onto the closed kinds this app names, carrying
/// anything else through verbatim.
///
/// The catch-all is not laziness about enumerating the rest: BigQuery ships
/// new types (snapshots, clones) without a client change, and telling the
/// reader an unrecognised row is a plain table would be a claim the API never
/// made.
fn table_kind_from_raw(raw: &str) -> BigQueryTableKind {
    match raw {
        "TABLE" => BigQueryTableKind::Table,
        "VIEW" => BigQueryTableKind::View,
        "MATERIALIZED_VIEW" => BigQueryTableKind::MaterializedView,
        "EXTERNAL" => BigQueryTableKind::External,
        other => BigQueryTableKind::Other(other.to_string()),
    }
}

/// The wire shape of the table resource `bq show --format=prettyjson` answers
/// with, narrowed to the three numbers this slice reads.
///
/// All three are `Option<String>`, not numbers: BigQuery serialises int64
/// fields as JSON strings (`"numRows": "101"`), so a numeric field here would
/// fail to deserialize against every real response.
///
/// `schema` is absent from this struct on purpose rather than by oversight.
/// Serde ignores unknown keys, so the schema the response really carries is
/// dropped here — see the module note for why a later drill-down will re-read
/// it instead of this call retaining it.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTableShow {
    #[serde(default)]
    num_rows: Option<String>,
    #[serde(default)]
    num_bytes: Option<String>,
    #[serde(default)]
    last_modified_time: Option<String>,
}

/// Parses a `bq show <table> --format=prettyjson` document.
///
/// Crate-visible and taking a `&str` for the same reason the two listing
/// parsers are: worth testing without spawning anything.
///
/// An unreadable *value* and an unreadable *document* part ways here. A
/// missing, empty, non-numeric or out-of-range field becomes
/// [`TableStat::Unavailable`] and the other two still land; a document that is
/// not the expected JSON fails, because there is then no table resource to
/// have read anything from. That is also why an empty stdout is not rescued
/// into a row of unavailables the way an empty listing is rescued into an
/// empty list: `bq show` answering nothing at all is a failure of the call,
/// and the caller can tell it apart by the error.
pub fn parse_table_show_json(stdout: &str) -> Result<BigQueryTableStats, BigQueryParseError> {
    let raw: RawTableShow = serde_json::from_str(stdout)?;
    Ok(BigQueryTableStats {
        rows: stat_from_raw(raw.num_rows.as_deref()),
        bytes: stat_from_raw(raw.num_bytes.as_deref()),
        last_modified: stat_from_raw(raw.last_modified_time.as_deref()),
    })
}

/// One wire string to one stat.
///
/// Every way the value can be unusable — the key absent, an empty string, a
/// word, a number too large for an `i64` — resolves to
/// [`TableStat::Unavailable`], and none of them resolves to `Known(0)`. A
/// literal `"0"` on the wire is the one input that does become `Known(0)`,
/// which is the whole distinction the type exists to keep.
fn stat_from_raw(raw: Option<&str>) -> TableStat {
    match raw.and_then(|value| value.trim().parse().ok()) {
        Some(value) => TableStat::Known(value),
        None => TableStat::Unavailable,
    }
}

/// The same `bq show` document [`RawTableShow`] reads, narrowed to the one key
/// that one deliberately drops.
///
/// A second struct rather than a wider first one: the two parses answer
/// different questions about the same response at different points in the
/// session (see the module note), and `schema` staying out of [`RawTableShow`]
/// is what keeps the fan-out from retaining a field list per listed row.
///
/// `schema` defaults, so a response without one parses to an empty field list.
/// A table resource that named no fields was still read; only a document that
/// is not the expected JSON fails.
#[derive(Debug, Default, Deserialize)]
struct RawTableSchemaShow {
    #[serde(default)]
    schema: RawSchema,
}

#[derive(Debug, Default, Deserialize)]
struct RawSchema {
    #[serde(default)]
    fields: Vec<RawSchemaField>,
}

/// One node of the schema tree, which is recursive: a `RECORD` carries its
/// children in its own `fields`, to arbitrary depth.
///
/// `mode` and `description` are `Option` because the wire really omits them —
/// `mode` defaults to `NULLABLE` server-side, and a field with no description
/// carries no key at all, which is a different document from one carrying
/// `"description": ""`.
#[derive(Debug, Default, Deserialize)]
struct RawSchemaField {
    #[serde(default)]
    name: String,
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    fields: Vec<RawSchemaField>,
}

/// BigQuery's documented default when a field omits `mode`.
///
/// Named rather than inlined so the parse and its test agree on one spelling,
/// and so the fallback is a documented value rather than a blank cell.
const DEFAULT_FIELD_MODE: &str = "NULLABLE";

/// Parses the schema out of a `bq show <table> --format=prettyjson` document,
/// flattened depth-first into one entry per field.
///
/// Parents come before their children and are kept as entries of their own: a
/// `RECORD`'s own type and mode — `REPEATED` above all — are facts no child of
/// it records, so replacing a record with its children would lose them.
///
/// Deliberately separate from [`parse_table_show_json`], which reads the same
/// document for its three numbers and drops `schema`. See the module note for
/// why the two are not merged.
pub fn parse_table_schema_json(
    stdout: &str,
) -> Result<Vec<BigQuerySchemaField>, BigQueryParseError> {
    let raw: RawTableSchemaShow = serde_json::from_str(stdout)?;
    let mut fields = Vec::new();
    flatten_schema_fields(&raw.schema.fields, "", &mut fields);
    Ok(fields)
}

/// Walks one level of the tree, appending each field and then its subtree.
///
/// `prefix` is the dotted ancestry of the level being walked, empty at the
/// top. Recursion rather than an explicit stack because the depth is the
/// schema's own nesting — single digits in practice — and a hand-rolled stack
/// is where an "emit the children first" ordering bug would hide.
fn flatten_schema_fields(raw: &[RawSchemaField], prefix: &str, out: &mut Vec<BigQuerySchemaField>) {
    for field in raw {
        let path = if prefix.is_empty() {
            field.name.clone()
        } else {
            format!("{prefix}.{}", field.name)
        };
        out.push(BigQuerySchemaField {
            path: path.clone(),
            field_type: field.r#type.clone(),
            // An absent `mode` is nullable, which is what the API means by
            // omitting it. `description` gets no such treatment: absent and
            // empty are different answers there, and only `mode` has a
            // documented default.
            mode: field
                .mode
                .clone()
                .unwrap_or_else(|| DEFAULT_FIELD_MODE.to_string()),
            description: field.description.clone(),
        });
        // After the parent, not instead of it, and with this field's path as
        // the children's prefix.
        flatten_schema_fields(&field.fields, &path, out);
    }
}
