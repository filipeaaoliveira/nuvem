//! `[columns.<service>]` as it behaves through a real config file:
//! what reaches `ResolvedConfig`, what reaches the warning list, and
//! what a broken block is and is not allowed to take down with it.

use std::io::Write;

use clap::Parser;
use nuvem::app::projection::binding::KeyTrigger;
use nuvem::app::projection::columns::catalog_meta;
use nuvem::app::projection::table::SortDirection;
use nuvem::config::columns::ColumnSettings;
use nuvem::config::hotkeys::RebindableAction;
use nuvem::config::theme::ThemeChoice;
use nuvem::{Cli, Config, ConfigOutcome};
use nuvem_domain::gcp::resource::ResourceServiceKind;

const COMPUTE: ResourceServiceKind = ResourceServiceKind::ComputeInstances;
const GKE: ResourceServiceKind = ResourceServiceKind::GkeClusters;

fn resolve(body: &str) -> (ConfigOutcome, tempfile::NamedTempFile) {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(body.as_bytes()).unwrap();
    let cli = Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        file.path().to_str().unwrap(),
    ]);
    // The temp file is returned so it outlives the resolve call's path.
    (Config::resolve(&cli), file)
}

fn index(kind: ResourceServiceKind, key: &str) -> usize {
    catalog_meta(kind)
        .iter()
        .position(|column| column.key == key)
        .expect("test names a real catalog column")
}

#[test]
fn a_file_without_a_columns_table_configures_nothing() {
    let (out, _file) = resolve("version = 1\n[general]\npoll_interval_secs = 12\n");

    assert!(
        out.problems.is_empty(),
        "unexpected: {:?}",
        out.problems.messages()
    );
    assert_eq!(out.config.columns, ColumnSettings::empty());
    for kind in ResourceServiceKind::ALL {
        assert_eq!(out.config.columns.get(kind), None);
        assert!(!out.config.columns.is_invalid(kind));
    }
}

#[test]
fn a_valid_block_reaches_the_resolved_config() {
    let (out, _file) = resolve(
        r#"
version = 1

[columns.compute]
columns = ["zone", "name", "status"]
sort = "status"
sort_direction = "desc"
"#,
    );

    assert!(
        out.problems.is_empty(),
        "unexpected: {:?}",
        out.problems.messages()
    );
    let compute = out.config.columns.get(COMPUTE).expect("compute configured");
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
}

#[test]
fn a_broken_block_warns_without_discarding_the_rest_of_the_file() {
    let (out, _file) = resolve(
        r#"
version = 1

[general]
poll_interval_secs = 12

[aliases]
vms = "gce"

[columns.compute]
columns = ["name", "zzz"]

[columns.gke]
columns = ["name", "location"]
sort = "location"
"#,
    );

    // Field-level problem: one warning, no load error, everything else applied.
    assert_eq!(out.load_error, None);
    assert_eq!(
        out.problems.messages(),
        vec!["columns.compute: unknown column \"zzz\"; ignoring this service's column settings"]
    );
    assert_eq!(out.config.poll_interval_secs, 12);
    assert!(out.config.aliases.get("vms").is_some());

    // The broken service falls back to defaults and is flagged; its neighbour
    // keeps its configuration.
    assert_eq!(out.config.columns.get(COMPUTE), None);
    assert!(out.config.columns.is_invalid(COMPUTE));
    let gke = out.config.columns.get(GKE).expect("gke configured");
    assert_eq!(
        gke.columns,
        Some(vec![index(GKE, "name"), index(GKE, "location")])
    );
    assert!(!out.config.columns.is_invalid(GKE));
}

#[test]
fn an_unknown_service_block_is_skipped_and_flags_no_service() {
    let (out, _file) = resolve("version = 1\n\n[columns.zzz]\ncolumns = [\"name\"]\n");

    assert_eq!(out.load_error, None);
    assert_eq!(
        out.problems.messages(),
        vec!["columns.zzz: unknown service; skipping"]
    );
    for kind in ResourceServiceKind::ALL {
        assert!(!out.config.columns.is_invalid(kind));
    }
}

#[test]
fn a_typo_key_inside_a_block_is_reported_rather_than_ignored() {
    let (out, _file) = resolve(
        r#"
version = 1

[columns.compute]
column = ["name", "zone"]
"#,
    );

    assert_eq!(out.load_error, None);
    assert_eq!(
        out.problems.messages(),
        vec![
            "columns.compute: unknown setting \"column\"; ignoring this service's column settings"
        ]
    );
    assert_eq!(out.config.columns.get(COMPUTE), None);
    assert!(out.config.columns.is_invalid(COMPUTE));
}

#[test]
fn a_wrongly_typed_setting_costs_its_service_and_nothing_else() {
    // A key of the wrong *type* is validated per service, like an unknown
    // key: one typo in one block must not cost the user their theme, aliases
    // and hotkeys, nor another service's valid block.
    let (out, _file) = resolve(
        r#"
version = 1

[general]
theme = "high-contrast"

[aliases]
vms = "gce"

[hotkeys]
refresh = "R"

[columns.compute]
columns = "name"

[columns.gke]
columns = ["name", "location"]
"#,
    );

    assert_eq!(out.load_error, None);
    assert_eq!(
        out.problems.messages(),
        vec![
            "columns.compute: columns must be a list of column names, found string; ignoring this service's column settings"
        ]
    );

    // The rest of the file survives the typo.
    assert_eq!(out.config.theme, ThemeChoice::HighContrast);
    assert!(out.config.aliases.get("vms").is_some());
    assert_eq!(
        out.config
            .hotkeys
            .effective_trigger(RebindableAction::Refresh),
        KeyTrigger::Char('R')
    );

    // Compute falls back whole and is flagged; gke keeps its block.
    assert_eq!(out.config.columns.get(COMPUTE), None);
    assert!(out.config.columns.is_invalid(COMPUTE));
    let gke = out.config.columns.get(GKE).expect("gke configured");
    assert_eq!(
        gke.columns,
        Some(vec![index(GKE, "name"), index(GKE, "location")])
    );
    assert!(!out.config.columns.is_invalid(GKE));
}

#[test]
fn a_service_block_that_is_not_a_table_is_still_per_service() {
    let (out, _file) = resolve("version = 1\n\n[columns]\ncompute = \"name\"\n");

    assert_eq!(out.load_error, None);
    assert_eq!(
        out.problems.messages(),
        vec![
            "columns.compute: expected a table of column settings, found string; ignoring this service's column settings"
        ]
    );
    assert_eq!(out.config.columns.get(COMPUTE), None);
    assert!(out.config.columns.is_invalid(COMPUTE));
}

#[test]
fn a_file_level_problem_still_discards_the_whole_file() {
    // The boundary the per-service handling above must not blur: malformed
    // TOML and an unsupported version are still load errors that keep the
    // prior config rather than half-applying this one.
    let (out, file) = resolve("version = 1\n\n[columns.compute\ncolumns = [\"name\"]\n");
    let load_error = out.load_error.expect("malformed TOML must fail the load");
    assert!(load_error.contains(file.path().to_str().unwrap()));
    assert_eq!(out.config.columns, ColumnSettings::empty());

    let (out, file) = resolve("version = 99\n\n[columns.compute]\ncolumns = [\"name\"]\n");
    let load_error = out.load_error.expect("bad version must fail the load");
    assert!(load_error.contains(file.path().to_str().unwrap()));
    assert_eq!(out.config.columns, ColumnSettings::empty());
}
