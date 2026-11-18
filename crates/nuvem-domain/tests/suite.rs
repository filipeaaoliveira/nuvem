// One integration-test binary for the domain crate; see the root crate's
// tests/suite.rs for the full rationale (per-file targets each link the
// whole library).

#[path = "bigquery_schema_field_test.rs"]
mod bigquery_schema_field_test;
#[path = "bigquery_table_stat_test.rs"]
mod bigquery_table_stat_test;
#[path = "configuration_display_test.rs"]
mod configuration_display_test;
#[path = "diagnostics_classify_test.rs"]
mod diagnostics_classify_test;
#[path = "diagnostics_sanitize_test.rs"]
mod diagnostics_sanitize_test;
#[path = "docs_completeness_test.rs"]
mod docs_completeness_test;
#[path = "filter_grammar_test.rs"]
mod filter_grammar_test;
#[path = "finops_domain_test.rs"]
mod finops_domain_test;
#[path = "finops_locations_test.rs"]
mod finops_locations_test;
#[path = "finops_money_test.rs"]
mod finops_money_test;
#[path = "firewall_targets_instance_test.rs"]
mod firewall_targets_instance_test;
#[path = "logging_metadata_test.rs"]
mod logging_metadata_test;
#[path = "plugin_output_test.rs"]
mod plugin_output_test;
#[path = "resource_call_test.rs"]
mod resource_call_test;

#[path = "resource_metadata_test.rs"]
mod resource_metadata_test;
#[path = "text_terminal_test.rs"]
mod text_terminal_test;
#[path = "tunnel_classify_test.rs"]
mod tunnel_classify_test;
