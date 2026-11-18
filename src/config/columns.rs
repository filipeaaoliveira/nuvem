//! User-configurable table columns and default sort, per resource service.
//!
//! Parses and validates the `[columns.<service>]` tables of the config file
//! against the per-service catalogs in
//! [`crate::app::projection::columns`] — the catalog keys (`name`, `zone`,
//! `machine-type`, ...) are the vocabulary, so there is no second list of
//! column names to keep in step.
//!
//! Validation follows `[aliases]`/`[hotkeys]`: a problem is a warning, never a
//! load failure. It differs in *granularity*. A block configures one table, so
//! it is validated all-or-nothing: any problem drops that service's whole
//! block (it falls back to its built-in columns) and leaves every other
//! service alone. A half-applied column list would be worse than none, and a
//! silently ignored setting — the user edits the file, sees no change, and is
//! never told why — is the failure mode this validation exists to prevent.
//!
//! That is why the blocks arrive here as raw [`toml::Value`]s: the *shape* of
//! a block is checked here too (`columns = "name"` instead of a list, a
//! non-string `sort`, ...), so a type slip in one service block is one more
//! per-service warning rather than a deserialization failure that would
//! discard the user's whole file.

use std::collections::BTreeMap;

use nuvem_domain::gcp::resource::ResourceServiceKind;

use crate::app::projection::columns::{ColumnMeta, IDENTITY_COLUMN, catalog_meta};
use crate::app::projection::table::SortDirection;
use crate::config::problems::{ConfigProblemSource, ConfigProblems};
use crate::config::resolve_service_kind;

/// The tail a block-invalidating warning ends with on a fresh load, when
/// nothing was already running for the service.
const FALLBACK: &str = "ignoring this service's column settings";

/// The reload counterpart of [`FALLBACK`]: the tail a block-invalidating
/// warning ends with when the service already has settings in force (a live
/// reload keeps them rather than falling back to the built-in columns).
const KEPT: &str = "ignoring this block, keeping the column settings already in use";

/// Every setting a `[columns.<service>]` block may carry; anything else in
/// the block is a typo and rejects it.
const SETTINGS: [&str; 3] = ["columns", "sort", "sort_direction"];

/// A validated, per-service column layout: catalog indices in display order,
/// plus the resolved default sort. `None` means "keep the built-in default",
/// so a block may configure only the columns, only the sort, or both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceColumns {
    /// Catalog indices in display order; `None` keeps the default columns.
    pub columns: Option<Vec<usize>>,
    /// Catalog index of the sort column plus its direction; `None` keeps the
    /// default sort.
    pub sort: Option<(usize, SortDirection)>,
}

/// The validated `[columns]` table: the services that carry a usable
/// configuration, and the services whose block was rejected on this load.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ColumnSettings {
    services: Vec<(ResourceServiceKind, ServiceColumns)>,
    invalid: Vec<ResourceServiceKind>,
}

impl ColumnSettings {
    /// No service configured — the default when the file has no `[columns]`
    /// table. Every service keeps its built-in columns and sort.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Parse and validate the raw `[columns.<service>]` blocks. Each service
    /// contributes at most one warning, composed from what it ended up on
    /// once every block naming it has been read; other services are untouched.
    ///
    /// `running` is the settings currently in force — on a live reload, the
    /// config that was applied before this one; on a fresh load, there is
    /// nothing running yet, so the caller passes [`Self::empty()`]. It never
    /// changes what gets validated or adopted, only whether an invalidated
    /// service's warning says it kept what was running or fell back to its
    /// built-in columns.
    pub fn from_file(
        raw: &BTreeMap<String, toml::Value>,
        problems: &mut ConfigProblems,
        running: &Self,
    ) -> Self {
        let mut settings = Self::empty();
        // What each service's blocks add up to, in first-encounter order.
        // Nothing is reported from inside the loop: several keys can name one
        // service (`gce`, `compute`) and only the alphabetically last decides,
        // so a message written for an earlier key would outlive its truth.
        let mut outcomes: Vec<(ResourceServiceKind, ServiceOutcome<'_>)> = Vec::new();

        for (name, section) in raw {
            let Some(kind) = resolve_service_kind(name) else {
                // Not a service at all: nothing to fall back to, no real
                // service is affected, and so nothing to compose later.
                problems.push(
                    ConfigProblemSource::Columns,
                    format!("columns.{name}: unknown service; skipping"),
                );
                continue;
            };

            // The blocks arrive in key order (see [`Self::insert`]), so each
            // one supersedes what the keys before it left, both in the
            // settings and in what this service will report.
            let problem = match validate(section, catalog_meta(kind)) {
                Ok(columns) => {
                    settings.insert(kind, columns);
                    None
                }
                Err(problem) => {
                    settings.reject(kind);
                    Some(problem)
                }
            };

            match outcomes.iter_mut().find(|(service, _)| *service == kind) {
                Some((_, outcome)) => outcome.keys.push((name.as_str(), problem)),
                None => outcomes.push((
                    kind,
                    ServiceOutcome {
                        keys: vec![(name.as_str(), problem)],
                        // The same branch `adopt_over` takes for this
                        // service: `Some` there means adoption keeps what is
                        // running rather than falling back, so the warning
                        // has to agree with it word for word, not with a
                        // separate "is this a reload" flag that could drift
                        // out of step with what actually gets kept.
                        tail: if running.get(kind).is_some() {
                            KEPT
                        } else {
                            FALLBACK
                        },
                    },
                )),
            }
        }

        for (_, outcome) in outcomes {
            if let Some(message) = outcome.message() {
                problems.push(ConfigProblemSource::Columns, message);
            }
        }

        settings
    }

    /// This service's validated configuration, if it has one.
    pub fn get(&self, kind: ResourceServiceKind) -> Option<&ServiceColumns> {
        self.services
            .iter()
            .find(|(service, _)| *service == kind)
            .map(|(_, columns)| columns)
    }

    /// True when this service's block failed validation on this load.
    pub fn is_invalid(&self, kind: ResourceServiceKind) -> bool {
        self.invalid.contains(&kind)
    }

    /// The settings a live reload installs: `self` (the file just read) laid
    /// over `previous` (the settings currently running), per service.
    ///
    /// A service whose new block failed validation keeps what it is already
    /// running — the user's last good table, not the built-in defaults, and
    /// never half of each. Every other service takes `self`'s entry, including
    /// its absence, so removing a block puts that service back on its
    /// defaults. Nothing is merged *within* a service: a block is the unit of
    /// configuration, so it is also the unit of adoption.
    pub fn adopt_over(&self, previous: &Self) -> Self {
        let mut merged = Self::empty();
        for kind in ResourceServiceKind::ALL {
            if self.is_invalid(kind) {
                match previous.get(kind) {
                    Some(columns) => merged.insert(kind, columns.clone()),
                    // Nothing was running either: the service stays on its
                    // defaults, and the merged settings record why.
                    None => merged.reject(kind),
                }
            } else if let Some(columns) = self.get(kind) {
                merged.insert(kind, columns.clone());
            }
        }
        merged
    }

    /// Records a service's validated block, clearing any rejection an earlier
    /// key for the same service left behind. Two config keys can name the
    /// same service (`gce` and `compute`); the blocks arrive here sorted by
    /// key — [`crate::config::file::FileConfig::columns`] is a `BTreeMap`, so
    /// file order is already lost — and the alphabetically last key decides
    /// the service's state on its own, so `get` and `is_invalid` can never
    /// both answer yes. [`Self::from_file`] warns when that happens rather
    /// than letting the losing block change nothing in silence.
    fn insert(&mut self, kind: ResourceServiceKind, columns: ServiceColumns) {
        self.invalid.retain(|service| *service != kind);
        match self
            .services
            .iter_mut()
            .find(|(service, _)| *service == kind)
        {
            Some(entry) => entry.1 = columns,
            None => self.services.push((kind, columns)),
        }
    }

    /// Marks a service's block as rejected, discarding anything an earlier
    /// key for the same service contributed: a rejected service falls back to
    /// its built-in defaults whole. The mirror of [`Self::insert`].
    fn reject(&mut self, kind: ResourceServiceKind) {
        self.services.retain(|(service, _)| *service != kind);
        if !self.invalid.contains(&kind) {
            self.invalid.push(kind);
        }
    }
}

/// What one service's `[columns.<service>]` blocks added up to: every config
/// key that named it, in the order they were read, each with its own verdict.
/// Only the last key decides the service's state, but a displaced key's
/// rejection is still a problem the user has to fix, so it is kept here
/// rather than discarded with the block.
struct ServiceOutcome<'a> {
    /// The config keys naming this service and each one's rejection, in
    /// encounter order. Never empty; the last entry is the winner.
    keys: Vec<(&'a str, Option<String>)>,
    /// [`KEPT`] or [`FALLBACK`], decided once from the running settings when
    /// the outcome is first recorded (see [`Self::message`]).
    tail: &'static str,
}

impl ServiceOutcome<'_> {
    /// The one warning this service contributes, or `None` when a single key
    /// configured it cleanly and there is nothing to tell the user.
    ///
    /// One message covers however many keys named the service, so it has to
    /// describe the whole outcome: a rejected winner takes the keys it
    /// displaced down with it, and naming a block as deciding there would send
    /// the user looking for a table that fell back to its built-in columns.
    /// A displaced key that was itself invalid has its problem named too, so
    /// that deleting the winning block cannot uncover a typo the user was
    /// never told about.
    fn message(&self) -> Option<String> {
        let ((winner, problem), losers) = self
            .keys
            .split_last()
            .expect("a service outcome is only recorded with a key");
        let also = losers
            .iter()
            .map(|(key, _)| format!("columns.{key}"))
            .collect::<Vec<_>>()
            .join(", ");

        match (problem, losers.len()) {
            (None, 0) => None,
            (None, _) => {
                let mut message = format!(
                    "columns.{winner}: the same service is also configured as {also}; \
                     columns.{winner} decides (blocks are read in alphabetical key order, \
                     not file order)"
                );
                for (key, problem) in losers {
                    if let Some(problem) = problem {
                        message.push_str(&format!("; columns.{key} is itself invalid ({problem})"));
                    }
                }
                Some(message)
            }
            (Some(problem), 0) => Some(format!("columns.{winner}: {problem}; {}", self.tail)),
            (Some(problem), count) => {
                let agrees = if count == 1 { "is" } else { "are" };
                Some(format!(
                    "columns.{winner}: {problem}; the same service is also configured as \
                     {also}, which {agrees} discarded with it (blocks are read in \
                     alphabetical key order, not file order); {}",
                    self.tail
                ))
            }
        }
    }
}

/// Validate one block against its service's catalog. `Err` carries the
/// problem clause of the warning; the first problem found wins, since the
/// block is rejected whole either way. Both the block's shape (is it a table,
/// is `columns` a list of strings) and its content are checked here, so
/// neither class of problem can reach beyond this service.
fn validate(section: &toml::Value, meta: &[ColumnMeta]) -> Result<ServiceColumns, String> {
    let table = section.as_table().ok_or_else(|| {
        format!(
            "expected a table of column settings, found {}",
            section.type_str()
        )
    })?;

    if let Some(key) = table.keys().find(|key| !SETTINGS.contains(&key.as_str())) {
        return Err(format!("unknown setting {key:?}"));
    }

    // Shape first, then meaning: every setting is read as the type it must
    // have before any of them is interpreted.
    let raw_columns = table.get("columns").map(column_names).transpose()?;
    let raw_sort = table
        .get("sort")
        .map(|value| setting_str("sort", value))
        .transpose()?;
    let raw_direction = table
        .get("sort_direction")
        .map(|value| setting_str("sort_direction", value))
        .transpose()?;

    let direction = match raw_direction {
        None => SortDirection::Ascending,
        Some(raw) => {
            parse_direction(raw).ok_or_else(|| format!("invalid sort_direction {raw:?}"))?
        }
    };
    if let (None, Some(raw)) = (raw_sort, raw_direction) {
        // Nothing to apply the direction to: silently keeping the default
        // sort would be exactly the change-nothing-say-nothing failure. The
        // remedy is in the warning, since the block is otherwise fine.
        return Err(format!(
            "sort_direction {raw:?} without a sort column (add sort = \"<column>\")"
        ));
    }

    let columns = match raw_columns {
        None => None,
        Some(raw) => {
            if raw.is_empty() {
                return Err("empty column list".to_string());
            }
            let mut indices: Vec<usize> = Vec::with_capacity(raw.len());
            for name in raw {
                let index = lookup(meta, name).ok_or_else(|| format!("unknown column {name:?}"))?;
                if indices.contains(&index) {
                    return Err(format!("duplicate column {name:?}"));
                }
                indices.push(index);
            }
            if !indices
                .iter()
                .any(|&index| meta[index].key == IDENTITY_COLUMN)
            {
                return Err(format!("column list must include {IDENTITY_COLUMN:?}"));
            }
            Some(indices)
        }
    };

    let sort = match raw_sort {
        None => None,
        Some(raw) => {
            let index = lookup(meta, raw).ok_or_else(|| format!("unknown sort column {raw:?}"))?;
            if !meta[index].sortable {
                return Err(format!("column {raw:?} is not sortable"));
            }
            // With no configured list the effective list is the whole
            // catalog, so any catalog column is reachable.
            if columns
                .as_ref()
                .is_some_and(|configured| !configured.contains(&index))
            {
                return Err(format!(
                    "sort column {raw:?} is not among the configured columns"
                ));
            }
            Some((index, direction))
        }
    };

    Ok(ServiceColumns { columns, sort })
}

/// One setting that must be a string, or the problem clause naming the type
/// the user wrote instead.
fn setting_str<'a>(key: &str, value: &'a toml::Value) -> Result<&'a str, String> {
    value
        .as_str()
        .ok_or_else(|| format!("{key} must be a string, found {}", value.type_str()))
}

/// The `columns` setting as the list of names it must be, or the problem
/// clause naming what was written instead.
fn column_names(value: &toml::Value) -> Result<Vec<&str>, String> {
    let items = value.as_array().ok_or_else(|| {
        format!(
            "columns must be a list of column names, found {}",
            value.type_str()
        )
    })?;
    items
        .iter()
        .map(|item| {
            item.as_str()
                .ok_or_else(|| format!("column name must be a string, found {}", item.type_str()))
        })
        .collect()
}

/// The catalog index of a user-written column name, matched leniently against
/// the catalog keys (which are lowercase by construction).
fn lookup(meta: &[ColumnMeta], raw: &str) -> Option<usize> {
    let wanted = raw.trim().to_ascii_lowercase();
    meta.iter().position(|column| column.key == wanted)
}

fn parse_direction(raw: &str) -> Option<SortDirection> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "asc" => Some(SortDirection::Ascending),
        "desc" => Some(SortDirection::Descending),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a `[columns]` body (service blocks without the `columns.`
    /// prefix) against no running settings, exactly as a fresh (non-reload)
    /// load does.
    fn settings(body: &str) -> (ColumnSettings, Vec<String>) {
        settings_over(body, &ColumnSettings::empty())
    }

    /// Parse a `[columns]` body with `running` in force, exactly as a live
    /// reload validates it: `running` is what decides whether an
    /// invalidated block's warning ends with [`KEPT`] or [`FALLBACK`].
    fn settings_over(body: &str, running: &ColumnSettings) -> (ColumnSettings, Vec<String>) {
        let raw: BTreeMap<String, toml::Value> =
            toml::from_str(body).expect("test fixture must be valid TOML");
        let mut problems = ConfigProblems::default();
        let settings = ColumnSettings::from_file(&raw, &mut problems, running);
        (settings, problems.into_messages())
    }

    /// The catalog index of `key` for `kind`, so tests name columns the way a
    /// user does instead of hardcoding positions.
    fn index(kind: ResourceServiceKind, key: &str) -> usize {
        crate::app::projection::columns::catalog_meta(kind)
            .iter()
            .position(|column| column.key == key)
            .expect("test names a real catalog column")
    }

    const COMPUTE: ResourceServiceKind = ResourceServiceKind::ComputeInstances;
    const GKE: ResourceServiceKind = ResourceServiceKind::GkeClusters;

    #[test]
    fn reorder_subset_sort_and_direction_are_all_applied() {
        let (settings, warnings) = settings(
            r#"
[compute]
columns = ["zone", "name", "status"]
sort = "status"
sort_direction = "desc"
"#,
        );

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let compute = settings.get(COMPUTE).expect("compute block must be kept");
        assert_eq!(
            compute.columns,
            Some(vec![
                index(COMPUTE, "zone"),
                index(COMPUTE, "name"),
                index(COMPUTE, "status"),
            ])
        );
        assert_eq!(
            compute.sort,
            Some((index(COMPUTE, "status"), SortDirection::Descending))
        );
        assert!(!settings.is_invalid(COMPUTE));
    }

    #[test]
    fn sort_only_block_keeps_the_default_columns() {
        let (settings, warnings) = settings("[compute]\nsort = \"zone\"\n");

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let compute = settings.get(COMPUTE).expect("compute block must be kept");
        assert_eq!(compute.columns, None);
        assert_eq!(
            compute.sort,
            Some((index(COMPUTE, "zone"), SortDirection::Ascending))
        );
    }

    #[test]
    fn columns_only_block_keeps_the_default_sort() {
        let (settings, warnings) = settings("[compute]\ncolumns = [\"name\", \"zone\"]\n");

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let compute = settings.get(COMPUTE).expect("compute block must be kept");
        assert_eq!(
            compute.columns,
            Some(vec![index(COMPUTE, "name"), index(COMPUTE, "zone")])
        );
        assert_eq!(compute.sort, None);
    }

    /// Every rule below drops the *whole* block, marks the service invalid,
    /// and pushes exactly one warning.
    fn assert_rejected(body: &str, expected_warning: &str) {
        let (settings, warnings) = settings(body);
        assert_eq!(warnings, vec![expected_warning.to_string()]);
        assert_eq!(settings.get(COMPUTE), None, "block must be dropped whole");
        assert!(settings.is_invalid(COMPUTE));
    }

    #[test]
    fn unknown_column_rejects_the_block() {
        assert_rejected(
            "[compute]\ncolumns = [\"name\", \"zzz\"]\n",
            "columns.compute: unknown column \"zzz\"; ignoring this service's column settings",
        );
    }

    #[test]
    fn duplicate_column_rejects_the_block() {
        assert_rejected(
            "[compute]\ncolumns = [\"name\", \"zone\", \"zone\"]\n",
            "columns.compute: duplicate column \"zone\"; ignoring this service's column settings",
        );
    }

    #[test]
    fn empty_column_list_rejects_the_block() {
        assert_rejected(
            "[compute]\ncolumns = []\n",
            "columns.compute: empty column list; ignoring this service's column settings",
        );
    }

    #[test]
    fn column_list_without_name_rejects_the_block() {
        assert_rejected(
            "[compute]\ncolumns = [\"status\", \"zone\"]\n",
            "columns.compute: column list must include \"name\"; ignoring this service's column settings",
        );
    }

    #[test]
    fn unknown_sort_column_rejects_the_block() {
        assert_rejected(
            "[compute]\nsort = \"zzz\"\n",
            "columns.compute: unknown sort column \"zzz\"; ignoring this service's column settings",
        );
    }

    #[test]
    fn non_sortable_sort_column_rejects_the_block() {
        assert_rejected(
            "[compute]\nsort = \"internal-ip\"\n",
            "columns.compute: column \"internal-ip\" is not sortable; ignoring this service's column settings",
        );
    }

    #[test]
    fn sort_column_outside_the_configured_columns_rejects_the_block() {
        assert_rejected(
            "[compute]\ncolumns = [\"name\", \"status\"]\nsort = \"zone\"\n",
            "columns.compute: sort column \"zone\" is not among the configured columns; ignoring this service's column settings",
        );
    }

    #[test]
    fn invalid_sort_direction_rejects_the_block() {
        assert_rejected(
            "[compute]\nsort = \"zone\"\nsort_direction = \"sideways\"\n",
            "columns.compute: invalid sort_direction \"sideways\"; ignoring this service's column settings",
        );
    }

    #[test]
    fn unknown_setting_in_the_block_rejects_the_block() {
        // A typo'd `column` (singular) would otherwise change nothing and say
        // nothing — the silent failure this feature exists to avoid.
        assert_rejected(
            "[compute]\ncolumn = [\"name\", \"zone\"]\n",
            "columns.compute: unknown setting \"column\"; ignoring this service's column settings",
        );
    }

    #[test]
    fn sort_direction_without_sort_rejects_the_block_and_names_the_remedy() {
        // The block cannot be half-applied, so the warning has to carry the
        // one edit that makes it work.
        assert_rejected(
            "[compute]\ncolumns = [\"name\", \"zone\"]\nsort_direction = \"desc\"\n",
            "columns.compute: sort_direction \"desc\" without a sort column (add sort = \"<column>\"); ignoring this service's column settings",
        );
    }

    #[test]
    fn a_block_that_is_not_a_table_rejects_only_that_service() {
        assert_rejected(
            "compute = \"name\"\n",
            "columns.compute: expected a table of column settings, found string; ignoring this service's column settings",
        );
    }

    #[test]
    fn columns_that_is_not_a_list_rejects_the_block() {
        // The type slip a user makes writing a single column: it stays this
        // service's problem instead of failing the whole file.
        assert_rejected(
            "[compute]\ncolumns = \"name\"\n",
            "columns.compute: columns must be a list of column names, found string; ignoring this service's column settings",
        );
    }

    #[test]
    fn a_non_string_column_name_rejects_the_block() {
        assert_rejected(
            "[compute]\ncolumns = [\"name\", 7]\n",
            "columns.compute: column name must be a string, found integer; ignoring this service's column settings",
        );
    }

    #[test]
    fn a_non_string_sort_rejects_the_block() {
        assert_rejected(
            "[compute]\nsort = 7\n",
            "columns.compute: sort must be a string, found integer; ignoring this service's column settings",
        );
    }

    #[test]
    fn a_non_string_sort_direction_rejects_the_block() {
        assert_rejected(
            "[compute]\nsort = \"zone\"\nsort_direction = true\n",
            "columns.compute: sort_direction must be a string, found boolean; ignoring this service's column settings",
        );
    }

    #[test]
    fn unknown_service_is_skipped_without_invalidating_any_service() {
        let (settings, warnings) = settings("[zzz]\ncolumns = [\"name\"]\n");

        assert_eq!(
            warnings,
            vec!["columns.zzz: unknown service; skipping".to_string()]
        );
        for kind in ResourceServiceKind::ALL {
            assert_eq!(settings.get(kind), None);
            assert!(!settings.is_invalid(kind), "{kind:?} must not be invalid");
        }
    }

    #[test]
    fn an_invalid_block_does_not_disturb_another_services_valid_block() {
        let (settings, warnings) = settings(
            r#"
[compute]
columns = ["name", "zzz"]

[gke]
columns = ["name", "location"]
sort = "location"
"#,
        );

        assert_eq!(warnings.len(), 1, "expected one warning: {warnings:?}");
        assert!(warnings[0].starts_with("columns.compute:"));
        assert_eq!(settings.get(COMPUTE), None);
        assert!(settings.is_invalid(COMPUTE));

        let gke = settings.get(GKE).expect("valid gke block must survive");
        assert_eq!(
            gke.columns,
            Some(vec![index(GKE, "name"), index(GKE, "location")])
        );
        assert_eq!(
            gke.sort,
            Some((index(GKE, "location"), SortDirection::Ascending))
        );
        assert!(!settings.is_invalid(GKE));
    }

    #[test]
    fn service_and_column_names_are_matched_leniently() {
        // Service names use the `[aliases]` vocabulary; column names are
        // matched case-insensitively against the catalog keys.
        let (settings, warnings) = settings("[GCE]\ncolumns = [\"NAME\", \"Zone\"]\n");

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(
            settings.get(COMPUTE).and_then(|c| c.columns.clone()),
            Some(vec![index(COMPUTE, "name"), index(COMPUTE, "zone")])
        );
    }

    #[test]
    fn every_service_can_be_configured_by_its_config_name() {
        let (settings, warnings) = settings(
            r#"
[compute]
columns = ["name"]

[gke]
columns = ["name"]

[cloudsql]
columns = ["name"]

[run]
columns = ["name"]

[storage]
columns = ["name"]

[secrets]
columns = ["name"]

[functions]
columns = ["name"]

[disks]
[pubsub-topics]
columns = ["name"]

[pubsub-subscriptions]
columns = ["name"]

[vpc-networks]
columns = ["name"]

[vpc-subnets]
columns = ["name"]

[firewall-rules]
columns = ["name"]

[bigquery-datasets]
columns = ["name"]

[service-accounts]
columns = ["name"]

[iam-policy]
columns = ["name"]
"#,
        );

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        for kind in ResourceServiceKind::ALL {
            assert!(settings.get(kind).is_some(), "{kind:?} was not configured");
        }
    }

    #[test]
    fn every_catalog_has_the_identity_column_the_rules_require() {
        // `column list must include "name"` is unsatisfiable for a service
        // whose catalog has no such column, so the assumption is pinned.
        for kind in ResourceServiceKind::ALL {
            assert!(
                crate::app::projection::columns::catalog_meta(kind)
                    .iter()
                    .any(|column| column.key == IDENTITY_COLUMN),
                "{kind:?} has no {IDENTITY_COLUMN:?} column"
            );
        }
    }

    #[test]
    fn a_block_that_sets_nothing_is_a_no_op_rather_than_an_error() {
        // Nothing is wrong with it and nothing is dropped: the service keeps
        // both its default columns and its default sort.
        let (settings, warnings) = settings("[compute]\n");

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let compute = settings.get(COMPUTE).expect("compute block must be kept");
        assert_eq!(compute.columns, None);
        assert_eq!(compute.sort, None);
        assert!(!settings.is_invalid(COMPUTE));
    }

    #[test]
    fn two_config_keys_for_one_service_leave_it_configured_once_and_warn() {
        // Blocks are read in key order, so `gce` lands after `compute` and
        // decides — even though the user wrote it second, they would have got
        // the same table writing it first. The block that loses changes
        // nothing, so it has to be reported: this is exactly the edit-and-see-
        // no-change failure the module exists to prevent.
        let (settings, warnings) = settings(
            "[compute]\ncolumns = [\"name\", \"status\"]\n\n[gce]\ncolumns = [\"name\", \"zone\"]\n",
        );

        assert_eq!(
            warnings,
            vec![
                "columns.gce: the same service is also configured as columns.compute; \
                 columns.gce decides (blocks are read in alphabetical key order, not file order)"
                    .to_string()
            ]
        );
        assert_eq!(
            settings.get(COMPUTE).and_then(|c| c.columns.clone()),
            Some(vec![index(COMPUTE, "name"), index(COMPUTE, "zone")])
        );
    }

    #[test]
    fn the_duplicate_key_warning_does_not_depend_on_file_order() {
        // The same two blocks written the other way round: same winner, same
        // warning. The outcome is a property of the keys, not of the file.
        let (swapped, swapped_warnings) = settings(
            "[gce]\ncolumns = [\"name\", \"zone\"]\n\n[compute]\ncolumns = [\"name\", \"status\"]\n",
        );
        let (original, original_warnings) = settings(
            "[compute]\ncolumns = [\"name\", \"status\"]\n\n[gce]\ncolumns = [\"name\", \"zone\"]\n",
        );

        assert_eq!(swapped_warnings, original_warnings);
        assert_eq!(swapped, original);
    }

    #[test]
    fn a_third_key_for_the_same_service_names_every_key_it_displaces() {
        // `compute` < `compute_instances` < `gce`, so `gce` decides and the
        // one warning lists both keys it displaced, in key order. The
        // intermediate "compute_instances decides" step is never reported:
        // it stopped being true the moment `gce` was read.
        let (settings, warnings) = settings(
            "[compute]\ncolumns = [\"name\"]\n\n\
             [gce]\ncolumns = [\"name\", \"zone\"]\n\n\
             [compute_instances]\ncolumns = [\"name\", \"status\"]\n",
        );

        assert_eq!(
            warnings,
            vec![
                "columns.gce: the same service is also configured as columns.compute, \
                 columns.compute_instances; columns.gce decides (blocks are read in \
                 alphabetical key order, not file order)"
                    .to_string()
            ]
        );
        assert_eq!(
            settings.get(COMPUTE).and_then(|c| c.columns.clone()),
            Some(vec![index(COMPUTE, "name"), index(COMPUTE, "zone")])
        );
    }

    #[test]
    fn a_rejected_deciding_block_discards_every_earlier_key_in_one_warning() {
        // Two valid keys then a broken deciding one: the service falls back
        // whole, so the single message names the rejection, lists both
        // discarded keys with `are`, and never claims anything decides a
        // table that shows built-in columns.
        let (settings, warnings) = settings(
            "[compute]\ncolumns = [\"name\"]\n\n\
             [compute_instances]\ncolumns = [\"name\", \"status\"]\n\n\
             [gce]\ncolumns = [\"name\", \"zzz\"]\n",
        );

        assert_eq!(
            warnings,
            vec![
                "columns.gce: unknown column \"zzz\"; the same service is also configured as \
                 columns.compute, columns.compute_instances, which are discarded with it \
                 (blocks are read in alphabetical key order, not file order); \
                 ignoring this service's column settings"
                    .to_string()
            ]
        );
        assert!(
            !warnings[0].contains("decides"),
            "a fallen-back service must not be told a block decides it: {:?}",
            warnings[0]
        );
        assert_eq!(settings.get(COMPUTE), None, "block must be dropped whole");
        assert!(settings.is_invalid(COMPUTE));
    }

    #[test]
    fn each_service_reports_once_in_config_key_order() {
        // Two independent services, each with its own problem: one message
        // apiece, ordered by the key they were encountered under.
        let (_, warnings) = settings(
            "[compute]\ncolumns = [\"name\", \"zzz\"]\n\n\
             [gke]\ncolumns = [\"name\", \"zzz\"]\n",
        );

        assert_eq!(
            warnings,
            vec![
                "columns.compute: unknown column \"zzz\"; ignoring this service's column settings"
                    .to_string(),
                "columns.gke: unknown column \"zzz\"; ignoring this service's column settings"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn an_unknown_service_is_reported_before_any_composed_message() {
        // The unknown key has no service to compose an outcome for, so it is
        // reported where it was found rather than joining the per-service
        // pass that runs after the loop.
        let (_, warnings) = settings(
            "[compute]\ncolumns = [\"name\", \"zzz\"]\n\n\
             [zzz]\ncolumns = [\"name\"]\n",
        );

        assert_eq!(
            warnings,
            vec![
                "columns.zzz: unknown service; skipping".to_string(),
                "columns.compute: unknown column \"zzz\"; ignoring this service's column settings"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn a_rejected_block_discards_an_earlier_key_for_the_same_service() {
        // `compute` is valid, `gce` is not: the service must fall back whole
        // rather than keep the half of its configuration that parsed.
        let (settings, warnings) = settings(
            "[compute]\ncolumns = [\"name\", \"status\"]\n\n[gce]\ncolumns = [\"name\", \"zzz\"]\n",
        );

        // One warning, not two: the deciding block's rejection is what the
        // duplicate keys resolved to, so it is reported once, with both facts
        // in it. Splitting them would put a warning claiming `gce` decides the
        // table next to one saying the table shows defaults.
        assert_eq!(
            warnings,
            vec![
                "columns.gce: unknown column \"zzz\"; the same service is also configured as \
                 columns.compute, which is discarded with it (blocks are read in alphabetical \
                 key order, not file order); ignoring this service's column settings"
                    .to_string()
            ]
        );
        assert_eq!(settings.get(COMPUTE), None);
        assert!(settings.is_invalid(COMPUTE));
    }

    #[test]
    fn a_valid_block_clears_an_earlier_rejection_for_the_same_service() {
        // The mirror of the case above: `compute` is rejected, then `gce`
        // configures the same service. The service is configured *or* fallen
        // back, never both — task 4 reads `is_invalid` to decide whether to
        // keep the previously applied settings.
        let (settings, warnings) = settings(
            "[compute]\ncolumns = [\"name\", \"zzz\"]\n\n[gce]\ncolumns = [\"name\", \"zone\"]\n",
        );

        // One message, not two: `compute`'s rejection stopped being this
        // service's outcome when `gce` configured it, so reporting it would
        // send the user looking for a fallback that never happened. Its
        // problem is still named, though — deleting the `gce` block must not
        // be how the user first learns about the typo.
        assert_eq!(
            warnings,
            vec![
                "columns.gce: the same service is also configured as columns.compute; \
                 columns.gce decides (blocks are read in alphabetical key order, not file \
                 order); columns.compute is itself invalid (unknown column \"zzz\")"
                    .to_string()
            ]
        );
        assert!(
            !warnings[0].contains(FALLBACK),
            "a configured service must not be told it fell back: {:?}",
            warnings[0]
        );
        assert_eq!(
            settings.get(COMPUTE).and_then(|c| c.columns.clone()),
            Some(vec![index(COMPUTE, "name"), index(COMPUTE, "zone")])
        );
        assert!(!settings.is_invalid(COMPUTE));
    }

    #[test]
    fn every_displaced_key_that_is_itself_invalid_is_named_in_the_one_message() {
        // Two broken losers behind a valid winner: the service is configured,
        // so the message still says `gce` decides and never claims a
        // fallback, but both typos are named, in key order, so neither can
        // hide behind the block that outranks it.
        let (settings, warnings) = settings(
            "[compute]\ncolumns = [\"name\", \"zzz\"]\n\n\
             [compute_instances]\nsort = \"yyy\"\n\n\
             [gce]\ncolumns = [\"name\", \"zone\"]\n",
        );

        assert_eq!(
            warnings,
            vec![
                "columns.gce: the same service is also configured as columns.compute, \
                 columns.compute_instances; columns.gce decides (blocks are read in \
                 alphabetical key order, not file order); columns.compute is itself invalid \
                 (unknown column \"zzz\"); columns.compute_instances is itself invalid \
                 (unknown sort column \"yyy\")"
                    .to_string()
            ]
        );
        assert!(
            !warnings[0].contains(FALLBACK),
            "a configured service must not be told it fell back: {:?}",
            warnings[0]
        );
        assert_eq!(
            settings.get(COMPUTE).and_then(|c| c.columns.clone()),
            Some(vec![index(COMPUTE, "name"), index(COMPUTE, "zone")])
        );
        assert!(!settings.is_invalid(COMPUTE));
    }

    /// The settings a reload adopts over `previous`, from two config bodies.
    fn adopted(previous_body: &str, new_body: &str) -> ColumnSettings {
        let (previous, _) = settings(previous_body);
        let (new, _) = settings(new_body);
        new.adopt_over(&previous)
    }

    #[test]
    fn adopt_over_keeps_the_running_settings_of_a_newly_invalid_service() {
        let merged = adopted(
            "[compute]\ncolumns = [\"name\", \"zone\"]\n",
            "[compute]\ncolumns = [\"name\", \"zzz\"]\n",
        );

        assert_eq!(
            merged
                .get(COMPUTE)
                .and_then(|compute| compute.columns.clone()),
            Some(vec![index(COMPUTE, "name"), index(COMPUTE, "zone")]),
            "a broken block leaves what was running in force"
        );
        assert!(!merged.is_invalid(COMPUTE));
    }

    #[test]
    fn adopt_over_takes_the_new_settings_of_a_valid_service() {
        let merged = adopted(
            "[compute]\ncolumns = [\"name\", \"zone\"]\n",
            "[compute]\ncolumns = [\"name\", \"status\"]\n",
        );

        assert_eq!(
            merged
                .get(COMPUTE)
                .and_then(|compute| compute.columns.clone()),
            Some(vec![index(COMPUTE, "name"), index(COMPUTE, "status")])
        );
    }

    #[test]
    fn adopt_over_lets_a_removed_block_fall_back_to_the_defaults() {
        let merged = adopted("[compute]\ncolumns = [\"name\", \"zone\"]\n", "");

        assert_eq!(merged.get(COMPUTE), None);
        assert!(!merged.is_invalid(COMPUTE));
    }

    #[test]
    fn adopt_over_never_merges_within_a_service() {
        // The new block configures only the columns, so the sort that was
        // running goes with the block it came from rather than surviving into
        // a state no config file describes.
        let merged = adopted(
            "[compute]\ncolumns = [\"name\", \"zone\"]\nsort = \"zone\"\n",
            "[compute]\ncolumns = [\"name\", \"status\"]\n",
        );

        assert_eq!(merged.get(COMPUTE).and_then(|compute| compute.sort), None);
    }

    #[test]
    fn adopt_over_isolates_the_services_from_each_other() {
        let merged = adopted(
            "[compute]\ncolumns = [\"name\", \"zone\"]\n\n[gke]\ncolumns = [\"name\", \"location\"]\n",
            "[compute]\ncolumns = [\"name\", \"zzz\"]\n\n[gke]\ncolumns = [\"name\", \"version\"]\n",
        );

        assert_eq!(
            merged
                .get(COMPUTE)
                .and_then(|compute| compute.columns.clone()),
            Some(vec![index(COMPUTE, "name"), index(COMPUTE, "zone")])
        );
        assert_eq!(
            merged.get(GKE).and_then(|gke| gke.columns.clone()),
            Some(vec![index(GKE, "name"), index(GKE, "version")])
        );
    }

    #[test]
    fn adopt_over_records_a_service_that_was_already_running_the_defaults() {
        // Nothing was running for compute and the new block is broken: it
        // stays on its defaults, and the merged settings say so rather than
        // claiming a configuration it does not have.
        let merged = adopted("", "[compute]\ncolumns = [\"name\", \"zzz\"]\n");

        assert_eq!(merged.get(COMPUTE), None);
        assert!(merged.is_invalid(COMPUTE));
    }

    #[test]
    fn empty_settings_configure_nothing() {
        let settings = ColumnSettings::empty();
        for kind in ResourceServiceKind::ALL {
            assert_eq!(settings.get(kind), None);
            assert!(!settings.is_invalid(kind));
        }
    }

    /// The resolved sort of a `[compute]` block, for the direction cases.
    fn compute_sort(body: &str) -> Option<(usize, SortDirection)> {
        let (settings, warnings) = settings(body);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        settings.get(COMPUTE).and_then(|compute| compute.sort)
    }

    #[test]
    fn sort_direction_accepts_asc_and_desc_case_insensitively() {
        assert_eq!(
            compute_sort("[compute]\nsort = \"zone\"\nsort_direction = \"DESC\"\n"),
            Some((index(COMPUTE, "zone"), SortDirection::Descending))
        );
        assert_eq!(
            compute_sort("[compute]\nsort = \"zone\"\nsort_direction = \"Asc\"\n"),
            Some((index(COMPUTE, "zone"), SortDirection::Ascending))
        );
    }

    #[test]
    fn a_newly_invalid_block_keeps_the_running_settings_tail() {
        // The reload case this is about: something is already running for
        // this service, so the broken block does not fall back to the
        // built-in columns — it keeps what is in force, and the warning has
        // to say so.
        let (running, _) = settings("[compute]\ncolumns = [\"name\", \"zone\"]\n");
        let (_, warnings) = settings_over("[compute]\ncolumns = [\"name\", \"zzz\"]\n", &running);

        assert_eq!(
            warnings,
            vec![format!("columns.compute: unknown column \"zzz\"; {KEPT}")]
        );
    }

    #[test]
    fn a_newly_invalid_block_falls_back_when_nothing_was_running() {
        // The existing, non-reload behaviour: nothing is running yet, so
        // there is nothing to keep and the block falls back to the built-in
        // defaults as before.
        let (_, warnings) = settings_over(
            "[compute]\ncolumns = [\"name\", \"zzz\"]\n",
            &ColumnSettings::empty(),
        );

        assert_eq!(
            warnings,
            vec![format!(
                "columns.compute: unknown column \"zzz\"; {FALLBACK}"
            )]
        );
    }

    #[test]
    fn a_rejected_deciding_block_with_discarded_keys_picks_the_tail_from_running_settings() {
        // Same shape as `a_rejected_deciding_block_discards_every_earlier_key_in_one_warning`,
        // but with something running for the service: the multi-key message
        // still ends on the tail the running settings earn, not on `FALLBACK`.
        let (running, _) = settings("[compute]\ncolumns = [\"name\", \"zone\"]\n");
        let (_, warnings) = settings_over(
            "[compute]\ncolumns = [\"name\"]\n\n\
             [compute_instances]\ncolumns = [\"name\", \"status\"]\n\n\
             [gce]\ncolumns = [\"name\", \"zzz\"]\n",
            &running,
        );

        assert_eq!(
            warnings,
            vec![format!(
                "columns.gce: unknown column \"zzz\"; the same service is also configured as \
                 columns.compute, columns.compute_instances, which are discarded with it \
                 (blocks are read in alphabetical key order, not file order); {KEPT}"
            )]
        );
    }

    #[test]
    fn running_settings_for_a_different_service_do_not_change_this_services_tail() {
        // Running settings are keyed per service; `compute` has nothing
        // running, so its broken block still falls back even though some
        // other service (`gke`) is running configured settings.
        let (running, _) = settings("[gke]\ncolumns = [\"name\", \"location\"]\n");
        let (_, warnings) = settings_over("[compute]\ncolumns = [\"name\", \"zzz\"]\n", &running);

        assert_eq!(
            warnings,
            vec![format!(
                "columns.compute: unknown column \"zzz\"; {FALLBACK}"
            )]
        );
    }

    #[test]
    fn the_tail_and_adopt_over_agree_on_the_same_running_settings() {
        // The message is only true if it matches what `adopt_over` actually
        // does with the same `running` argument: `KEPT` must mean
        // `adopt_over` keeps the running layout, and `FALLBACK` must mean it
        // falls back to `None` with `is_invalid` set.
        let (running, _) = settings("[compute]\ncolumns = [\"name\", \"zone\"]\n");
        let (new, warnings) = settings_over("[compute]\ncolumns = [\"name\", \"zzz\"]\n", &running);
        assert!(warnings[0].ends_with(KEPT));
        let merged = new.adopt_over(&running);
        assert_eq!(merged.get(COMPUTE), running.get(COMPUTE));

        let (empty_new, empty_warnings) = settings_over(
            "[compute]\ncolumns = [\"name\", \"zzz\"]\n",
            &ColumnSettings::empty(),
        );
        assert!(empty_warnings[0].ends_with(FALLBACK));
        let empty_merged = empty_new.adopt_over(&ColumnSettings::empty());
        assert_eq!(empty_merged.get(COMPUTE), None);
        assert!(empty_merged.is_invalid(COMPUTE));
    }
}
